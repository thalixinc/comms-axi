//! R2 — the single resolution source, host-aware (founder amendment).
//!
//! `resolve_role(role, run, record) -> StationBinding` is the ONLY producer of a binding; no verb
//! re-resolves the surface string. It generalizes `codefactory/src/team/adapter.rs::resolve_binding`
//! and resolves HOST-first: (1) which host owns the station (fleet record `host` field), (2) the
//! transport to that host (by reachability, via the host registry), (3) adapter + surface ON that
//! host. Surface precedence (R4): explicit override > fleet record > `cos_surface` hint > default
//! (`omp`). A stale hint that contradicts the binding loses loudly (error + diagnostic).
//!
//! The record shape (a `serde_json::Value`) the resolver reads:
//!
//! ```json
//! {
//!   "host": "thalixs-mbp",                // fleet record host field; default "local"
//!   "hosts": {                            // host registry: host name → transport (reachability)
//!     "local": "local-socket",
//!     "thalixs-mbp": "tailscale",
//!     "ec2-control": "ssh"
//!   },
//!   "fleet": {                            // G5 fleet profile inputs (optional)
//!     "default":   { "surface": "herdr", "harness": "omp", "layout": "3by2" },
//!     "override":  { "surface": "omp" },
//!     "generation": 3
//!   },
//!   "cos_surface": "omp",                 // R4 hint (optional)
//!   "overrides": {                        // per-role explicit overrides (optional)
//!     "coordinator": { "surface": "omp" }
//!   },
//!   "instances": {                        // role → station instance
//!     "coordinator": { "station_instance_id": "sess-1", "lifetime": "run", "adapter": "herdr" }
//!   },
//!   "allocation": { "target_sandbox": "sb-1" },
//!   "environments": { "coordinator": "sb-b" }
//! }
//! ```

use std::fmt;
use std::str::FromStr;

use serde_json::Value;

use crate::adapter::{AdapterKind, HostRef, Route, StationBinding, TransportKind};
use crate::fleet::{self, EffectiveProfile, FleetDefault, Override};

/// The host a role lives on when the fleet record names none.
pub const DEFAULT_HOST: &str = "local";
/// The surface fallback when no override, fleet record, or hint names one (R4).
pub const DEFAULT_SURFACE: &str = "omp";

/// Errors `resolve_role` reports loudly, never a silent fork.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum ResolveError {
    /// A `cos_surface` hint contradicted the resolved binding surface (R4).
    StaleSurfaceHint { hint: String, resolved: String },
    /// The fleet record named a host with no registry entry (no transport to reach it).
    UnknownHost { host: String },
    /// A host registry entry named a transport kind we do not know.
    UnknownTransport { host: String, transport: String },
    /// An instance named an adapter kind we do not know.
    UnknownAdapter(String),
    /// The fleet profile inputs failed G5 resolution (e.g. a named-but-blank override key).
    Fleet(String),
}

impl fmt::Display for ResolveError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            ResolveError::StaleSurfaceHint { hint, resolved } => write!(
                f,
                "stale cos_surface hint {hint:?}: the binding resolves to surface {resolved:?}"
            ),
            ResolveError::UnknownHost { host } => {
                write!(
                    f,
                    "no host-registry entry for host {host:?}; cannot derive a transport"
                )
            }
            ResolveError::UnknownTransport { host, transport } => {
                write!(f, "host {host:?} names unknown transport {transport:?}")
            }
            ResolveError::UnknownAdapter(a) => write!(f, "instance names unknown adapter {a:?}"),
            ResolveError::Fleet(e) => write!(f, "fleet profile resolution failed: {e}"),
        }
    }
}

impl std::error::Error for ResolveError {}

