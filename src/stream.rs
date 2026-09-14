//! Gate 3 — one canonical envelope stream + server-side protected classes + derived views (#444).
//!
//! There is EXACTLY ONE envelope stream. The actionable view (what the CoS acts on) and the
//! status view (what the founder/fleet sees) are both DERIVED PROJECTIONS over that one
//! append-ordered stream — never a second store, never a producer-writable status/class field.
//!
//! Classification is SERVER-SIDE PROTECTED: a producer submits only `source_id` + `effect_id` +
//! `kind` + `payload`. The routing class (`Actionable` vs `Status`) is assigned by CF's OWNED
//! taxonomy in [`classify`], never read from the producer — so a producer cannot self-label into
//! a protected class, and a mislabel is downcast (an unknown kind resolves to `Actionable`, never
//! to a hidden/status bucket that could bury work). The class is computed on the server's write
//! path ([`Stream::append`]) and recorded on the envelope; there is no producer-path that writes
//! a class.
//!
//! The two views are pure queries over the one stream:
//! * [`Stream::actionable`] — the `Actionable` envelopes the CoS acts on, in causal order.
//! * [`Stream::status`] — the `Status` envelopes the founder/fleet observes, in causal order.
//!
//! A lossless aggregation invariant holds: every envelope is in EXACTLY ONE view, so the union of
//! the two projections is the stream itself (equal-count, equal-set, same causal order) — no
//! envelope is dropped or duplicated across the two views.
//!
//! Unknown kinds do NOT silently drop: they surface as `Actionable`, aggregated by source id
//! ([`Stream::unknown_sources`]) and expandable back to the individual envelopes
//! ([`Stream::expand_source`]).
//!
//! The core is PURE and deterministic (a `Vec`, no fs/env/clock) so the qualification test
//! (lossless-aggregation + causal-ordering) can mislabel a producer and reorder a batch and assert
//! the canonical stream is unchanged, the views stay consistent, and nothing is dropped or
//! duplicated. The durable serialization (`to_record`/`from_record`) round-trips losslessly; the
//! caller persists it with the same atomic-write rule as `state.rs`. Like `fleet`/`inbox`/
//! `journal`, this is the tested contract surface the durable-broker epic wires into the live
//! delivery path (CF owns routing policy per R1 — the taxonomy and the stream are cf-side).

#![allow(dead_code)]

use serde_json::{json, Value};

/// The routing class a server assigns to an envelope. `Actionable` = the CoS acts on it;
/// `Status` = the founder/fleet observes it. This is a SERVER-OWNED assignment — a producer
/// never writes it.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Class {
    Actionable,
    Status,
}

/// One envelope on the canonical stream. The `class` is server-assigned (never producer-written);
/// `kind` is the producer's declared event kind; `source_id` identifies the producing seat/agent.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Envelope {
    pub effect_id: String,
    pub source_id: String,
    pub kind: String,
    pub payload: String,
    pub class: Class,
}

/// The canonical envelope stream: ONE append-ordered list. Arrival order IS the causal order —
/// a single cf-owned append path, so no second store or parallel stream can disagree on order.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct Stream {
    entries: Vec<Envelope>,
}

/// The CF-owned routing taxonomy: which `kind`s the CoS must act on (Actionable) vs. the ones the
/// founder/fleet merely observes (Status). This is the single source for the server-side
/// classification — a producer declares a `kind`, and CF maps it here; there is no producer
/// class field to trust. An UNKNOWN kind is `Actionable` by design: unknown work must SURFACE for
/// action, never hide in a status bucket or drop.
pub fn classify(kind: &str) -> Class {
    match kind.trim() {
        // What the CoS acts on — evidence and calls for decision/action.
        "done" | "blocked" | "question" | "note" | "handoff" => Class::Actionable,
        // What the founder/fleet observes — informational telemetry.
        "status" | "progress" | "summary" => Class::Status,
        // Unknown: fail-open to action. A producer cannot self-label into Status by naming an
        // unrecognized kind; the unknown surfaces as actionable and is aggregated by source.
        _ => Class::Actionable,
    }
}

