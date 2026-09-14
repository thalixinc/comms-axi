//! S2 — `comms read`/`drain`: pull a station's queue on demand.
//!
//! The pull half of the publish/pull pair (S1 `send` publishes; S2 `read` pulls). Dequeues the
//! recipient station's pending events, clears `hasMail` when the queue empties, and renders the
//! drained events as a single injectable turn. Idempotent + bounded + lossless: an empty queue is
//! a no-op; a bounded read drains at most [`DRAIN_BOUND`](crate::scheduler::DRAIN_BOUND) events and
//! leaves the rest (no loss); the journal's `admit` dedup guarantees each event is enacted exactly
//! once. The queue record and `hasMail` marker are the S1 contract
//! (`<state-dir>/queues/<role>.json` + `<state-dir>/queues/<role>.hasmail`).

use std::path::Path;

use serde::Serialize;

use crate::error::Result;
use crate::scheduler::DRAIN_BOUND;
use crate::send::{clear_has_mail, load_queue, save_queue};

/// One drained event, rendered for the recipient's turn: the sender and the message body. The
/// envelope's `effect_id`/`kind`/`class` are queue/journal internals — the turn needs only who
/// sent it and what they said.
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub struct ReadEvent {
    pub from: String,
    pub text: String,
}

/// The read/drain result: the events drained this turn (causal order) plus what remains queued.
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub struct ReadResult {
    pub role: String,
    pub events: Vec<ReadEvent>,
    pub remaining: usize,
    pub has_mail: bool,
}

/// S2 — pull the station's queue on demand. Dequeues the pending envelopes (all of them when
/// `all`, else at most [`DRAIN_BOUND`]), admitting each in the journal, then clears `hasMail` when
/// the queue empties. Idempotent: an empty queue is a no-op (zero events; a stale marker is
/// repaired defensively). Lossless: a bounded read drains only the bound and leaves the rest, and
/// the marker stays set so the wake re-fires.
pub fn read(state_dir: &Path, role: &str, all: bool) -> Result<ReadResult> {
    let limit = if all { usize::MAX } else { DRAIN_BOUND };
    let mut store = load_queue(state_dir, role)?;

    if store.is_empty() {
        // No-op: nothing pending. Repair a stale marker defensively (empty ⟺ no marker).
        clear_has_mail(state_dir, role)?;
        return Ok(ReadResult {
            role: role.to_string(),
            events: Vec::new(),
            remaining: 0,
            has_mail: false,
        });
    }

    let drained = store.drain(limit);
    save_queue(state_dir, role, &store)?;

    let remaining = store.len();
    let has_mail = remaining > 0;
    if has_mail {
        // Partial drain: more pending — the marker stays (the wake re-fires next idle tick).
    } else {
        clear_has_mail(state_dir, role)?;
    }

    let events = drained
        .into_iter()
        .map(|e| ReadEvent {
            from: e.source_id,
            text: e.payload,
        })
        .collect();

    Ok(ReadResult {
        role: role.to_string(),
        events,
        remaining,
        has_mail,
    })
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::send::{has_mail, send_to};

    fn tmpdir(tag: &str) -> std::path::PathBuf {
        let dir = std::env::temp_dir().join(format!(
            "comms-axi-read-{tag}-{}-{:?}",
            std::process::id(),
            std::thread::current().id()
        ));
        let _ = std::fs::remove_dir_all(&dir);
        std::fs::create_dir_all(&dir).unwrap();
        dir
    }

    #[test]
    fn read_drains_all_and_clears_has_mail() {
        let dir = tmpdir("drains-all");
        send_to(&dir, "coordinator", "dev-1", "one").unwrap();
        send_to(&dir, "coordinator", "dev-2", "two").unwrap();
        assert!(has_mail(&dir, "coordinator"));

        let r = read(&dir, "coordinator", true).unwrap();
        assert_eq!(r.events.len(), 2);
        assert_eq!(r.events[0].from, "dev-1");
        assert_eq!(r.events[0].text, "one");
        assert_eq!(r.events[1].from, "dev-2");
        assert_eq!(r.events[1].text, "two");
        assert_eq!(r.remaining, 0);
        assert!(!r.has_mail);
        assert!(!has_mail(&dir, "coordinator"));
    }

    #[test]
    fn read_is_bounded_default() {
        // DRAIN_BOUND is 20; a bounded read drains at most 20 and leaves the rest (lossless).
        let dir = tmpdir("bounded");
        for i in 0..(DRAIN_BOUND + 5) {
            send_to(&dir, "coordinator", "dev-1", &format!("m{i}")).unwrap();
        }
        let r = read(&dir, "coordinator", false).unwrap();
        assert_eq!(r.events.len(), DRAIN_BOUND);
        assert_eq!(r.remaining, 5);
        assert!(r.has_mail, "a partial drain keeps the marker");
        assert!(has_mail(&dir, "coordinator"));
    }

    #[test]
    fn read_empty_is_no_op() {
        let dir = tmpdir("empty");
        let r = read(&dir, "coordinator", true).unwrap();
        assert_eq!(r.events.len(), 0);
        assert_eq!(r.remaining, 0);
        assert!(!r.has_mail);
    }

    #[test]
    fn read_drains_exactly_once() {
        // The journal admit dedup: after a full drain, a second read yields nothing.
        let dir = tmpdir("once");
        send_to(&dir, "coordinator", "dev-1", "only once").unwrap();
        let first = read(&dir, "coordinator", true).unwrap();
        assert_eq!(first.events.len(), 1);
        let second = read(&dir, "coordinator", true).unwrap();
        assert_eq!(second.events.len(), 0);
    }

    #[test]
    fn read_is_per_station_no_cross_reads() {
        let dir = tmpdir("per-station");
        send_to(&dir, "coordinator", "dev-1", "a").unwrap();
        send_to(&dir, "planner", "dev-2", "b").unwrap();
        let r = read(&dir, "coordinator", true).unwrap();
        assert_eq!(r.events.len(), 1);
        assert_eq!(r.events[0].text, "a");
        // planner's queue is untouched by reading coordinator's.
        assert!(has_mail(&dir, "planner"));
        assert_eq!(read(&dir, "planner", true).unwrap().events.len(), 1);
    }

    #[test]
    fn read_admits_in_the_journal() {
        // A drained event is journaled Admitted (exposed), never left Prepared.
        let dir = tmpdir("admit");
        send_to(&dir, "coordinator", "dev-1", "x").unwrap();
        let _ = read(&dir, "coordinator", true).unwrap();
        let store = load_queue(&dir, "coordinator").unwrap();
        let admitted = store.journal.ids_in(crate::journal::EffectState::Admitted);
        let prepared = store.journal.ids_in(crate::journal::EffectState::Prepared);
        assert_eq!(admitted.len(), 1);
        assert_eq!(prepared.len(), 0);
    }

    #[test]
    fn read_result_serializes() {
        let dir = tmpdir("serialize");
        send_to(&dir, "coordinator", "dev-1", "hi").unwrap();
        let r = read(&dir, "coordinator", true).unwrap();
        let v = serde_json::to_value(&r).unwrap();
        assert_eq!(v["role"], "coordinator");
        assert_eq!(v["events"][0]["from"], "dev-1");
        assert_eq!(v["events"][0]["text"], "hi");
        assert_eq!(v["remaining"], 0);
        assert_eq!(v["has_mail"], false);
    }
}