/// Resolve a role to its fleet-wide binding — the single resolution source (R2).
///
/// Host-first: the fleet record's `host` field names the machine the role's station lives on;
/// the host registry supplies the transport to reach it (reachability); adapter + surface resolve
/// ON that host. Surface precedence is R4 (override > fleet record > hint > default `omp`), and a
/// `cos_surface` hint that contradicts the resolved binding errors loudly (never a silent fork).
pub fn resolve_role(role: &str, run: &str, record: &Value) -> Result<StationBinding, ResolveError> {
    let (host, transport) = resolve_host(record)?;
    let (surface, generation) = resolve_surface(role, record)?;
    let adapter = adapter_for(record, role, &surface)?;
    let surface_ref = backend_surface(record, run, role, adapter);
    let session_id = session_id(record, role);
    let route = Route {
        host: host.name.clone(),
        surface: surface.clone(),
        generation,
    };
    Ok(StationBinding {
        adapter,
        host,
        transport,
        surface_ref,
        session_id,
        route,
    })
}

/// (1) host, then (2) transport. The fleet record's `host` field names the machine; the host
/// registry (`hosts`) maps that name to a transport by reachability. `local` is always
/// `local-socket`; a non-local host without a registry entry (or with an unknown transport) is
/// reported loudly — never a silent local fork.
fn resolve_host(record: &Value) -> Result<(HostRef, TransportKind), ResolveError> {
    let name = record
        .get("host")
        .and_then(Value::as_str)
        .unwrap_or(DEFAULT_HOST)
        .to_string();

    if name == DEFAULT_HOST {
        return Ok((HostRef { name }, TransportKind::LocalSocket));
    }

    let transport = record
        .get("hosts")
        .and_then(|h| h.get(&name))
        .and_then(Value::as_str)
        .ok_or_else(|| ResolveError::UnknownHost { host: name.clone() })?;

    match TransportKind::from_str(transport) {
        Ok(kind) => Ok((HostRef { name }, kind)),
        Err(_) => Err(ResolveError::UnknownTransport {
            host: name,
            transport: transport.to_string(),
        }),
    }
}

/// (3) the surface, R4 precedence: explicit override > fleet record > `cos_surface` hint > default
/// (`omp`). Returns the surface plus the fleet's observed-handle generation (the G5 truthful stamp
/// for `route`; 0 when no fleet record). A hint that contradicts the resolved surface errors loudly.
fn resolve_surface(role: &str, record: &Value) -> Result<(String, u64), ResolveError> {
    let override_surface = record
        .get("overrides")
        .and_then(|o| o.get(role))
        .and_then(|v| v.get("surface"))
        .and_then(Value::as_str);

    let fleet = fleet_profile(record)?;
    let hint = record.get("cos_surface").and_then(Value::as_str);

    let surface = override_surface
        .map(str::to_string)
        .or_else(|| fleet.as_ref().map(|p| p.surface.clone()))
        .or_else(|| hint.map(str::to_string))
        .unwrap_or_else(|| DEFAULT_SURFACE.to_string());

    // A stale hint that contradicts the binding loses loudly, never silently forks.
    if let Some(h) = hint {
        if h != surface {
            return Err(ResolveError::StaleSurfaceHint {
                hint: h.to_string(),
                resolved: surface,
            });
        }
    }

    let generation = fleet.as_ref().map(|p| p.observed_generation).unwrap_or(0);
    Ok((surface, generation))
}

/// Resolve the G5 fleet profile from the record's `fleet` inputs (`default` + `override` +
/// `generation`) via `fleet::resolve`. `None` when the record carries no fleet profile.
fn fleet_profile(record: &Value) -> Result<Option<EffectiveProfile>, ResolveError> {
    let Some(fleet) = record.get("fleet") else {
        return Ok(None);
    };
    let default = fleet_default(fleet.get("default"));
    let over = fleet_override(fleet.get("override"));
    let generation = fleet.get("generation").and_then(Value::as_u64).unwrap_or(0);
    fleet::resolve(&default, &over, generation)
        .map(Some)
        .map_err(ResolveError::Fleet)
}

