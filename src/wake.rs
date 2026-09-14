//! S3 — the hasMail-gated idle heartbeat wake (omp/pi).
//!
//! The wake layer answers ONE question on an idle tick: *does this station have mail?* — by
//! **file-stat only**, never by parsing the queue. `hasMail == true && idle` → re-wake (the
//! station then `read`s); `hasMail == false` → silent no-op (a marker stat, zero LLM tokens).
//!
//! FOUNDER NOTE: this is the SIMPLER hasMail-gated wake — a heartbeat that re-injects only when a
//! mail marker exists — NOT the #496 backoff/streaks/ladder (replaced). The wake does not itself
//! re-inject; it emits the decision the harness (S4, cf-side) consumes.
//!
//! The `hasMail` flag is the S1 marker at `<state-dir>/queues/<role>.hasmail` (existence == mail
//! pending). `idle_tick` stats that marker and nothing else, so an empty inbox costs a single
//! `exists()` syscall — never a JSON parse, never a prompt.

use std::path::Path;

use crate::send::has_mail;

/// The wake decision an idle tick produces.
#[derive(Debug, Clone, Copy, PartialEq, Eq, serde::Serialize)]
#[serde(rename_all = "kebab-case")]
pub enum WakeAction {
    /// The station has pending mail — re-wake it (the station then pulls its queue).
    ReWake,
    /// No pending mail — silent no-op. Zero tokens: the harness must NOT re-inject.
    NoOp,
}

impl WakeAction {
    pub fn as_str(self) -> &'static str {
        match self {
            WakeAction::ReWake => "re-wake",
            WakeAction::NoOp => "no-op",
        }
    }
}

/// The pure wake rule: re-wake only when idle AND there is mail. Total and side-effect-free —
/// the whole policy in one place, testable without the filesystem.
///
/// A non-idle station never wakes (a producer's message must not interrupt an active turn), and an
/// idle station with no mail is a silent no-op (no re-injection, zero tokens).
pub fn wake_decision(idle: bool, has_mail: bool) -> WakeAction {
    if idle && has_mail {
        WakeAction::ReWake
    } else {
        WakeAction::NoOp
    }
}

/// The idle-tick entry point: stat the station's `hasMail` marker and apply [`wake_decision`].
/// File-stat ONLY — it never opens or parses the queue record, so an empty inbox costs one
/// `exists()` syscall and nothing else. Infallible by construction (a single stat).
pub fn idle_tick(state_dir: &Path, role: &str, idle: bool) -> WakeAction {
    // A single stat of the marker. No queue read, no parse, no prompt.
    let pending = has_mail(state_dir, role);
    wake_decision(idle, pending)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::send::send_to;
    use std::path::PathBuf;

    fn tmpdir(tag: &str) -> PathBuf {
        let dir = std::env::temp_dir().join(format!(
            "comms-axi-wake-{tag}-{}-{:?}",
            std::process::id(),
            std::thread::current().id()
        ));
        let _ = std::fs::remove_dir_all(&dir);
        std::fs::create_dir_all(&dir).unwrap();
        dir
    }

    #[test]
    fn wake_decision_is_total() {
        // idle && mail → re-wake; every other combination → no-op.
        assert_eq!(wake_decision(true, true), WakeAction::ReWake);
        assert_eq!(wake_decision(true, false), WakeAction::NoOp);
        assert_eq!(wake_decision(false, true), WakeAction::NoOp);
        assert_eq!(wake_decision(false, false), WakeAction::NoOp);
    }

    #[test]
    fn idle_with_mail_re_wakes() {
        let dir = tmpdir("wake-mail");
        send_to(&dir, "coordinator", "dev-1", "done").unwrap();
        let action = idle_tick(&dir, "coordinator", true);
        assert_eq!(action, WakeAction::ReWake);
    }

    #[test]
    fn idle_with_empty_queue_is_silent_no_op() {
        let dir = tmpdir("wake-empty");
        // No publish happened: the marker is absent → no-op (zero tokens).
        let action = idle_tick(&dir, "coordinator", true);
        assert_eq!(action, WakeAction::NoOp);
    }

    #[test]
    fn non_idle_never_wakes_even_with_mail() {
        let dir = tmpdir("wake-nonidle");
        send_to(&dir, "coordinator", "dev-1", "done").unwrap();
        // A producer's message must not interrupt an active turn: not idle → no-op.
        let action = idle_tick(&dir, "coordinator", false);
        assert_eq!(action, WakeAction::NoOp);
    }

    #[test]
    fn idle_tick_is_file_stat_only_never_parses_queue() {
        // The zero-token guarantee, proven: idle_tick stats the MARKER, never the queue record.
        // A CORRUPT queue with a present marker must still re-wake (it never touches the queue);
        // a corrupt queue with NO marker must still no-op (again, no parse — no parse error).
        let dir = tmpdir("wake-stat-only");
        let q = crate::send::queue_path(&dir, "coordinator");
        std::fs::create_dir_all(q.parent().unwrap()).unwrap();
        std::fs::write(&q, "this is not json").unwrap();

        // No marker → NoOp, and crucially NOT a parse error.
        assert_eq!(idle_tick(&dir, "coordinator", true), WakeAction::NoOp);

        // Marker present → ReWake, still without parsing the corrupt queue.
        std::fs::write(crate::send::has_mail_path(&dir, "coordinator"), b"").unwrap();
        assert_eq!(idle_tick(&dir, "coordinator", true), WakeAction::ReWake);
    }

    #[test]
    fn wake_action_serializes() {
        assert_eq!(
            serde_json::to_value(WakeAction::ReWake).unwrap(),
            serde_json::json!("re-wake")
        );
        assert_eq!(
            serde_json::to_value(WakeAction::NoOp).unwrap(),
            serde_json::json!("no-op")
        );
    }
}
