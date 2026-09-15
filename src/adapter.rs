//! The binding contract (R2/R5): the types `resolve_role` emits, host + transport-qualified.
//!
//! comms-axi routes ACROSS machines, not within one host (founder amendment). A role's agents may
//! live on any host in the fleet, reached over tailscale / ssh / herdr-machine / sbox-remote. The
//! seat's API is unchanged — only the resolver sees host/transport. The full `MESSAGING`
//! SURFACE→ADAPTER map (the #463 fold) lands in #11; here lives the binding shape itself.

use serde::Serialize;
use std::str::FromStr;

/// The transport adapters a binding may select (the SURFACE). `Cmux` is the legacy default;
/// `Sbox` is a station bound to a sandbox environment; `Herdr` is the transport of record.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize)]
#[serde(rename_all = "kebab-case")]
pub enum AdapterKind {
    Cmux,
    Sbox,
    Herdr,
}

impl AdapterKind {
    pub fn as_str(self) -> &'static str {
        match self {
            AdapterKind::Cmux => "cmux",
            AdapterKind::Sbox => "sbox",
            AdapterKind::Herdr => "herdr",
        }
    }
}

impl FromStr for AdapterKind {
    type Err = String;

    fn from_str(s: &str) -> Result<Self, Self::Err> {
        match s {
            "cmux" => Ok(AdapterKind::Cmux),
            "sbox" => Ok(AdapterKind::Sbox),
            "herdr" => Ok(AdapterKind::Herdr),
            _ => Err(format!("unknown adapter {s:?}")),
        }
    }
}

/// How a message reaches the host a role lives on. Only `LocalSocket` is built now; the remote
/// variants are NAMED (so the contract is host-aware from day one) but not yet implemented —
/// additive adapters later, never a redesign.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize)]
#[serde(rename_all = "kebab-case")]
pub enum TransportKind {
    LocalSocket,
    Tailscale,
    Ssh,
    HerdrMachine,
    SboxRemote,
}

impl TransportKind {
    pub fn as_str(self) -> &'static str {
        match self {
            TransportKind::LocalSocket => "local-socket",
            TransportKind::Tailscale => "tailscale",
            TransportKind::Ssh => "ssh",
            TransportKind::HerdrMachine => "herdr-machine",
            TransportKind::SboxRemote => "sbox-remote",
        }
    }
}

impl TransportKind {
    /// The transports that DELIVER cross-host (epic #31): `ssh` (exec a remote command),
    /// `tailscale` (a socket over the tailnet), `herdr-machine` (ride herdr — the carrier — to a
    /// machine). `LocalSocket` is the in-host render path and `SboxRemote` is the sandbox adapter's
    /// concern; neither opens a cross-host channel here.
    pub fn is_cross_host(self) -> bool {
        matches!(
            self,
            TransportKind::Ssh | TransportKind::Tailscale | TransportKind::HerdrMachine
        )
    }
}

impl FromStr for TransportKind {
    type Err = String;

    fn from_str(s: &str) -> Result<Self, Self::Err> {
        match s {
            "local-socket" => Ok(TransportKind::LocalSocket),
            "tailscale" => Ok(TransportKind::Tailscale),
            "ssh" => Ok(TransportKind::Ssh),
            "herdr-machine" => Ok(TransportKind::HerdrMachine),
            "sbox-remote" => Ok(TransportKind::SboxRemote),
            _ => Err(format!("unknown transport {s:?}")),
        }
    }
}

/// Which machine a role's station lives on (e.g. a tailscale name, an ssh target, or `local`).
/// Read from the fleet record's `host` field — never a static host list.
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub struct HostRef {
    pub name: String,
}

/// The fleet-resolved live handle, now host-qualified (founder amendment). Uniquely identifies
/// WHERE the binding lives — host + surface on that host + the observed-handle generation the run
/// actually used (G5 truthful stamp). Read from the fleet record, never derived statically.
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub struct Route {
    pub host: String,
    pub surface: String,
    pub generation: u64,
}

/// A resolved station binding (fleet-wide shape, R2). The seat never sees this; producers consume
/// it. `adapter` is the SURFACE, `host` + `transport` are the remote-aware dimension,
/// `surface_ref`/`session_id` are the ids ON that host, and `route` is the host-qualified handle.
/// `wake` reserves the wake mechanism (S5 scaffold): `HeartbeatPoll` is wired now (the S3
/// hasMail-gated idle heartbeat); `Webhook` is a named placeholder for ThalixRuntime later.
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub struct StationBinding {
    pub adapter: AdapterKind,
    pub host: HostRef,
    pub transport: TransportKind,
    pub surface_ref: String,
    pub session_id: String,
    pub route: Route,
    pub wake: WakeKind,
}

/// How a station is re-woken when it has pending mail (S5 scaffold). `HeartbeatPoll` is the only
/// wired variant — the S3 hasMail-gated idle heartbeat (omp/pi). `Webhook` is a NAMED placeholder
/// for ThalixRuntime's instant-wake, reserved so it lands as an additive adapter, not a re-design.
/// Do NOT build the webhook/acp/socket/API here — it is backlog.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Default)]
#[serde(rename_all = "kebab-case")]
pub enum WakeKind {
    /// The hasMail-gated idle heartbeat (omp/pi) — wired now.
    #[default]
    HeartbeatPoll,
    /// ThalixRuntime instant-wake — named/backlog, NOT built.
    Webhook,
}

impl WakeKind {
    pub fn as_str(self) -> &'static str {
        match self {
            WakeKind::HeartbeatPoll => "heartbeat-poll",
            WakeKind::Webhook => "webhook",
        }
    }
}

