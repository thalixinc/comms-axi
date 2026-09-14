//! R3 — the return-channel producer.
//!
//! comms-axi's `report` is the seat's OWN turn result: resolve the seat's own binding (via
//! [`resolve_role`](crate::resolve::resolve_role)), then delegate to the adapter's concrete report
//! mechanism — `herdr-axi report` on herdr, `cmux-axi status` on cmux. It NEVER re-implements the
//! adapter's transport, and it does NOT absorb the producer-side herdr-axi `report` verb (which
//! stays in herdr-axi); comms-axi addresses it via the adapter's `report_tool`, never a seat-typed
//! literal.

use std::fmt;

use serde::Serialize;
use serde_json::Value;

use crate::adapter::{messaging_for_adapter, AdapterKind};
use crate::resolve::{resolve_role, ResolveError};

/// The concrete return-channel delegation `report` produces — the collapsed report verb. `tool` is
/// the return-channel tool from the folded [`MESSAGING`](crate::adapter::MESSAGING) map
/// (`herdr-axi` on herdr, `cmux-axi` on cmux), and `text` is the seat's turn-result payload.
/// Local-first: it is rendered, not executed — spawning it is the transport adapter's job
/// (additive for the remote transports).
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub struct Report {
    pub adapter: AdapterKind,
    pub tool: &'static str,
    pub text: String,
}

/// Errors [`report`] reports loudly — never a silent fork to another adapter, never an absorbed
/// producer verb.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum ReportError {
    /// The seat's binding has no return-channel entry in the MESSAGING map (`Sbox`): its report
    /// transport is the sandbox adapter's concern, not a surface template.
    NoReportAdapter(AdapterKind),
    /// Resolving the seat's own binding failed (e.g. stale `cos_surface` hint).
    Resolve(ResolveError),
}

impl fmt::Display for ReportError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            ReportError::NoReportAdapter(a) => {
                write!(
                    f,
                    "adapter {a:?} has no return-channel entry in the MESSAGING map"
                )
            }
            ReportError::Resolve(e) => write!(f, "cannot resolve the seat's own binding: {e}"),
        }
    }
}

impl std::error::Error for ReportError {}

/// The return-channel producer (R3). Resolves the seat's OWN binding (via `resolve_role`), then
/// delegates to the adapter's report mechanism — selected by `binding.adapter`, never a seat-typed
/// literal. A binding whose adapter has no report entry (`Sbox`) errors loudly rather than
/// silently re-routing through cmux.
pub fn report(role: &str, run: &str, record: &Value, text: &str) -> Result<Report, ReportError> {
    let binding = resolve_role(role, run, record).map_err(ReportError::Resolve)?;
    let msg = messaging_for_adapter(binding.adapter)
        .ok_or(ReportError::NoReportAdapter(binding.adapter))?;
    Ok(Report {
        adapter: binding.adapter,
        tool: msg.report_tool,
        text: text.to_string(),
    })
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    #[test]
    fn report_resolves_own_binding_and_delegates_cmux() {
        // Default local binding → cmux → report_tool `cmux-axi`.
        let r = report("dev-1", "r1", &json!({}), "PR #15 done").unwrap();
        assert_eq!(r.adapter, AdapterKind::Cmux);
        assert_eq!(r.tool, "cmux-axi");
        assert_eq!(r.text, "PR #15 done");
    }

    #[test]
    fn report_delegates_herdr_return_channel() {
        // Fleet record → herdr surface → report_tool `herdr-axi` (the producer-side herdr-axi
        // verb stays in herdr-axi; comms-axi only names it via the adapter map).
        let record = json!({ "fleet": { "generation": 1 } });
        let r = report("dev-1", "r1", &record, "done").unwrap();
        assert_eq!(r.adapter, AdapterKind::Herdr);
        assert_eq!(r.tool, "herdr-axi");
        assert_eq!(r.text, "done");
    }

    #[test]
    fn report_follows_binding_adapter_not_surface_string() {
        // An explicit herdr adapter resolves over an `omp` surface (resolve_role's instance
        // override path); the report must follow `binding.adapter`, never the surface string.
        let record = json!({ "instances": { "dev-1": { "adapter": "herdr" } } });
        let r = report("dev-1", "r1", &record, "x").unwrap();
        assert_eq!(r.tool, "herdr-axi");
        assert_ne!(r.tool, "cmux-axi");
    }

    #[test]
    fn report_sbox_has_no_return_channel() {
        let record = json!({ "allocation": { "target_sandbox": "sb-1" } });
        let err = report("dev-1", "r1", &record, "x").unwrap_err();
        assert!(
            matches!(err, ReportError::NoReportAdapter(AdapterKind::Sbox)),
            "got {err:?}"
        );
    }

    #[test]
    fn report_surfaces_resolve_error_loudly() {
        // A stale cos_surface hint fails resolve_role; report must surface it, not swallow it.
        let record = json!({ "fleet": { "generation": 3 }, "cos_surface": "omp" });
        let err = report("dev-1", "r1", &record, "x").unwrap_err();
        assert!(matches!(err, ReportError::Resolve(_)), "got {err:?}");
        assert!(err.to_string().contains("stale"), "got {err:?}");
    }

    #[test]
    fn report_serializes_with_adapter_tool_text() {
        let r = report("dev-1", "r1", &json!({}), "hi").unwrap();
        let v = serde_json::to_value(&r).unwrap();
        assert_eq!(v["adapter"], "cmux");
        assert_eq!(v["tool"], "cmux-axi");
        assert_eq!(v["text"], "hi");
    }
}
