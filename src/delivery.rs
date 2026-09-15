//! Cross-host DELIVERY (epic #31): open a real channel and deliver an envelope to the host a role
//! lives on, once `binding.transport` is `ssh`/`tailscale`/`herdr-machine` (not `local-socket`).
//!
//! This is the SENDER half. `send_event` (the one emit primitive) delegates here when the
//! ADDITIVE gate is on (`CF_COMMS_MODE=comms`) AND the binding is cross-host. It opens a REAL
//! channel — `ssh <host> …` (exec), a `tailscale ssh` socket, or `herdr machine <host> …` — and
//! delivers a [`Envelope`] (version + effect id + target + payload) to the target host's inbound
//! listener (#33). That listener enqueues for the target cof and the shipped pull-on-idle +
//! `hasMail` model (#19) injects it as a NEW TURN — pull, never push.
//!
//! **Herdr REMAINS the carrier.** comms-axi rides herdr; it does NOT fork a second comm path,
//! revive cmux, or replace herdr. `herdr-machine` is literally a `herdr` invocation to a machine.
//!
//! **Transition rules (hard, from spec.md):**
//! 1. **ADDITIVE + default-off** — delivery only under `CF_COMMS_MODE=comms`; the default (unset)
//!    is herdr render-only, byte-stable vs today. [`gate`] decides.
//! 2. **VERSION-GATED handshake** — the envelope carries [`PROTOCOL_VERSION`]; [`handshake`] fails
//!    loudly on an old↔new mismatch and the caller falls back to herdr render. Never half-delivers.
//! 3. **CANARY-first, per-factory** — transport is selected per-binding by `binding.transport`,
//!    never a seat-typed surface; one factory opts in at a time (revert = unset).
//! 4. **NO teardown, preserve in-flight** — no flag-day cutover; effect-id dedup (#35) survives a
//!    mid-swap re-send. The envelope carries a STABLE effect id so #35 can dedup on it.
//!
//! The core ([`plan`], [`handshake`], [`gate`], [`stable_effect_id`]) is PURE — no fs/env/clock —
//! so the channel argv + envelope + gate + version checks are deterministic and testable. The thin
//! spawn ([`Delivery::open`]) is the only I/O; it is the bin's job to invoke it, not the core's.

use std::fmt;
use std::process::Command;

use serde::{Deserialize, Serialize};

use crate::adapter::{StationBinding, TransportKind};
use crate::event::Event;

/// The comms protocol version both ends must speak. An old↔new mismatch is a loud handshake
/// failure, never a silent half-delivery. Bumped only when the delivery wire contract changes.
pub const PROTOCOL_VERSION: u32 = 1;

/// The `CF_COMMS_MODE` value that turns cross-host delivery ON. Anything else (unset, empty, or a
/// different value) = herdr render-only, byte-stable.
pub const COMMS_MODE: &str = "comms";

/// The envelope delivered over a cross-host channel. `version` gates the handshake; `effect_id` is
/// the STABLE id the inbound listener journals for #35 dedup (a re-delivery is idempotent); `to`
/// is the target cof role; `payload` is the turn text to inject.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct Envelope {
    pub version: u32,
    pub effect_id: String,
    pub to: String,
    pub payload: String,
}

/// A concrete cross-host delivery: the transport, the target host, the exact channel argv to
/// spawn, and the envelope it injects. `argv[0]` is the transport binary; the rest open the channel
/// and hand the envelope to the remote inbound listener.
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub struct Delivery {
    pub transport: TransportKind,
    pub host: String,
    pub argv: Vec<String>,
    pub envelope: Envelope,
}

/// Errors cross-host delivery reports loudly — never a silent fork to render, never a half-send.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum DeliveryError {
    /// The ADDITIVE gate is off (`CF_COMMS_MODE` unset or not `comms`) — herdr render-only.
    Disabled,
    /// The binding's transport has no cross-host channel (`local-socket` / `sbox-remote`).
    NoChannel(TransportKind),
    /// The version-gated handshake failed: the two ends speak different protocol versions.
    VersionMismatch { local: u32, remote: u32 },
}

