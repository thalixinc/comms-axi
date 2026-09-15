//! R1/R5 — the one emit primitive.
//!
//! A seat's ONLY comm surface is an [`Event`] addressed to a role; [`send_event`] dispatches it to
//! the adapter the resolver selected. The seat names no tool, no project token, no roster index,
//! no surface — the runtime supplies them all. This collapses today's producer-side verbs
//! (`cmux-axi send` / `herdr agent prompt` / `herdr-axi report` / `abridge::send`) into a single
//! entry point that dispatches by `binding.adapter`, never a seat-typed literal.

use std::fmt;

use serde::Serialize;

use crate::adapter::{messaging_for_adapter, AdapterKind, StationBinding};
use crate::delivery::{self, Delivery, DeliveryError};

/// An event addressed to a role (R1) — the seat's only comm surface. `to` is the recipient role;
/// `payload` is the message body. Everything else (tool, project, roster, surface) is a runtime
/// concern the layer supplies.
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub struct Event {
    pub to: String,
    pub payload: String,
}

impl Event {
    pub fn new(to: impl Into<String>, payload: impl Into<String>) -> Self {
        Event {
            to: to.into(),
            payload: payload.into(),
        }
    }
}

/// The concrete adapter dispatch [`send_event`] produces — the collapsed producer verb. `tool` is
/// the peer-messaging tool from the folded [`MESSAGING`](crate::adapter::MESSAGING) map, `target`
/// is the binding's resolved backend surface ON the host (a handle the seat never knows), and
/// `text` is the event payload. Local-first: it is rendered, not executed — spawning it is the
/// transport adapter's job (additive for the remote transports).
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub struct Dispatch {
    pub adapter: AdapterKind,
    pub tool: &'static str,
    pub target: String,
    pub text: String,
}

/// What [`send_event`] actually did. `Render` is the herdr render-only path (the default, and the
/// fallback on a version mismatch); `Deliver` is a real cross-host channel to open (epic #31),
/// gated behind `CF_COMMS_MODE=comms` + a remote transport.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum SendOutcome {
    Render(Dispatch),
    Deliver(Delivery),
}

/// Errors [`send_event`] reports loudly — never a silent fork to another adapter.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum SendError {
    /// The binding's adapter has no messaging entry in the MESSAGING map. `Sbox` is a station bound
    /// to a sandbox whose send transport is the sandbox adapter's concern, not a surface template.
    NoMessagingAdapter(AdapterKind),
    /// Cross-host delivery failed loudly (gate off is NOT an error — it falls back to render;
    /// these are the genuine failures: no channel, or a version handshake mismatch).
    Delivery(DeliveryError),
}

impl fmt::Display for SendError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            SendError::NoMessagingAdapter(a) => {
                write!(
                    f,
                    "adapter {a:?} has no messaging entry in the MESSAGING map"
                )
            }
            SendError::Delivery(e) => write!(f, "cross-host delivery failed: {e}"),
        }
    }
}

impl std::error::Error for SendError {}