fn fleet_default(v: Option<&Value>) -> FleetDefault {
    let mut d = FleetDefault::default();
    if let Some(v) = v {
        if let Some(s) = v.get("surface").and_then(Value::as_str) {
            d.surface = s.to_string();
        }
        if let Some(s) = v.get("harness").and_then(Value::as_str) {
            d.harness = s.to_string();
        }
        if let Some(s) = v.get("layout").and_then(Value::as_str) {
            d.layout = s.to_string();
        }
    }
    d
}

fn fleet_override(v: Option<&Value>) -> Override {
    let mut o = Override::default();
    if let Some(v) = v {
        o.surface = v.get("surface").and_then(Value::as_str).map(str::to_string);
        o.harness = v.get("harness").and_then(Value::as_str).map(str::to_string);
        o.layout = v.get("layout").and_then(Value::as_str).map(str::to_string);
    }
    o
}

/// The adapter a station binds to: an explicit `instances[role].adapter` wins; else `sbox` when the
/// station is bound to a sandbox environment; else the surface maps to its adapter (`herdr` →
/// Herdr, anything else → Cmux). Transport is a property of the binding, NOT of the definition.
fn adapter_for(record: &Value, role: &str, surface: &str) -> Result<AdapterKind, ResolveError> {
    if let Some(explicit) = record
        .get("instances")
        .and_then(|i| i.get(role))
        .and_then(|v| v.get("adapter"))
        .and_then(Value::as_str)
    {
        return AdapterKind::from_str(explicit)
            .map_err(|_| ResolveError::UnknownAdapter(explicit.to_string()));
    }
    if !station_environment(record, role).is_empty() {
        return Ok(AdapterKind::Sbox);
    }
    match surface {
        "herdr" => Ok(AdapterKind::Herdr),
        _ => Ok(AdapterKind::Cmux),
    }
}

/// The environment a station is bound to: a per-role `environments[role]`, else the run's shared
/// `allocation.target_sandbox`. Empty when the run has no sbox allocation.
fn station_environment(record: &Value, role: &str) -> String {
    let shared = record
        .get("allocation")
        .and_then(|a| a.get("target_sandbox"))
        .and_then(Value::as_str)
        .unwrap_or("")
        .to_string();
    let per_station = record
        .get("environments")
        .and_then(|e| e.get(role))
        .and_then(Value::as_str)
        .unwrap_or("")
        .to_string();
    if !per_station.is_empty() {
        per_station
    } else {
        shared
    }
}

/// The adapter-owned backend surface ref ON the host. Deterministic per (adapter, run, role);
/// never a caller-supplied `surface:N`.
fn backend_surface(record: &Value, run: &str, role: &str, adapter: AdapterKind) -> String {
    match adapter {
        AdapterKind::Cmux => format!("cf-subway-{run}:{role}"),
        AdapterKind::Sbox => format!("sbox:{}", station_environment(record, role)),
        AdapterKind::Herdr => herdr_session_name(run, role),
    }
}

/// Bounded herdr session name (sun_path-safe): "cf-subway-" (10) + 32 + 1 + role, well under 104.
fn herdr_session_name(run: &str, role: &str) -> String {
    const RUN_CAP: usize = 32;
    let bounded_run: String = run.chars().take(RUN_CAP).collect();
    format!("cf-subway-{bounded_run}-{role}")
}