impl fmt::Display for DeliveryError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            DeliveryError::Disabled => write!(
                f,
                "cross-host delivery is disabled: set CF_COMMS_MODE={COMMS_MODE} to opt in \
                 (default is herdr render-only)"
            ),
            DeliveryError::NoChannel(t) => write!(
                f,
                "transport {t:?} has no cross-host channel; nothing to deliver"
            ),
            DeliveryError::VersionMismatch { local, remote } => write!(
                f,
                "version handshake failed: local speaks v{local}, remote speaks v{remote} — \
                 falling back to herdr render (never half-delivers)"
            ),
        }
    }
}

impl std::error::Error for DeliveryError {}

/// The version-gated handshake (transition rule 2): both ends must speak the SAME protocol version.
/// A mismatch is LOUD — the caller falls back to herdr render, never half-delivers.
pub fn handshake(local: u32, remote: u32) -> Result<(), DeliveryError> {
    if local == remote {
        Ok(())
    } else {
        Err(DeliveryError::VersionMismatch { local, remote })
    }
}

/// The ADDITIVE gate (transition rule 1). PURE: `CF_COMMS_MODE` must be exactly [`COMMS_MODE`];
/// anything else is render-only. Reads its input, never the process env, so it is deterministic.
pub fn gate(mode: Option<&str>) -> bool {
    matches!(mode, Some(m) if m.trim() == COMMS_MODE)
}

/// A STABLE effect id for one delivered envelope: FNV-1a over `to \0 payload`, hex. Deterministic
/// and dependency-free, so a mid-swap re-send of the same event yields the same id and #35's
/// journal dedup acknowledges-but-does-not-re-enact. Not a uniqueness promise across *distinct*
/// events — that residual is named honestly (#35), never hidden.
pub fn stable_effect_id(to: &str, payload: &str) -> String {
    const OFFSET: u64 = 0xcbf2_9ce4_8422_2325;
    const PRIME: u64 = 0x0000_0100_0000_01b3;
    let mut h = OFFSET;
    for b in to.bytes().chain(std::iter::once(0)).chain(payload.bytes()) {
        h ^= u64::from(b);
        h = h.wrapping_mul(PRIME);
    }
    format!("{h:016x}")
}

impl Delivery {
    /// Open the REAL channel: spawn the transport argv and wait. `argv[0]` is the transport binary
    /// (`ssh` / `tailscale` / `herdr`); the rest open the channel and inject the envelope. This is
    /// the only I/O in the module — the bin calls it on the `Deliver` path, the core never does.
    pub fn open(&self) -> std::io::Result<std::process::ExitStatus> {
        Command::new(&self.argv[0]).args(&self.argv[1..]).status()
    }
}