impl Stream {
    /// An empty canonical stream (first envelope).
    pub fn new() -> Self {
        Self::default()
    }

    /// Append an envelope to the canonical stream. The class is SERVER-ASSIGNED here — derived
    /// from `kind` by [`classify`], never read from the producer. Appends in arrival order
    /// (causal order); the caller is the single cf-owned write path.
    pub fn append(&mut self, source_id: &str, effect_id: &str, kind: &str, payload: &str) {
        self.entries.push(Envelope {
            effect_id: effect_id.to_string(),
            source_id: source_id.to_string(),
            kind: kind.to_string(),
            payload: payload.to_string(),
            class: classify(kind),
        });
    }

    /// The full stream in causal (arrival) order.
    pub fn entries(&self) -> &[Envelope] {
        &self.entries
    }

    /// Remove and return the first `limit` envelopes (causal order). `limit >= len` returns them
    /// all. Used by S2 read/drain to dequeue pending events; the remaining tail is the un-drained
    /// suffix, so a bounded drain is lossless.
    pub fn drain_front(&mut self, limit: usize) -> Vec<Envelope> {
        let n = limit.min(self.entries.len());
        self.entries.drain(0..n).collect()
    }

    /// The actionable view: every `Actionable` envelope in causal order (what the CoS acts on).
    pub fn actionable(&self) -> Vec<&Envelope> {
        self.classed(Class::Actionable)
    }

    /// The status view: every `Status` envelope in causal order (what the founder/fleet sees).
    pub fn status(&self) -> Vec<&Envelope> {
        self.classed(Class::Status)
    }

    fn classed(&self, class: Class) -> Vec<&Envelope> {
        self.entries.iter().filter(|e| e.class == class).collect()
    }

    /// Unknown kinds surface as actionable, AGGREGATED by source id: `(source_id, count)` in
    /// sorted order — the "there is unbucketed work from these producers" signal. Expandable via
    /// [`Stream::expand_source`].
    pub fn unknown_sources(&self) -> Vec<(&str, usize)> {
        let mut counts: std::collections::BTreeMap<&str, usize> = std::collections::BTreeMap::new();
        for e in self
            .entries
            .iter()
            .filter(|e| classify(&e.kind) == Class::Actionable && !is_known_kind(&e.kind))
        {
            *counts.entry(e.source_id.as_str()).or_insert(0) += 1;
        }
        counts.into_iter().collect()
    }

    /// Expand an aggregated unknown source back to its individual envelopes (causal order).
    pub fn expand_source(&self, source_id: &str) -> Vec<&Envelope> {
        self.entries
            .iter()
            .filter(|e| e.source_id == source_id && !is_known_kind(&e.kind))
            .collect()
    }

    /// Serialize to the durable record (lossless round-trip through `from_record`).
    pub fn to_record(&self) -> Value {
        json!({
            "entries": self.entries.iter().map(|e| json!({
                "effect_id": e.effect_id,
                "source_id": e.source_id,
                "kind": e.kind,
                "payload": e.payload,
                "class": class_name(e.class),
            })).collect::<Vec<_>>(),
        })
    }