/// The one emit primitive (R1/R5, now with epic #31 delivery). Dispatches by `binding.adapter` —
/// never a seat-typed literal. When `binding.transport` is cross-host (`ssh`/`tailscale`/
/// `herdr-machine`) AND the ADDITIVE gate is on (`CF_COMMS_MODE=comms`), it builds a REAL channel
/// ([`SendOutcome::Deliver`]); otherwise it renders the herdr producer verb
/// ([`SendOutcome::Render`]) — byte-stable vs today. `mode` is `CF_COMMS_MODE` (the caller reads
/// the env once); a version handshake mismatch or a non-deliverable transport falls back to render,
/// never half-delivers.
pub fn send_event(
    binding: &StationBinding,
    event: &Event,
    mode: Option<&str>,
) -> Result<SendOutcome, SendError> {
    // Cross-host delivery: ADDITIVE + default-off + version-gated (transition rules 1–2).
    if binding.transport.is_cross_host() && delivery::gate(mode) {
        match delivery::deliver_event(binding, event, mode) {
            Ok(d) => return Ok(SendOutcome::Deliver(d)),
            // A version mismatch or a non-deliverable transport falls back to herdr render — loud,
            // never a half-delivery. (Disabled can't reach here: gate(mode) already passed.)
            Err(DeliveryError::VersionMismatch { .. }) | Err(DeliveryError::NoChannel(_)) => {
                // fall through to render below
            }
            Err(e) => return Err(SendError::Delivery(e)),
        }
    }

    let msg = messaging_for_adapter(binding.adapter)
        .ok_or(SendError::NoMessagingAdapter(binding.adapter))?;
    Ok(SendOutcome::Render(Dispatch {
        adapter: binding.adapter,
        tool: msg.tool,
        target: binding.surface_ref.clone(),
        text: event.payload.clone(),
    }))
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::adapter::{HostRef, Route, TransportKind, WakeKind};

    fn binding(adapter: AdapterKind) -> StationBinding {
        StationBinding {
            adapter,
            host: HostRef {
                name: "local".to_string(),
            },
            transport: TransportKind::LocalSocket,
            surface_ref: "cf-subway-r1:coordinator".to_string(),
            session_id: "ephemeral".to_string(),
            route: Route {
                host: "local".to_string(),
                surface: "omp".to_string(),
                generation: 0,
            },
            wake: WakeKind::HeartbeatPoll,
        }
    }

    /// The render-only Dispatch a default (gate-off, local-socket) send produces.
    fn render(b: &StationBinding, e: &Event) -> Dispatch {
        match send_event(b, e, None).unwrap() {
            SendOutcome::Render(d) => d,
            other => panic!("expected Render, got {other:?}"),
        }
    }

    #[test]
    fn send_event_dispatches_cmux_by_adapter() {
        let b = binding(AdapterKind::Cmux);
        let d = render(&b, &Event::new("coordinator", "done"));
        assert_eq!(d.adapter, AdapterKind::Cmux);
        assert_eq!(d.tool, "cmux-axi");
        assert_eq!(d.target, "cf-subway-r1:coordinator");
        assert_eq!(d.text, "done");
    }

    #[test]
    fn send_event_dispatches_herdr_by_adapter() {
        let b = binding(AdapterKind::Herdr);
        let d = render(&b, &Event::new("coordinator", "done"));
        assert_eq!(d.adapter, AdapterKind::Herdr);
        assert_eq!(d.tool, "herdr");
    }

    #[test]
    fn send_event_follows_binding_adapter_not_surface_string() {
        // An explicit herdr adapter resolves over an `omp` surface (resolve_role's instance override
        // path); the dispatch must follow `binding.adapter`, never the surface string.
        let mut b = binding(AdapterKind::Herdr);
        b.route.surface = "omp".to_string();
        let d = render(&b, &Event::new("coordinator", "x"));
        assert_eq!(d.tool, "herdr");
        assert_ne!(d.tool, "cmux-axi");
    }

    #[test]
    fn send_event_sbox_has_no_messaging_template() {
        let b = binding(AdapterKind::Sbox);
        let err = send_event(&b, &Event::new("coordinator", "x"), None).unwrap_err();
        assert!(
            matches!(err, SendError::NoMessagingAdapter(AdapterKind::Sbox)),
            "got {err:?}"
        );
    }

    #[test]
    fn dispatch_serializes_with_adapter_tool_target_text() {
        let d = render(
            &binding(AdapterKind::Cmux),
            &Event::new("coordinator", "hi"),
        );
        let v = serde_json::to_value(&d).unwrap();
        assert_eq!(v["adapter"], "cmux");
        assert_eq!(v["tool"], "cmux-axi");
        assert_eq!(v["target"], "cf-subway-r1:coordinator");
        assert_eq!(v["text"], "hi");
    }

    /// A cross-host binding (herdr-machine transport) so the delivery path is reachable.
    fn cross_host_binding() -> StationBinding {
        let mut b = binding(AdapterKind::Herdr);
        b.host.name = "thalixs-mbp".to_string();
        b.transport = TransportKind::Ssh;
        b
    }

    #[test]
    fn send_event_delivers_cross_host_when_gate_on() {
        let b = cross_host_binding();
        let out = send_event(&b, &Event::new("cof", "hi"), Some("comms")).unwrap();
        match out {
            SendOutcome::Deliver(d) => {
                assert_eq!(d.transport, TransportKind::Ssh);
                assert_eq!(d.host, "thalixs-mbp");
                assert_eq!(d.argv[0], "ssh");
                assert_eq!(d.envelope.to, "cof");
            }
            other => panic!("expected Deliver, got {other:?}"),
        }
    }

    #[test]
    fn send_event_renders_when_gate_off_even_if_cross_host() {
        // Transition rule 1: default-off. A remote transport with CF_COMMS_MODE unset is render-only
        // (byte-stable vs today), never a channel.
        let b = cross_host_binding();
        let out = send_event(&b, &Event::new("cof", "hi"), None).unwrap();
        match out {
            SendOutcome::Render(d) => assert_eq!(d.adapter, AdapterKind::Herdr),
            other => panic!("expected Render (gate off), got {other:?}"),
        }
    }

    #[test]
    fn send_event_renders_for_local_socket_even_when_gate_on() {
        // `local-socket` has no cross-host channel; gate on still renders (in-host herdr path).
        let b = binding(AdapterKind::Herdr);
        let out = send_event(&b, &Event::new("cof", "hi"), Some("comms")).unwrap();
        assert!(matches!(out, SendOutcome::Render(_)));
    }
}