/// Build the concrete cross-host delivery for a binding: the channel argv (per transport) and the
/// version-stamped envelope. PURE — no spawn. `receiver_version` is the protocol version the remote
/// listener advertises; it must equal [`PROTOCOL_VERSION`] or the handshake fails loudly.
pub fn plan(
    binding: &StationBinding,
    event: &Event,
    receiver_version: u32,
) -> Result<Delivery, DeliveryError> {
    // Version-gated handshake BEFORE any channel is formed — never half-delivers.
    handshake(PROTOCOL_VERSION, receiver_version)?;

    let host = binding.host.name.clone();
    let envelope = Envelope {
        version: PROTOCOL_VERSION,
        effect_id: stable_effect_id(&event.to, &event.payload),
        to: event.to.clone(),
        payload: event.payload.clone(),
    };
    let envelope_json = serde_json::to_string(&envelope)
        .map_err(|_| DeliveryError::NoChannel(binding.transport))?; // unreachable: envelope is JSON-safe

    // The remote verb is `comms-axi deliver <envelope>` — the inbound listener's entry (#33). The
    // sender hands the envelope to it; the listener enqueues for the target cof.
    let argv = match binding.transport {
        TransportKind::Ssh => vec![
            "ssh".to_string(),
            host.clone(),
            "--".to_string(),
            "comms-axi".to_string(),
            "deliver".to_string(),
            envelope_json.clone(),
        ],
        TransportKind::Tailscale => vec![
            "tailscale".to_string(),
            "ssh".to_string(),
            host.clone(),
            "--".to_string(),
            "comms-axi".to_string(),
            "deliver".to_string(),
            envelope_json.clone(),
        ],
        // Herdr REMAINS the carrier: `herdr machine <host> …` rides herdr, never a second path.
        TransportKind::HerdrMachine => vec![
            "herdr".to_string(),
            "machine".to_string(),
            host.clone(),
            "--".to_string(),
            "comms-axi".to_string(),
            "deliver".to_string(),
            envelope_json.clone(),
        ],
        // local-socket / sbox-remote: no cross-host channel (loud, never a silent fork).
        other => return Err(DeliveryError::NoChannel(other)),
    };

    Ok(Delivery {
        transport: binding.transport,
        host,
        argv,
        envelope,
    })
}