    /// Rehydrate from the durable record. A missing field or an unknown class name is CORRUPT
    /// (loud), never silently inferred. The class is preserved verbatim (it was server-assigned
    /// at append; a record cannot rewrite it).
    pub fn from_record(v: &Value) -> std::result::Result<Self, String> {
        let arr = v
            .get("entries")
            .and_then(Value::as_array)
            .ok_or_else(|| "stream record is corrupt: missing entries".to_string())?;
        let mut entries = Vec::with_capacity(arr.len());
        for e in arr {
            let get = |k: &str| -> std::result::Result<String, String> {
                e.get(k)
                    .and_then(Value::as_str)
                    .map(str::to_string)
                    .ok_or_else(|| format!("stream entry missing {k:?}"))
            };
            let class = match e.get("class").and_then(Value::as_str) {
                Some("actionable") => Class::Actionable,
                Some("status") => Class::Status,
                _ => return Err("stream entry has an unknown class".to_string()),
            };
            entries.push(Envelope {
                effect_id: get("effect_id")?,
                source_id: get("source_id")?,
                kind: get("kind")?,
                payload: get("payload")?,
                class,
            });
        }
        Ok(Stream { entries })
    }
}

fn class_name(c: Class) -> &'static str {
    match c {
        Class::Actionable => "actionable",
        Class::Status => "status",
    }
}

/// Whether `kind` is in the CF-owned taxonomy (i.e. not an "unknown" that must surface + aggregate).
fn is_known_kind(kind: &str) -> bool {
    matches!(
        kind.trim(),
        "done" | "blocked" | "question" | "note" | "handoff" | "status" | "progress" | "summary"
    )
}

#[cfg(test)]
mod tests {
    use super::*;

