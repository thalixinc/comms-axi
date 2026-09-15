//! S4 — SESSION INJECTION (epic #31, ticket #34): turn the enqueued handoff into a NEW TURN in
//! the target cof session — pull, never push.
//!
//! This is the wiring between the two shipped halves: the S3 wake (file-stat `hasMail`, decides
//! re-wake vs no-op) and the S2 read (drain the queue + clear `hasMail`, render the drained events
//! as ONE turn). On an idle tick: stat the marker; if the station has mail, drain the queue
//! EXACTLY ONCE (S2 `read(all=true)` clears `hasMail`) and render the events as one turn; else a
//! silent no-op (zero tokens — no queue read, no prompt). A non-idle station is never touched —
//! no mid-turn interrupt.
//!
//! **Pull, never push** is the hard rule: the injection only fires on an idle heartbeat with
//! pending mail; it never resolves-and-fires into a live session, never interrupts an active turn.
//! The turn events carry their STABLE `effect_id` (the #35 dedup axis, established by the #33
//! listener and passed through S2 read) so dedup is additive later.

use std::path::Path;

use serde::Serialize;

use crate::error::Result;
use crate::read::{read, ReadEvent};
use crate::wake::{idle_tick, WakeAction};

/// What an idle-tick injection produced: a turn to inject, or a silent no-op.
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
#[serde(tag = "action", rename_all = "kebab-case")]
pub enum InjectOutcome {
    /// The station was idle and had mail: the drained events form ONE turn (pull, never push).
    Turn {
        role: String,
        events: Vec<ReadEvent>,
        remaining: usize,
        has_mail: bool,
    },
    /// No mail, or the station is not idle: silent no-op — zero tokens, no queue read.
    NoOp,
}

/// S4 — the pull-on-idle injection. On an idle tick, stat the station's `hasMail` marker (S3
/// wake); if it has mail, drain the queue EXACTLY ONCE (S2 read — admits each event in the journal
/// and clears `hasMail` when it empties) and render the drained events as one turn. Pull, never
/// push: a non-idle station is never touched (no mid-turn interrupt); an empty inbox is a
/// file-stat no-op (zero tokens — the queue is never opened).
pub fn inject(state_dir: &Path, role: &str, idle: bool) -> Result<InjectOutcome> {
    match idle_tick(state_dir, role, idle) {
        WakeAction::ReWake => {
            let r = read(state_dir, role, true)?;
            Ok(InjectOutcome::Turn {
                role: r.role,
                events: r.events,
                remaining: r.remaining,
                has_mail: r.has_mail,
            })
        }
        WakeAction::NoOp => Ok(InjectOutcome::NoOp),
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::delivery::stable_effect_id;
    use crate::send::{has_mail, load_queue, send_to, send_to_with_id};
    use std::path::PathBuf;

    fn tmpdir(tag: &str) -> PathBuf {
        let dir = std::env::temp_dir().join(format!(
            "comms-axi-inject-{tag}-{}-{:?}",
            std::process::id(),
            std::thread::current().id()
        ));
        let _ = std::fs::remove_dir_all(&dir);
        std::fs::create_dir_all(&dir).unwrap();
        dir
    }

    #[test]
    fn inject_turns_when_idle_with_mail() {
        let dir = tmpdir("turn");
        send_to(&dir, "cof", "dev-1", "done: PR #9").unwrap();
        let out = inject(&dir, "cof", true).unwrap();
        match out {
            InjectOutcome::Turn {
                role,
                events,
                remaining,
                has_mail: hm,
            } => {
                assert_eq!(role, "cof");
                assert_eq!(events.len(), 1);
                assert_eq!(events[0].from, "dev-1");
                assert_eq!(events[0].text, "done: PR #9");
                assert_eq!(remaining, 0);
                assert!(!hm);
            }
            other => panic!("expected Turn, got {other:?}"),
        }
        // Consumed: the queue drained and hasMail cleared.
        assert!(!has_mail(&dir, "cof"));
        assert!(load_queue(&dir, "cof").unwrap().is_empty());
    }

    #[test]
    fn inject_is_no_op_when_no_mail() {
        let dir = tmpdir("no-mail");
        // Empty inbox: file-stat only, no queue read. Zero tokens.
        let out = inject(&dir, "cof", true).unwrap();
        assert!(matches!(out, InjectOutcome::NoOp));
    }

    #[test]
    fn inject_never_touches_non_idle() {
        let dir = tmpdir("non-idle");
        send_to(&dir, "cof", "dev-1", "hi").unwrap();
        // Non-idle: the producer's message must NOT interrupt an active turn.
        let out = inject(&dir, "cof", false).unwrap();
        assert!(matches!(out, InjectOutcome::NoOp));
        // Queue untouched — still pending for a later idle tick.
        assert!(has_mail(&dir, "cof"));
        assert_eq!(load_queue(&dir, "cof").unwrap().len(), 1);
    }

    #[test]
    fn inject_consumes_queue_exactly_once() {
        let dir = tmpdir("once");
        send_to(&dir, "cof", "dev-1", "only once").unwrap();
        let first = inject(&dir, "cof", true).unwrap();
        assert!(matches!(first, InjectOutcome::Turn { .. }));
        // A second injection finds an empty queue → no-op, never a re-injection.
        let second = inject(&dir, "cof", true).unwrap();
        assert!(matches!(second, InjectOutcome::NoOp));
    }

    #[test]
    fn inject_carries_stable_effect_id_axis() {
        // A cross-host handoff enqueues UNDER the delivery envelope's STABLE id (#33 listener);
        // the injection turn must carry that id (not a local seq) so #35 dedup is additive.
        let dir = tmpdir("stable-axis");
        let stable = stable_effect_id("cof", "done: PR #9");
        send_to_with_id(&dir, "cof", "remote", "done: PR #9", &stable).unwrap();
        let out = inject(&dir, "cof", true).unwrap();
        match out {
            InjectOutcome::Turn { events, .. } => {
                assert_eq!(events.len(), 1);
                assert_eq!(events[0].effect_id, stable);
            }
            other => panic!("expected Turn, got {other:?}"),
        }
    }

    #[test]
    fn inject_outcome_serializes() {
        let dir = tmpdir("serialize");
        send_to(&dir, "cof", "dev-1", "hi").unwrap();
        let turn = serde_json::to_value(inject(&dir, "cof", true).unwrap()).unwrap();
        assert_eq!(turn["action"], "turn");
        assert_eq!(turn["role"], "cof");
        assert_eq!(turn["events"][0]["text"], "hi");

        let noop = serde_json::to_value(inject(&dir, "cof", true).unwrap()).unwrap();
        assert_eq!(noop["action"], "no-op");
    }
}