/// The public cross-host delivery entry: checks the ADDITIVE gate (process env), then builds the
/// concrete delivery against the current protocol version. `Err(Disabled)` when `CF_COMMS_MODE`
/// isn't `comms` — the caller falls back to herdr render.
pub fn deliver_event(
    binding: &StationBinding,
    event: &Event,
    mode: Option<&str>,
) -> Result<Delivery, DeliveryError> {
    if !gate(mode) {
        return Err(DeliveryError::Disabled);
    }
    plan(binding, event, PROTOCOL_VERSION)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::adapter::{AdapterKind, HostRef, Route, WakeKind};

    fn binding(transport: TransportKind) -> StationBinding {
        StationBinding {
            adapter: AdapterKind::Herdr,
            host: HostRef {
                name: "thalixs-mbp".to_string(),
            },
            transport,
            surface_ref: "cof".to_string(),
            session_id: "sess-1".to_string(),
            route: Route {
                host: "thalixs-mbp".to_string(),
                surface: "herdr".to_string(),
                generation: 3,
            },
            wake: WakeKind::HeartbeatPoll,
        }
    }

    fn event() -> Event {
        Event::new("cof", "done: PR #9")
    }

    #[test]
    fn gate_requires_exactly_comms() {
        assert!(gate(Some("comms")));
        assert!(gate(Some(" comms ")));
        assert!(!gate(None));
        assert!(!gate(Some("")));
        assert!(!gate(Some("off")));
        assert!(!gate(Some("herdr")));
    }

    #[test]
    fn handshake_matches_ok_and_mismatch_loud() {
        assert!(handshake(1, 1).is_ok());
        assert!(handshake(0, 0).is_ok());
        let err = handshake(1, 2).unwrap_err();
        assert!(
            matches!(
                err,
                DeliveryError::VersionMismatch {
                    local: 1,
                    remote: 2
                }
            ),
            "got {err:?}"
        );
    }

    #[test]
    fn stable_effect_id_is_deterministic_and_content_sensitive() {
        let a = stable_effect_id("cof", "done");
        let b = stable_effect_id("cof", "done");
        let c = stable_effect_id("cof", "other");
        assert_eq!(a, b, "same content -> same id (idempotent re-send)");
        assert_ne!(a, c, "different payload -> different id");
        // `to` also participates (different recipient is a different effect).
        assert_ne!(a, stable_effect_id("coordinator", "done"));
    }

    #[test]
    fn plan_stamps_version_and_effect_id() {
        let d = plan(&binding(TransportKind::Ssh), &event(), PROTOCOL_VERSION).unwrap();
        assert_eq!(d.envelope.version, PROTOCOL_VERSION);
        assert_eq!(d.envelope.to, "cof");
        assert_eq!(d.envelope.payload, "done: PR #9");
        assert_eq!(d.envelope.effect_id, stable_effect_id("cof", "done: PR #9"));
    }

    #[test]
    fn plan_rejects_version_mismatch() {
        let err = plan(&binding(TransportKind::Ssh), &event(), 0).unwrap_err();
        assert!(
            matches!(
                err,
                DeliveryError::VersionMismatch {
                    local: 1,
                    remote: 0
                }
            ),
            "got {err:?}"
        );
    }

    #[test]
    fn plan_rejects_non_cross_host_transports() {
        for t in [TransportKind::LocalSocket, TransportKind::SboxRemote] {
            let err = plan(&binding(t), &event(), PROTOCOL_VERSION).unwrap_err();
            assert!(
                matches!(err, DeliveryError::NoChannel(_)),
                "transport {t:?}: got {err:?}"
            );
        }
    }

    #[test]
    fn ssh_argv_opens_a_real_channel() {
        let d = plan(&binding(TransportKind::Ssh), &event(), PROTOCOL_VERSION).unwrap();
        assert_eq!(d.argv[0], "ssh");
        assert_eq!(d.argv[1], "thalixs-mbp");
        assert_eq!(d.argv[2], "--");
        assert_eq!(d.argv[3], "comms-axi");
        assert_eq!(d.argv[4], "deliver");
        // The last arg is the JSON envelope (version-stamped), handed to the remote listener.
        let parsed: Envelope = serde_json::from_str(&d.argv[5]).unwrap();
        assert_eq!(parsed, d.envelope);
    }

    #[test]
    fn tailscale_argv_is_a_socket_over_the_tailnet() {
        let d = plan(
            &binding(TransportKind::Tailscale),
            &event(),
            PROTOCOL_VERSION,
        )
        .unwrap();
        assert_eq!(d.argv[0], "tailscale");
        assert_eq!(d.argv[1], "ssh");
        assert_eq!(d.argv[2], "thalixs-mbp");
        assert_eq!(d.argv[4], "comms-axi");
        assert_eq!(d.argv[5], "deliver");
    }

    #[test]
    fn herdr_machine_rides_herdr_not_a_second_path() {
        let d = plan(
            &binding(TransportKind::HerdrMachine),
            &event(),
            PROTOCOL_VERSION,
        )
        .unwrap();
        // Herdr REMAINS the carrier: argv[0] is `herdr`, not a new broker.
        assert_eq!(d.argv[0], "herdr");
        assert_eq!(d.argv[1], "machine");
        assert_eq!(d.argv[2], "thalixs-mbp");
        assert_eq!(d.argv[5], "deliver");
    }

    #[test]
    fn deliver_event_gates_on_mode() {
        assert!(matches!(
            deliver_event(&binding(TransportKind::Ssh), &event(), None),
            Err(DeliveryError::Disabled)
        ));
        assert!(matches!(
            deliver_event(&binding(TransportKind::Ssh), &event(), Some("off")),
            Err(DeliveryError::Disabled)
        ));
        let d = deliver_event(&binding(TransportKind::Ssh), &event(), Some("comms")).unwrap();
        assert_eq!(d.transport, TransportKind::Ssh);
        assert_eq!(d.envelope.version, PROTOCOL_VERSION);
    }

    #[test]
    fn is_cross_host_only_for_deliverable_transports() {
        assert!(TransportKind::Ssh.is_cross_host());
        assert!(TransportKind::Tailscale.is_cross_host());
        assert!(TransportKind::HerdrMachine.is_cross_host());
        assert!(!TransportKind::LocalSocket.is_cross_host());
        assert!(!TransportKind::SboxRemote.is_cross_host());
    }
}