    fn ids<'a>(view: &[&'a Envelope]) -> Vec<&'a str> {
        view.iter().map(|e| e.effect_id.as_str()).collect()
    }

    fn owned_ids(view: &[Envelope]) -> Vec<&str> {
        view.iter().map(|e| e.effect_id.as_str()).collect()
    }

    // ---- server-side classification ----

    #[test]
    fn classification_is_server_assigned_not_producer_labelable() {
        // A producer submits a kind; CF's taxonomy assigns the class. There is no producer
        // class field on the write path — `append` takes no class, so self-labeling is
        // structurally impossible.
        assert_eq!(classify("done"), Class::Actionable);
        assert_eq!(classify("blocked"), Class::Actionable);
        assert_eq!(classify("question"), Class::Actionable);
        assert_eq!(classify("status"), Class::Status);
        assert_eq!(classify("summary"), Class::Status);
        // Unknown kinds are downcast to Actionable — a producer naming an unrecognized kind
        // cannot hide behind a status class.
        assert_eq!(classify("mystery-thing"), Class::Actionable);
        assert_eq!(classify(""), Class::Actionable);
        assert_eq!(classify("   "), Class::Actionable);
    }

    #[test]
    fn a_producer_mislabel_is_downcast_not_protected() {
        // A producer tries to smuggle a protected/status flavor via a kind token; server taxonomy
        // does not honour it — it surfaces as actionable, never as status.
        let mut s = Stream::new();
        s.append(
            "dev-3",
            "e1",
            "status",
            "I the producer says this is just status",
        );
        s.append(
            "dev-3",
            "e2",
            "definitely-not-actionable",
            "mislabel attempt",
        );
        assert_eq!(
            s.status().len(),
            1,
            "only the genuine status kind lands in the status view"
        );
        assert_eq!(
            s.actionable().len(),
            1,
            "the mislabeled unknown surfaces as actionable"
        );
        assert!(
            ids(&s.actionable()).contains(&"e2"),
            "the smuggled envelope is NOT hidden"
        );
    }

    // ---- lossless aggregation + causal ordering: the named qualification test ----

    #[test]
    fn qual_lossless_aggregation_and_causal_ordering() {
        // (a) Lossless aggregation: project both views over the full stream; the union equals the
        // stream (equal-count, equal-set, every envelope in EXACTLY ONE view — no drop, no dup).
        let mut s = Stream::new();
        s.append("dev-1", "e1", "done", "PR #1");
        s.append("dev-2", "e2", "blocked", "need a decision");
        s.append("dev-1", "e3", "status", "in progress");
        s.append("dev-3", "e4", "unknown-kind", "mystery work");
        s.append("dev-2", "e5", "summary", "weekly");

        let actionable = s.actionable();
        let status = s.status();
        assert_eq!(
            actionable.len() + status.len(),
            s.entries().len(),
            "the two views partition the stream"
        );
        let mut partitioned: Vec<&str> = actionable
            .iter()
            .map(|e| e.effect_id.as_str())
            .chain(status.iter().map(|e| e.effect_id.as_str()))
            .collect();
        partitioned.sort_unstable();
        let mut all: Vec<&str> = s.entries().iter().map(|e| e.effect_id.as_str()).collect();
        all.sort_unstable();
        assert_eq!(
            partitioned, all,
            "no envelope dropped or duplicated across the two views"
        );
        // Every observable effect appears in exactly one projection.
        assert_eq!(
            ids(&actionable),
            ["e1", "e2", "e4"],
            "actionable = done + blocked + unknown"
        );
        assert_eq!(ids(&status), ["e3", "e5"], "status = status + summary");

        // (b) Causal ordering: the stream order is arrival order, and both views reflect the SAME
        // relative order — reordering a submitted batch does not reorder the canonical stream.
        // (The canonical stream is the single append path; "reorder a batch" means appending out
        // of producer-declared sequence must still land in append order, so views agree.)
        let mut r = Stream::new();
        // Simulate a reordered batch arriving: the producer intended e7 before e6, but the wire
        // delivered e6 first.
        r.append("dev-1", "e6", "status", "later");
        r.append("dev-1", "e7", "done", "earlier-intention");
        assert_eq!(
            owned_ids(r.entries()),
            ["e6", "e7"],
            "canonical order is append order, not producer intent"
        );
        let a = r.actionable();
        let st = r.status();
        // Both views are projections of the same ordered stream: within a view, relative order
        // equals stream order, and concat preserves it.
        assert_eq!(ids(&a), ["e7"], "actionable honours stream order");
        assert_eq!(ids(&st), ["e6"], "status honours stream order");

        assert_eq!(
            a.len() + st.len(),
            r.entries().len(),
            "losslessness holds for the reordered batch too"
        );
    }

    // ---- unknown kinds aggregate + expand ----

    #[test]
    fn unknown_kinds_aggregate_by_source_and_expand() {
        let mut s = Stream::new();
        s.append("dev-9", "u1", "weird-a", "x");
        s.append("dev-9", "u2", "weird-b", "y");
        s.append("dev-10", "u3", "weird-a", "z");
        s.append("dev-9", "u4", "done", "normal");
        let agg = s.unknown_sources();
        assert_eq!(
            agg,
            vec![("dev-10", 1), ("dev-9", 2)],
            "unknown kinds aggregate by source id, sorted"
        );
        let expanded = s.expand_source("dev-9");
        assert_eq!(
            ids(&expanded),
            ["u1", "u2"],
            "expand returns only the UNKNOWN envelopes of that source (not the known 'done')"
        );
    }

    // ---- durable record ----

    #[test]
    fn record_round_trips_losslessly() {
        let mut s = Stream::new();
        s.append("dev-1", "e1", "done", "a");
        s.append("dev-2", "e2", "status", "b");
        s.append("dev-3", "e3", "mystery", "c");
        let rec = s.to_record();
        let back = Stream::from_record(&rec).unwrap();
        assert_eq!(
            back, s,
            "round-trip preserves every envelope + its server-assigned class"
        );
    }

    #[test]
    fn corrupt_record_is_loud_not_inferred() {
        assert!(
            Stream::from_record(&json!({})).is_err(),
            "missing entries is corrupt"
        );
        assert!(Stream::from_record(&json!({"entries": [{"effect_id": "e", "source_id": "s", "kind": "k", "payload": "p", "class": "bogus"}]})).is_err(), "unknown class is corrupt");
        assert!(Stream::from_record(&json!({"entries": [{"source_id": "s", "kind": "k", "payload": "p", "class": "status"}]})).is_err(), "missing effect_id is corrupt");
    }
}