/// The harness session id ON the host: a persistent session dir for a `run`-lifetime station,
/// `ephemeral` otherwise.
fn session_id(record: &Value, role: &str) -> String {
    let persistent = record
        .get("instances")
        .and_then(|i| i.get(role))
        .and_then(|v| v.get("lifetime"))
        .and_then(Value::as_str)
        == Some("run");
    if persistent {
        let state_dir = std::env::var("HOME").unwrap_or_default() + "/.omp/state";
        format!("{state_dir}/sessions/{role}")
    } else {
        "ephemeral".to_string()
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    #[test]
    fn empty_record_resolves_local_host_and_default_surface() {
        let b = resolve_role("coordinator", "r1", &json!({})).unwrap();
        assert_eq!(b.host.name, "local");
        assert_eq!(b.transport, TransportKind::LocalSocket);
        assert_eq!(b.adapter, AdapterKind::Cmux);
        assert_eq!(b.surface_ref, "cf-subway-r1:coordinator");
        assert_eq!(b.session_id, "ephemeral");
        assert_eq!(b.route.host, "local");
        assert_eq!(b.route.surface, "omp");
        assert_eq!(b.route.generation, 0);
    }

    #[test]
    fn host_first_resolves_transport_from_registry() {
        let record = json!({
            "host": "thalixs-mbp",
            "hosts": { "thalixs-mbp": "tailscale" }
        });
        let b = resolve_role("coordinator", "r1", &record).unwrap();
        assert_eq!(b.host.name, "thalixs-mbp");
        assert_eq!(b.transport, TransportKind::Tailscale);
        // adapter + surface resolve on that host, unchanged by the remote transport
        assert_eq!(b.adapter, AdapterKind::Cmux);
        assert_eq!(b.route.host, "thalixs-mbp");
    }

    #[test]
    fn remote_transport_is_named_not_built() {
        // The contract carries host + transport from day one; the remote transport is NAMED here
        // (tailscale) even though only local-socket is implemented. Resolution must still succeed.
        for (host, transport) in [
            ("thalixs-mbp", TransportKind::Tailscale),
            ("ec2-control", TransportKind::Ssh),
            ("omniroute", TransportKind::HerdrMachine),
            ("sbox-7", TransportKind::SboxRemote),
        ] {
            let record = json!({ "host": host, "hosts": { host: transport.as_str() } });
            let b = resolve_role("coordinator", "r1", &record).unwrap();
            assert_eq!(
                b.transport, transport,
                "host {host} resolves its named transport"
            );
        }
    }

    #[test]
    fn nonlocal_host_without_registry_entry_errors_loudly() {
        let record = json!({ "host": "ec2-control" });
        let err = resolve_role("coordinator", "r1", &record).unwrap_err();
        assert!(
            matches!(err, ResolveError::UnknownHost { .. }),
            "got {err:?}"
        );
    }

    #[test]
    fn unknown_transport_errors_loudly() {
        let record = json!({ "host": "ec2-control", "hosts": { "ec2-control": "carrier-pigeon" } });
        let err = resolve_role("coordinator", "r1", &record).unwrap_err();
        assert!(
            matches!(err, ResolveError::UnknownTransport { .. }),
            "got {err:?}"
        );
    }

    #[test]
    fn surface_precedence_explicit_override_wins() {
        let record = json!({
            "overrides": { "coordinator": { "surface": "omp" } },
            "fleet": { "generation": 5 },
            "cos_surface": "omp"
        });
        let b = resolve_role("coordinator", "r1", &record).unwrap();
        // explicit override (omp) beats the fleet record default (herdr)
        assert_eq!(b.adapter, AdapterKind::Cmux);
        assert_eq!(b.route.surface, "omp");
        assert_eq!(b.route.generation, 5);
    }

    #[test]
    fn surface_precedence_fleet_record_beats_hint() {
        // fleet record resolves to herdr (G5 default); the hint names herdr too → consistent.
        let record = json!({ "fleet": { "generation": 3 }, "cos_surface": "herdr" });
        let b = resolve_role("coordinator", "r1", &record).unwrap();
        assert_eq!(b.adapter, AdapterKind::Herdr);
        assert_eq!(b.route.surface, "herdr");
        assert_eq!(b.route.generation, 3);
    }

    #[test]
    fn surface_precedence_hint_beats_default() {
        let record = json!({ "cos_surface": "herdr" });
        let b = resolve_role("coordinator", "r1", &record).unwrap();
        assert_eq!(b.adapter, AdapterKind::Herdr);
        assert_eq!(b.route.surface, "herdr");
    }

    #[test]
    fn stale_hint_contradicting_binding_errors_loudly() {
        // fleet record resolves to herdr; the hint names omp → stale → loud error, no silent fork.
        let record = json!({ "fleet": { "generation": 3 }, "cos_surface": "omp" });
        let err = resolve_role("coordinator", "r1", &record).unwrap_err();
        assert!(
            matches!(err, ResolveError::StaleSurfaceHint { .. }),
            "got {err:?}"
        );
        let msg = err.to_string();
        assert!(
            msg.contains("stale") && msg.contains("omp") && msg.contains("herdr"),
            "got {msg:?}"
        );
    }

    #[test]
    fn fleet_override_resolves_surface_via_g5() {
        // A fleet override to omp over the herdr default, carrying a generation stamp.
        let record = json!({
            "fleet": { "override": { "surface": "omp" }, "generation": 7 }
        });
        let b = resolve_role("coordinator", "r1", &record).unwrap();
        assert_eq!(b.adapter, AdapterKind::Cmux);
        assert_eq!(b.route.surface, "omp");
        assert_eq!(b.route.generation, 7);
    }

    #[test]
    fn fleet_named_blank_override_key_reports_g5_error() {
        let record = json!({ "fleet": { "override": { "surface": "  " }, "generation": 1 } });
        let err = resolve_role("coordinator", "r1", &record).unwrap_err();
        assert!(matches!(err, ResolveError::Fleet(_)), "got {err:?}");
    }

    #[test]
    fn sbox_allocation_binds_sbox_adapter() {
        let record = json!({ "allocation": { "target_sandbox": "sb-1" } });
        let b = resolve_role("coordinator", "r1", &record).unwrap();
        assert_eq!(b.adapter, AdapterKind::Sbox);
        assert_eq!(b.surface_ref, "sbox:sb-1");
    }

    #[test]
    fn split_environment_binds_per_role() {
        let record = json!({
            "allocation": { "target_sandbox": "sb-a" },
            "environments": { "coordinator": "sb-b" }
        });
        let b = resolve_role("coordinator", "r1", &record).unwrap();
        assert_eq!(b.adapter, AdapterKind::Sbox);
        assert_eq!(b.surface_ref, "sbox:sb-b");
    }

    #[test]
    fn explicit_instance_adapter_wins_over_surface_and_sbox() {
        let record = json!({
            "host": "local",
            "instances": { "coordinator": { "adapter": "herdr" } },
            "fleet": { "generation": 1 }
        });
        let b = resolve_role("coordinator", "r1", &record).unwrap();
        assert_eq!(b.adapter, AdapterKind::Herdr);
        assert_eq!(b.surface_ref, "cf-subway-r1-coordinator");
    }

    #[test]
    fn unknown_instance_adapter_errors_loudly() {
        let record = json!({ "instances": { "coordinator": { "adapter": "pigeon" } } });
        let err = resolve_role("coordinator", "r1", &record).unwrap_err();
        assert!(
            matches!(err, ResolveError::UnknownAdapter(_)),
            "got {err:?}"
        );
    }

    #[test]
    fn persistent_station_session_is_a_dir() {
        let record = json!({ "instances": { "coordinator": { "lifetime": "run" } } });
        let b = resolve_role("coordinator", "r1", &record).unwrap();
        assert!(
            b.session_id.ends_with("/sessions/coordinator"),
            "got {}",
            b.session_id
        );
    }

    #[test]
    fn binding_serializes_with_host_and_transport() {
        let record = json!({
            "host": "thalixs-mbp",
            "hosts": { "thalixs-mbp": "tailscale" },
            "fleet": { "generation": 4 }
        });
        let b = resolve_role("coordinator", "r1", &record).unwrap();
        let v = serde_json::to_value(&b).unwrap();
        assert_eq!(v["host"]["name"], "thalixs-mbp");
        assert_eq!(v["transport"], "tailscale");
        assert_eq!(v["adapter"], "herdr");
        assert_eq!(v["route"]["host"], "thalixs-mbp");
        assert_eq!(v["route"]["generation"], 4);
    }
}
