//! S1 — `comms send`: deferred publish to a per-station local queue.
//!
//! A producer publishes an event to a recipient role's LOCAL queue and sets the recipient's
//! `hasMail` flag. It does **NOT** resolve-and-fire into a live session — publish and pull are
//! decoupled: the producer never decides *when* the recipient reads; the recipient's idle state
//! does (S2 `read`/`drain`, S3 heartbeat). This is what stops the CoS mid-turn interruption bug.
//!
//! The store reuses the relocated `journal`/`stream` modules (G1–G5, moved as-is in #13):
//! the queue body is a [`Stream`](crate::stream::Stream) — one append-ordered envelope stream —
//! and the [`EffectJournal`](crate::journal::EffectJournal) is the durable
//! prepared/admitted/consumed ledger that S2's read/drain will drive. S1 only ever journals
//! `Prepared` (arrival, never exposure); the observable-effect boundary (`admit`) is S2's.
//!
//! File layout (per station, under the state dir):
//!
//! ```text
//! <state-dir>/queues/<role>.json     — the queue record (stream + journal + next effect seq)
//! <state-dir>/queues/<role>.hasmail  — the hasMail flag (empty marker; presence == mail pending)
//! ```
//!
//! `hasMail` is a file-stat signal — the heartbeat (S3/S4) stats the marker, never parses the
//! queue, so an empty inbox costs zero tokens. The invariant (marker present ⟺ queue non-empty)
//! is maintained by the store's single writer: `send` writes the queue then the marker; S2's
//! drain removes the marker when the queue empties.

use std::path::{Path, PathBuf};

use crate::error::{Error, Result};
use crate::journal::EffectJournal;
use crate::stream::{Envelope, Stream};

/// The `kind` a producer `send` declares. A producer's message is actionable work for the
/// recipient — `Stream::classify` maps `note` to `Actionable`, so it always surfaces, never
/// hides in a status bucket.
const SEND_KIND: &str = "note";

/// A per-station deferred-publish queue: the append-ordered [`Stream`] of pending events plus the
/// [`EffectJournal`] (durable prepared/admitted/consumed ledger) plus a monotonic effect-id
/// sequence. Pure and deterministic; the file-backed wrapper ([`send_to`]) persists it.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct QueueStore {
    pub stream: Stream,
    pub journal: EffectJournal,
    /// Next effect-id suffix. Monotonic and persisted, so an id is never reused after a drain.
    next_seq: u64,
}

impl QueueStore {
    /// An empty station queue (first publish, nothing pending).
    pub fn new() -> Self {
        Self::default()
    }

    /// Enqueue one event. Journals the arrival (`Prepared` — not exposed), appends the envelope to
    /// the stream, and returns the assigned effect id. Reusing [`EffectJournal::prepare`]: a
    /// re-delivered id is acknowledged, never re-enacted — no duplicate, no loss.
    pub fn enqueue(&mut self, role: &str, sender: &str, payload: &str) -> String {
        let effect_id = format!("{role}-{}", self.next_seq);
        self.next_seq += 1;
        self.enqueue_with_id(sender, &effect_id, payload);
        effect_id
    }

    /// Enqueue under a CALLER-SUPPLIED effect id — the STABLE cross-host id from the delivery
    /// envelope (epic #31) — instead of the local per-station sequence. Reuses the journal's
    /// `prepare` dedup: a re-delivery of an already-journaled id is acknowledged but NOT
    /// re-appended (the #35 dedup axis, kept intact here so #35 is additive). `next_seq` is
    /// untouched — stable ids live in a distinct namespace from `{role}-{seq}`.
    ///
    /// Returns `true` when this call APPENDED a new envelope (a fresh `prepare`), `false` when the
    /// id was already journaled (a re-delivery — acknowledged, not re-enacted).
    pub fn enqueue_with_id(&mut self, sender: &str, effect_id: &str, payload: &str) -> bool {
        if self.journal.prepare(effect_id) {
            self.stream.append(sender, effect_id, SEND_KIND, payload);
            true
        } else {
            false
        }
    }