/// The peer-messaging contract a surface maps to (the #463 SURFACE→ADAPTER map, folded in).
/// `adapter` is the AdapterKind that surface binds; `tool` is the peer-messaging tool; the
/// `send`/`report` templates are the surface-specific verbs a producer renders. Derived from the
/// [`MESSAGING`] map — never a hard-coded `tool` column, never a cmux-vs-herdr `if/else`.
///
/// Template placeholders: `{PROJ}` project token, `{TARGET}` send target, `{TEXT}` message body,
/// `{STATUS_ARGS}` an omp-only status-verb suffix (absent on herdr).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize)]
pub struct MessagingAdapter {
    pub adapter: AdapterKind,
    /// The peer-messaging tool (the role card's `tool` column).
    pub tool: &'static str,
    /// The turn-report / return-channel tool (`herdr-axi` on herdr).
    pub report_tool: &'static str,
    /// The peer-send verb template (surface-specific syntax + target).
    pub send: &'static str,
    /// The turn-report verb template (return channel), surface-specific.
    pub report: &'static str,
}

/// SURFACE → ADAPTER MAP (the #463 fold): one entry per surface; `omp` (cmux) is the default entry
/// — an unset/unknown surface resolves it, back-compat with existing cmux crews.
pub const MESSAGING: &[(&str, MessagingAdapter)] = &[
    (
        "omp",
        MessagingAdapter {
            adapter: AdapterKind::Cmux,
            tool: "cmux-axi",
            report_tool: "cmux-axi",
            send: "cmux-axi send {PROJ} {TARGET} \"{TEXT}\"",
            report: "cmux-axi status --project {PROJ}{STATUS_ARGS}",
        },
    ),
    (
        "herdr",
        MessagingAdapter {
            adapter: AdapterKind::Herdr,
            tool: "herdr",
            report_tool: "herdr-axi",
            send: "herdr agent prompt {TARGET} \"{TEXT}\"",
            report: "herdr-axi report \"<text>\"",
        },
    ),
];

/// Resolve a surface to its messaging adapter, defaulting to the `omp` (cmux) entry for any
/// unset/unknown surface — the same precedence `resolve_surface` produces.
pub fn messaging_for(surface: &str) -> MessagingAdapter {
    MESSAGING
        .iter()
        .find(|(s, _)| *s == surface)
        .map(|(_, m)| *m)
        .unwrap_or(MESSAGING[0].1)
}

/// The messaging entry for an adapter kind — the lookup `send_event` uses, which dispatches on
/// `binding.adapter` (never a surface string). `None` for `Sbox`: a sandbox station's send
/// transport is the sandbox adapter's concern, not a surface template.
pub fn messaging_for_adapter(adapter: AdapterKind) -> Option<MessagingAdapter> {
    MESSAGING
        .iter()
        .find(|(_, m)| m.adapter == adapter)
        .map(|(_, m)| *m)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn messaging_map_resolves_surface_to_adapter() {
        assert_eq!(messaging_for("omp").adapter, AdapterKind::Cmux);
        assert_eq!(messaging_for("herdr").adapter, AdapterKind::Herdr);
    }

    #[test]
    fn messaging_map_defaults_unknown_surface_to_omp() {
        assert_eq!(messaging_for("bogus").adapter, AdapterKind::Cmux);
        assert_eq!(messaging_for("").adapter, AdapterKind::Cmux);
    }

    #[test]
    fn messaging_map_has_one_entry_per_surface() {
        assert_eq!(MESSAGING.len(), 2);
        let surfaces: Vec<&str> = MESSAGING.iter().map(|(s, _)| *s).collect();
        assert_eq!(surfaces, vec!["omp", "herdr"]);
    }

    #[test]
    fn messaging_for_adapter_maps_built_adapters() {
        assert_eq!(
            messaging_for_adapter(AdapterKind::Cmux).unwrap().tool,
            "cmux-axi"
        );
        assert_eq!(
            messaging_for_adapter(AdapterKind::Herdr).unwrap().tool,
            "herdr"
        );
        assert!(messaging_for_adapter(AdapterKind::Sbox).is_none());
    }

    #[test]
    fn transport_kind_names_round_trip() {
        for kind in [
            TransportKind::LocalSocket,
            TransportKind::Tailscale,
            TransportKind::Ssh,
            TransportKind::HerdrMachine,
            TransportKind::SboxRemote,
        ] {
            assert_eq!(TransportKind::from_str(kind.as_str()).unwrap(), kind);
        }
    }

    #[test]
    fn local_socket_is_the_built_transport() {
        assert_eq!(TransportKind::LocalSocket.as_str(), "local-socket");
    }

    #[test]
    fn wake_kind_defaults_to_heartbeat_poll() {
        // The only wired wake is the hasMail-gated heartbeat; Webhook is a named backlog variant.
        assert_eq!(WakeKind::default(), WakeKind::HeartbeatPoll);
        assert_eq!(WakeKind::HeartbeatPoll.as_str(), "heartbeat-poll");
        assert_eq!(WakeKind::Webhook.as_str(), "webhook");
    }

    #[test]
    fn wake_kind_serializes() {
        assert_eq!(
            serde_json::to_value(WakeKind::HeartbeatPoll).unwrap(),
            serde_json::json!("heartbeat-poll")
        );
        assert_eq!(
            serde_json::to_value(WakeKind::Webhook).unwrap(),
            serde_json::json!("webhook")
        );
    }
}
