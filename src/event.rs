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

/// Errors [`send_event`] reports loudly — never a silent fork to another adapter.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum SendError {
    /// The binding's adapter has no messaging entry in the MESSAGING map. `Sbox` is a station bound
    /// to a sandbox whose send transport is the sandbox adapter's concern, not a surface template.
    NoMessagingAdapter(AdapterKind),
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
        }
    }
}

impl std::error::Error for SendError {}

/// The one emit primitive (R1/R5). Dispatches by `binding.adapter` — never a seat-typed literal —
/// selecting the peer-messaging tool from the folded SURFACE→ADAPTER map and addressing the
/// binding's resolved backend surface ON the host. A binding whose adapter has no messaging entry
/// (Sbox) errors loudly rather than silently re-routing through cmux.
pub fn send_event(binding: &StationBinding, event: &Event) -> Result<Dispatch, SendError> {
    let msg = messaging_for_adapter(binding.adapter)
        .ok_or(SendError::NoMessagingAdapter(binding.adapter))?;
    Ok(Dispatch {
        adapter: binding.adapter,
        tool: msg.tool,
        target: binding.surface_ref.clone(),
        text: event.payload.clone(),
    })
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

    #[test]
    fn send_event_dispatches_cmux_by_adapter() {
        let b = binding(AdapterKind::Cmux);
        let d = send_event(&b, &Event::new("coordinator", "done")).unwrap();
        assert_eq!(d.adapter, AdapterKind::Cmux);
        assert_eq!(d.tool, "cmux-axi");
        assert_eq!(d.target, "cf-subway-r1:coordinator");
        assert_eq!(d.text, "done");
    }

    #[test]
    fn send_event_dispatches_herdr_by_adapter() {
        let b = binding(AdapterKind::Herdr);
        let d = send_event(&b, &Event::new("coordinator", "done")).unwrap();
        assert_eq!(d.adapter, AdapterKind::Herdr);
        assert_eq!(d.tool, "herdr");
    }

    #[test]
    fn send_event_follows_binding_adapter_not_surface_string() {
        // An explicit herdr adapter resolves over an `omp` surface (resolve_role's instance override
        // path); the dispatch must follow `binding.adapter`, never the surface string.
        let mut b = binding(AdapterKind::Herdr);
        b.route.surface = "omp".to_string();
        let d = send_event(&b, &Event::new("coordinator", "x")).unwrap();
        assert_eq!(d.tool, "herdr");
        assert_ne!(d.tool, "cmux-axi");
    }

    #[test]
    fn send_event_sbox_has_no_messaging_template() {
        let b = binding(AdapterKind::Sbox);
        let err = send_event(&b, &Event::new("coordinator", "x")).unwrap_err();
        assert!(
            matches!(err, SendError::NoMessagingAdapter(AdapterKind::Sbox)),
            "got {err:?}"
        );
    }

    #[test]
    fn dispatch_serializes_with_adapter_tool_target_text() {
        let d = send_event(
            &binding(AdapterKind::Cmux),
            &Event::new("coordinator", "hi"),
        )
        .unwrap();
        let v = serde_json::to_value(&d).unwrap();
        assert_eq!(v["adapter"], "cmux");
        assert_eq!(v["tool"], "cmux-axi");
        assert_eq!(v["target"], "cf-subway-r1:coordinator");
        assert_eq!(v["text"], "hi");
    }
}