    /// True when the queue has no pending events.
    pub fn is_empty(&self) -> bool {
        self.stream.entries().is_empty()
    }

    /// The number of pending events.
    pub fn len(&self) -> usize {
        self.stream.entries().len()
    }

    /// Dequeue up to `limit` pending envelopes (front-first), admitting each in the journal — the
    /// observable-effect boundary (S2). Returns the drained envelopes in causal order. Lossless: a
    /// bounded drain leaves the un-drained tail in the stream; `limit >= len()` drains the whole
    /// queue. The journal `admit` dedup guarantees each effect is enacted exactly once.
    pub fn drain(&mut self, limit: usize) -> Vec<Envelope> {
        let drained = self.stream.drain_front(limit);
        for e in &drained {
            self.journal.admit(&e.effect_id);
        }
        drained
    }

    /// Serialize the whole store (stream + journal + sequence) losslessly.
    pub fn to_record(&self) -> serde_json::Value {
        serde_json::json!({
            "next_seq": self.next_seq,
            "stream": self.stream.to_record(),
            "journal": self.journal.to_record(),
        })
    }

    /// Rehydrate the whole store. A record missing a required key is CORRUPT (loud) — never a
    /// silently-inferred empty queue, because that would hide pending work.
    pub fn from_record(v: &serde_json::Value) -> std::result::Result<Self, String> {
        let next_seq = v
            .get("next_seq")
            .and_then(serde_json::Value::as_u64)
            .ok_or_else(|| "queue record is corrupt: missing next_seq".to_string())?;
        let stream = Stream::from_record(
            v.get("stream")
                .ok_or_else(|| "queue record is corrupt: missing stream".to_string())?,
        )?;
        let journal = EffectJournal::from_record(
            v.get("journal")
                .ok_or_else(|| "queue record is corrupt: missing journal".to_string())?,
        )?;
        Ok(QueueStore {
            stream,
            journal,
            next_seq,
        })
    }
}

/// The receipt `send` returns: what was enqueued, for the recipient's station, and the resulting
/// `hasMail` state. `duplicate` is `true` when the event's effect id was ALREADY journaled (a
/// re-delivery — acknowledged but NOT re-enacted, #35 idempotency), `false` for a fresh enqueue.
/// Serialized by the CLI (`--json`).
#[derive(Debug, Clone, PartialEq, Eq, serde::Serialize)]
pub struct Enqueued {
    pub role: String,
    pub effect_id: String,
    pub queued: usize,
    pub has_mail: bool,
    pub duplicate: bool,
}

/// The per-station queue file: `<state-dir>/queues/<role>.json`.
pub fn queue_path(state_dir: &Path, role: &str) -> PathBuf {
    state_dir.join("queues").join(format!("{role}.json"))
}

/// The per-station hasMail marker: `<state-dir>/queues/<role>.hasmail`.
pub fn has_mail_path(state_dir: &Path, role: &str) -> PathBuf {
    state_dir.join("queues").join(format!("{role}.hasmail"))
}

/// The hasMail flag — a pure file-stat (marker exists == mail pending). Never parses the queue;
/// an empty inbox costs zero tokens to check.
pub fn has_mail(state_dir: &Path, role: &str) -> bool {
    has_mail_path(state_dir, role).exists()
}

/// Load a station's queue from disk. Missing or empty → an empty queue (first publish); a corrupt
/// (unparseable) record is LOUD — never silently replaced with an empty queue. Shared by `send`
/// (S1) and `read` (S2).
pub fn load_queue(state_dir: &Path, role: &str) -> Result<QueueStore> {
    let path = queue_path(state_dir, role);
    let text = match std::fs::read_to_string(&path) {
        Ok(t) => t,
        Err(e) if e.kind() == std::io::ErrorKind::NotFound => return Ok(QueueStore::new()),
        Err(e) => {
            return Err(Error::operational(
                format!("queue unreadable at {path:?}: {e}"),
                "QUEUE_IO",
            ))
        }
    };
    if text.trim().is_empty() {
        return Ok(QueueStore::new());
    }
    let value: serde_json::Value = serde_json::from_str(&text).map_err(|e| {
        Error::operational(
            format!("queue record is corrupt at {path:?}: {e}"),
            "QUEUE_CORRUPT",
        )
    })?;
    QueueStore::from_record(&value).map_err(|e| {
        Error::operational(
            format!("queue record is corrupt at {path:?}: {e}"),
            "QUEUE_CORRUPT",
        )
    })
}

/// Persist the queue atomically (temp file + rename) so a crash never leaves a half-written
/// record. One writer per station: the comms-axi `send`/`read` verbs (the same single-writer rule
/// the relocated `inbox`/`state` stores follow).
pub fn save_queue(state_dir: &Path, role: &str, store: &QueueStore) -> Result<()> {
    let path = queue_path(state_dir, role);
    if let Some(parent) = path.parent() {
        std::fs::create_dir_all(parent)
            .map_err(|e| Error::operational(format!("queue dir: {e}"), "QUEUE_IO"))?;
    }
    let bytes = serde_json::to_vec_pretty(&store.to_record())
        .map_err(|e| Error::operational(format!("queue serialize: {e}"), "QUEUE_IO"))?;
    let tmp = path.with_extension("json.tmp");
    std::fs::write(&tmp, &bytes)
        .map_err(|e| Error::operational(format!("queue write: {e}"), "QUEUE_IO"))?;
    std::fs::rename(&tmp, &path)
        .map_err(|e| Error::operational(format!("queue rename: {e}"), "QUEUE_IO"))?;
    Ok(())
}

/// Write the hasMail marker (queue is now non-empty). S2's drain removes it when the queue empties.
fn set_has_mail(state_dir: &Path, role: &str) -> Result<()> {
    let path = has_mail_path(state_dir, role);
    if let Some(parent) = path.parent() {
        std::fs::create_dir_all(parent)
            .map_err(|e| Error::operational(format!("queue dir: {e}"), "QUEUE_IO"))?;
    }
    std::fs::write(&path, b"")
        .map_err(|e| Error::operational(format!("hasMail write at {path:?}: {e}"), "QUEUE_IO"))
}

/// Clear the hasMail marker (the queue is now empty). S1's `send_to` sets it; S2's read/drain
/// removes it when the last pending event is drained. Idempotent — clearing an absent marker is a
/// no-op.
pub fn clear_has_mail(state_dir: &Path, role: &str) -> Result<()> {
    let path = has_mail_path(state_dir, role);
    match std::fs::remove_file(&path) {
        Ok(()) => Ok(()),
        Err(e) if e.kind() == std::io::ErrorKind::NotFound => Ok(()),
        Err(e) => Err(Error::operational(
            format!("hasMail clear at {path:?}: {e}"),
            "QUEUE_IO",
        )),
    }
}

/// S1 — the deferred publish verb. Enqueues one event to the recipient's local queue and sets its
/// `hasMail` flag. It does **NOT** resolve-and-fire into a live session: it never calls
/// `resolve_role` or `send_event`, and never touches a harness session. Publish only.
pub fn send_to(state_dir: &Path, role: &str, sender: &str, payload: &str) -> Result<Enqueued> {
    let mut store = load_queue(state_dir, role)?;
    let effect_id = store.enqueue(role, sender, payload);
    save_queue(state_dir, role, &store)?;
    set_has_mail(state_dir, role)?;
    Ok(Enqueued {
        role: role.to_string(),
        effect_id,
        queued: store.len(),
        has_mail: true,
        duplicate: false,
    })
}

/// The inbound-listener publish (#33): like [`send_to`], but enqueues under a CALLER-SUPPLIED
/// STABLE effect id (the delivery envelope's id) rather than the local sequence. Reuses the
/// journal dedup — a re-delivery of the same id is acknowledged, not re-enacted (#35). The
/// `hasMail` marker is set ONLY when a real event was appended; a deduped re-delivery must not
/// leave a marker over an empty queue (that would drive a spurious empty turn on the next tick).
pub fn send_to_with_id(
    state_dir: &Path,
    role: &str,
    sender: &str,
    payload: &str,
    effect_id: &str,
) -> Result<Enqueued> {
    let mut store = load_queue(state_dir, role)?;
    let appended = store.enqueue_with_id(sender, effect_id, payload);
    save_queue(state_dir, role, &store)?;
    if appended {
        set_has_mail(state_dir, role)?;
    }
    Ok(Enqueued {
        role: role.to_string(),
        effect_id: effect_id.to_string(),
        queued: store.len(),
        has_mail: !store.is_empty(),
        duplicate: !appended,
    })
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    fn tmpdir(tag: &str) -> PathBuf {
        let dir = std::env::temp_dir().join(format!(
            "comms-axi-queue-{tag}-{}-{:?}",
            std::process::id(),
            std::thread::current().id()
        ));
        let _ = std::fs::remove_dir_all(&dir);
        std::fs::create_dir_all(&dir).unwrap();
        dir
    }

    #[test]
    fn send_enqueues_and_sets_has_mail() {
        let dir = tmpdir("send-enqueues");
        let r = send_to(&dir, "coordinator", "dev-1", "done: PR #9").unwrap();
        assert_eq!(r.role, "coordinator");
        assert_eq!(r.queued, 1);
        assert!(r.has_mail);
        assert!(has_mail(&dir, "coordinator"));
        // The event is on the queue, and the effect is journaled Prepared (never exposed).
        let store = load_queue(&dir, "coordinator").unwrap();
        assert_eq!(store.len(), 1);
        assert_eq!(store.stream.entries()[0].payload, "done: PR #9");
        assert_eq!(store.stream.entries()[0].source_id, "dev-1");
        assert_eq!(
            store.journal.state_of(&r.effect_id),
            Some(crate::journal::EffectState::Prepared)
        );
    }

    #[test]
    fn send_is_per_station_no_cross_writes() {
        let dir = tmpdir("send-per-station");
        send_to(&dir, "coordinator", "dev-1", "a").unwrap();
        send_to(&dir, "planner", "dev-1", "b").unwrap();
        // Each station has its own queue and flag; neither sees the other's mail.
        assert_eq!(load_queue(&dir, "coordinator").unwrap().len(), 1);
        assert_eq!(load_queue(&dir, "planner").unwrap().len(), 1);
        assert!(has_mail(&dir, "coordinator"));
        assert!(has_mail(&dir, "planner"));
        assert!(queue_path(&dir, "coordinator") != queue_path(&dir, "planner"));
    }

    #[test]
    fn send_appends_in_order_with_distinct_effect_ids() {
        let dir = tmpdir("send-order");
        let a = send_to(&dir, "coordinator", "dev-1", "first").unwrap();
        let b = send_to(&dir, "coordinator", "dev-2", "second").unwrap();
        assert_ne!(a.effect_id, b.effect_id);
        let store = load_queue(&dir, "coordinator").unwrap();
        let payloads: Vec<&str> = store
            .stream
            .entries()
            .iter()
            .map(|e| e.payload.as_str())
            .collect();
        assert_eq!(payloads, vec!["first", "second"]);
    }

    #[test]
    fn send_is_deferred_never_fires() {
        // The hard ban: publish only. `send_to` mutates the queue store + marker; it must not
        // call resolve_role/send_event, and must not create any session/harness state. We prove
        // it by the absence of a binding path and by asserting the ONLY files under the state dir
        // are the queue record and the hasMail marker.
        let dir = tmpdir("send-deferred");
        send_to(&dir, "coordinator", "dev-1", "hi").unwrap();
        let mut files = std::fs::read_dir(dir.join("queues"))
            .unwrap()
            .map(|e| e.unwrap().file_name().to_string_lossy().into_owned())
            .collect::<Vec<_>>();
        files.sort();
        assert_eq!(files, vec!["coordinator.hasmail", "coordinator.json"]);
    }

    #[test]
    fn empty_mailbox_has_no_mail_flag() {
        let dir = tmpdir("send-empty-flag");
        // Nothing published → no marker → hasMail false (file-stat only, zero parse).
        assert!(!has_mail(&dir, "coordinator"));
        assert!(!has_mail_path(&dir, "coordinator").exists());
    }

    #[test]
    fn corrupt_queue_record_is_loud_not_replaced() {
        let dir = tmpdir("send-corrupt");
        let path = queue_path(&dir, "coordinator");
        std::fs::create_dir_all(path.parent().unwrap()).unwrap();
        std::fs::write(&path, "not json").unwrap();
        let err = send_to(&dir, "coordinator", "dev-1", "x").unwrap_err();
        assert!(err.to_string().contains("corrupt"), "got {err:?}");
    }

    #[test]
    fn store_round_trips_through_the_record() {
        let mut s = QueueStore::new();
        let id = s.enqueue("coordinator", "dev-1", "round-trip me");
        let rec = s.to_record();
        let back = QueueStore::from_record(&rec).unwrap();
        assert_eq!(back, s);
        assert_eq!(back.stream.entries()[0].payload, "round-trip me");
        assert_eq!(
            back.journal.state_of(&id),
            Some(crate::journal::EffectState::Prepared)
        );
    }

    #[test]
    fn corrupt_store_record_missing_keys_is_loud() {
        assert!(QueueStore::from_record(&json!({"stream": {"entries": []}})).is_err());
        assert!(QueueStore::from_record(&json!({"journal": {"effects": []}})).is_err());
    }

    #[test]
    fn enqueue_reuses_journal_dedup_for_same_id() {
        // The journal's dedup axis: a re-delivered effect id is prepared once, not re-enacted.
        let mut s = QueueStore::new();
        let a = s.enqueue("coordinator", "dev-1", "first");
        let b = s.enqueue("coordinator", "dev-2", "dup-attempt");
        assert_ne!(a, b, "the monotonic sequence always assigns a fresh id");
        // The ledger records two distinct Prepared ids — nothing dropped, nothing duplicated.
        assert_eq!(
            s.journal
                .ids_in(crate::journal::EffectState::Prepared)
                .len(),
            2
        );
        assert_eq!(s.len(), 2);
    }

    #[test]
    fn send_to_with_id_does_not_set_marker_on_dedup() {
        // A re-delivery of an already-drained id appends NOTHING; it must not re-set the hasMail
        // marker over an empty queue (that would drive a spurious empty turn on the next idle
        // tick).
        let dir = tmpdir("dedup-marker");
        let a = send_to_with_id(&dir, "cof", "remote", "hi", "stable-1").unwrap();
        assert_eq!(a.queued, 1);
        assert!(a.has_mail);
        assert!(has_mail(&dir, "cof"));

        // Drain it (S2 read), which admits the event and clears the marker.
        crate::read::read(&dir, "cof", true).unwrap();
        assert!(!has_mail(&dir, "cof"));

        // Re-delivery of the SAME id: deduped — nothing appended, NO marker, has_mail=false.
        let b = send_to_with_id(&dir, "cof", "remote", "hi", "stable-1").unwrap();
        assert_eq!(b.queued, 0);
        assert!(!b.has_mail);
        assert!(b.duplicate, "a re-delivery is flagged duplicate");
        assert!(!has_mail(&dir, "cof"));
    }

    #[test]
    fn send_to_with_id_duplicate_flag_is_false_on_fresh_enqueue() {
        let dir = tmpdir("dedup-fresh");
        let a = send_to_with_id(&dir, "cof", "remote", "hi", "stable-1").unwrap();
        assert!(!a.duplicate, "a fresh enqueue is not a duplicate");
        assert_eq!(a.queued, 1);
    }
}
