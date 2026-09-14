//! Gate 2 — prepared/admitted/consumed journaling + ambiguous-effect reconciliation (#443).
//!
//! A crash after context exposure but before ack must not DUPLICATE an observable effect. This
//! module models the three durable states a delivered envelope passes through — `prepared` →
//! `admitted` → `consumed` — keyed by a STABLE effect id, plus the harness-journal reconciliation
//! that recovers a crash between exposure and ack.
//!
//! The load-bearing rule (from #440 option 13): this is ACKNOWLEDGED-DURABLE delivery with
//! EFFECT-LEVEL DEDUP — a re-delivered envelope whose effect id is already journaled is
//! acknowledged but NOT re-enacted. It is NEVER a promise of exactly-once: a crash *after*
//! exposure but *before* ack is reconciled by dedup (no re-enactment, no loss), but the residual
//! (e.g. crash before exposure-with-durable-journal-write) is named honestly, not hidden.
//!
//! The core ([`EffectJournal`] + its transitions) is PURE and deterministic (a `HashMap`, no
//! fs/env/clock) so the qualification test can fault-inject a crash at EACH boundary —
//! after-prepared, after-admitted-before-ack, after-consumed — and assert no duplicated
//! observable effect and no lost envelope, purely from the journal state. The durable
//! serialization (`to_record`/`from_record`) round-trips it losslessly; the caller persists it
//! with the same atomic-write rule as `state.rs`.
//!
//! Like `fleet`/`native`/`workspace`, this is the tested contract surface the durable-broker
//! epic wires into the live delivery path (the Abridge durable store — Gate 2's three-owner
//! boundary keeps the journaling model cf-side, the broker Abridge-side). Until then it is not
//! dead weight: the transitions + reconciliation are the named Gate 2 qualification.

#![allow(dead_code)]

use serde_json::{json, Value};
use std::collections::HashMap;

/// The three durable states a delivered envelope passes through. `Prepared` = journaled on
/// arrival (not yet exposed); `Admitted` = exposed to the CoS context (the observable effect
/// was enacted); `Consumed` = acknowledged (the enactment is recorded, never re-enacted).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum EffectState {
    Prepared,
    Admitted,
    Consumed,
}

/// The effect journal: the single durable record of every observed effect id and its state.
/// Keyed by effect id — the ONE canonical dedup axis. A re-delivered envelope whose effect id
/// is already here is acknowledged-but-not-re-enacted (see [`EffectJournal::admit`]).
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct EffectJournal {
    effects: HashMap<String, EffectState>,
}

impl EffectJournal {
    /// An empty journal (first delivery, nothing journaled yet).
    pub fn new() -> Self {
        Self::default()
    }

    /// The durable state of an effect id, if journaled.
    pub fn state_of(&self, effect_id: &str) -> Option<EffectState> {
        self.effects.get(effect_id).copied()
    }

    /// `prepare` — an envelope ARRIVED and is journaled as `Prepared`. Arrival itself is not an
    /// observable effect; nothing is enacted. Idempotent: a re-delivered (same id) envelope that
    /// is already `Prepared`/`Admitted`/`Consumed` is NOT re-prepared (no state downgrade, no
    /// loss). Returns `true` when this call newly recorded the id, `false` when it was already
    /// journaled (a re-push — acknowledged, not re-enacted).
    pub fn prepare(&mut self, effect_id: &str) -> bool {
        use std::collections::hash_map::Entry as HE;
        match self.effects.entry(effect_id.to_string()) {
            HE::Occupied(_) => false, // already journaled — re-delivery, no re-enact
            HE::Vacant(v) => {
                v.insert(EffectState::Prepared);
                true
            }
        }
    }

    /// `admit` — the Prepared effect is EXPOSED to the CoS context. This is the observable
    /// effect boundary: `admit` returns `Some(effect_id)` exactly once per id (the CALLER enacts
    /// the effect on that first admission), and `None` when the id is already `Admitted` or
    /// `Consumed` — the dedup rule: a re-admission is acknowledged, NOT re-enacted.
    pub fn admit<'a>(&mut self, effect_id: &'a str) -> Option<&'a str> {
        match self.effects.get(effect_id) {
            Some(EffectState::Prepared) => {
                self.effects
                    .insert(effect_id.to_string(), EffectState::Admitted);
                Some(effect_id)
            }
            // Already exposed (Admitted) or already acked (Consumed): dedup — acked, not re-enacted.
            Some(EffectState::Admitted) | Some(EffectState::Consumed) => None,
            None => None, // never prepared: nothing to admit (a lost envelope is NOT silently admitted)
        }
    }

    /// `consume` — the Admitted effect is ACKNOWLEDGED (the enactment is recorded). Returns
    /// `true` when this transitioned Admitted → Consumed (a fresh ack), `false` when the id was
    /// not `Admitted` (already consumed, or never admitted) — no double-ack.
    pub fn consume(&mut self, effect_id: &str) -> bool {
        match self.effects.get(effect_id) {
            Some(EffectState::Admitted) => {
                self.effects
                    .insert(effect_id.to_string(), EffectState::Consumed);
                true
            }
            _ => false,
        }
    }

    /// The ids in a given state, in stable (sorted) order — the derived "what is still pending
    /// acknowledgement" view used by the harness journal reconcile below.
    pub fn ids_in(&self, state: EffectState) -> Vec<&str> {
        let mut ids: Vec<&str> = self
            .effects
            .iter()
            .filter(|(_, s)| **s == state)
            .map(|(id, _)| id.as_str())
            .collect();
        ids.sort_unstable();
        ids
    }

    /// Serialize to the durable record (lossless round-trip through `from_record`).
    pub fn to_record(&self) -> Value {
        let mut entries: Vec<Value> = self
            .effects
            .iter()
            .map(|(id, s)| {
                json!({ "effect_id": id, "state": match s {
                    EffectState::Prepared => "prepared",
                    EffectState::Admitted => "admitted",
                    EffectState::Consumed => "consumed",
                }})
            })
            .collect();
        entries.sort_by(|a, b| a["effect_id"].as_str().cmp(&b["effect_id"].as_str()));
        json!({ "effects": entries })
    }

    /// Rehydrate from the durable record. A record with an unknown state name is CORRUPT (loud),
    /// never silently mapped to a guess — the same fail-closed rule as every other cf store.
    pub fn from_record(v: &Value) -> std::result::Result<Self, String> {
        let arr = v
            .get("effects")
            .and_then(Value::as_array)
            .ok_or_else(|| "journal record is corrupt: missing effects".to_string())?;
        let mut effects = HashMap::new();
        for e in arr {
            let id = e
                .get("effect_id")
                .and_then(Value::as_str)
                .filter(|s| !s.is_empty())
                .ok_or_else(|| "journal entry missing effect_id".to_string())?
                .to_string();
            let state = match e.get("state").and_then(Value::as_str) {
                Some("prepared") => EffectState::Prepared,
                Some("admitted") => EffectState::Admitted,
                Some("consumed") => EffectState::Consumed,
                _ => return Err(format!("journal entry {id:?} has an unknown state")),
            };
            effects.insert(id, state);
        }
        Ok(EffectJournal { effects })
    }
}

/// Reconcile a crash between exposure and ack, from the JOURNAL alone (the durable record is the
/// truth; what lived only in the dead process's memory is irrelevant).
///
/// The harness journal's recovery rule — the "no duplicated effect, no lost envelope" guarantee:
/// * `Prepared` ids (journaled on arrival, never exposed) → admitted now (each enacted EXACTLY
///   once, via `admit`, which is deduped).
/// * `Admitted` ids (exposed but the ack was lost to the crash) → re-acked via `consume` (NOT
///   re-enacted — the effect already happened; only the acknowledgement is recovered).
/// * `Consumed` ids → untouched (acknowledged; nothing to do).
///
/// Returns the effects admitted (enacted) this recovery, in sorted order — the caller enacts
/// each exactly once; a subsequent reconcile of the same journal admits nothing new (idempotent).
pub fn reconcile(ledger: &mut EffectJournal) -> Vec<String> {
    let admitted_now: Vec<String> = ledger
        .ids_in(EffectState::Prepared)
        .into_iter()
        .map(str::to_string)
        .collect();
    let to_reack: Vec<String> = ledger
        .ids_in(EffectState::Admitted)
        .into_iter()
        .map(str::to_string)
        .collect();
    // Re-ack everything already exposed-but-unacked (Admitted) WITHOUT re-enacting it.
    for id in to_reack {
        ledger.consume(&id);
    }
    // Then admit each Prepared id exactly once.
    let mut enacted = Vec::new();
    for id in admitted_now {
        if ledger.admit(&id).is_some() {
            enacted.push(id);
        }
    }
    enacted
}

#[cfg(test)]
mod tests {
    use super::*;

    // ---- state-transition tests (prepared -> admitted -> consumed) ----

    #[test]
    fn transitions_are_ordered_and_idempotent() {
        let mut j = EffectJournal::new();
        assert!(j.prepare("e1"), "arrival journaled");
        assert_eq!(j.state_of("e1"), Some(EffectState::Prepared));
        // re-delivery of the same effect id before admit: acked-not-re-prepared.
        assert!(
            !j.prepare("e1"),
            "a re-push of a journaled effect is not re-prepared"
        );
        assert_eq!(
            j.state_of("e1"),
            Some(EffectState::Prepared),
            "state not downgraded"
        );

        assert_eq!(j.admit("e1"), Some("e1"), "first admit enacts");
        assert_eq!(j.state_of("e1"), Some(EffectState::Admitted));
        assert_eq!(j.admit("e1"), None, "re-admit is acked-not-re-enacted");

        assert!(j.consume("e1"), "ack transitions Admitted->Consumed");
        assert_eq!(j.state_of("e1"), Some(EffectState::Consumed));
        assert!(!j.consume("e1"), "double-ack is a no-op");
        assert_eq!(j.admit("e1"), None, "a consumed effect is never re-enacted");
    }

    #[test]
    fn record_round_trips_losslessly() {
        let mut j = EffectJournal::new();
        j.prepare("a");
        j.admit("a");
        j.consume("a");
        j.prepare("b"); // left Prepared
        let rec = j.to_record();
        let back = EffectJournal::from_record(&rec).unwrap();
        assert_eq!(back, j, "round-trip preserves every effect id + state");
    }

    #[test]
    fn corrupt_record_is_loud_not_inferred() {
        assert!(
            EffectJournal::from_record(&json!({})).is_err(),
            "missing effects is corrupt"
        );
        assert!(
            EffectJournal::from_record(&json!({"effects": [{"effect_id": "x", "state": "bogus"}]}))
                .is_err(),
            "unknown state is corrupt"
        );
        assert!(
            EffectJournal::from_record(&json!({"effects": [{"state": "prepared"}]})).is_err(),
            "missing id is corrupt"
        );
    }

    // ---- fault-injection at EACH boundary: the named qualification test ----

    /// The named Gate 2 qualification test: fault-inject a crash at EACH of the three boundaries
    /// — after-prepared, after-admitted-before-ack, after-consumed — and assert (a) no duplicated
    /// observable effect, (b) no lost-envelope outcome. This is lossless aggregation, NOT an
    /// exactly-once claim.
    #[test]
    fn qual_fault_injection_at_each_boundary_is_lossless_not_duplicated() {
        // The crash is modelled as "the durable journal is the ONLY surviving state" — what was
        // in the dead process's memory (an in-flight admit/consume) is lost. Recovery re-runs
        // `reconcile` from the journal alone. For each boundary, the journal reflects exactly what
        // was durably written before the crash, which IS the boundary's definition.

        // (0) Reference: N delivered envelopes, enacted exactly N times, no loss.
        let reference = run_delivery(&["e1", "e2", "e3"]);
        assert_eq!(
            reference.enacted,
            vec!["e1", "e2", "e3"],
            "no-loss baseline"
        );
        assert_eq!(reference.enacted.len(), 3);

        // (1) Crash AFTER prepared: every envelope journaled Prepared, none exposed. Recovery
        //     admits each exactly once -> enacted == all, no loss, no dup.
        let mut j = EffectJournal::new();
        for id in ["e1", "e2", "e3"] {
            j.prepare(id);
        }
        assert_eq!(
            j.ids_in(EffectState::Prepared).len(),
            3,
            "all prepared, none admitted"
        );
        let enacted = reconciled_enact(&mut j);
        assert_eq!(
            enacted,
            vec!["e1", "e2", "e3"],
            "after-prepared crash: every envelope enacted exactly once"
        );
        assert_eq!(j.ids_in(EffectState::Admitted).len(), 3);

        // (2) Crash AFTER admitted BEFORE ack: the effects were exposed (Admitted) but the ack
        //     (Consumed) was lost. Recovery must NOT re-enact (the effect already happened) — it
        //     re-acks only; enacted count on this recovery is 3 (they were already enacted pre-crash).
        let mut j = EffectJournal::new();
        for id in ["e1", "e2", "e3"] {
            j.prepare(id);
            j.admit(id); // exposed
        }
        // pre-crash, these 3 were enacted. Simulate the crash: ack (consume) never happened.
        assert_eq!(
            j.ids_in(EffectState::Admitted).len(),
            3,
            "all admitted, none consumed (ack lost)"
        );
        let enacted = reconciled_enact(&mut j);
        assert!(
            enacted.is_empty(),
            "after-admitted-before-ack: NOTHING re-enacted (effects already happened)"
        );
        assert_eq!(
            j.ids_in(EffectState::Consumed).len(),
            3,
            "recovery re-acked all three without re-enacting"
        );

        // (3) Crash AFTER consumed: everything acked. Recovery enacts nothing, dedups nothing away.
        let mut j = EffectJournal::new();
        for id in ["e1", "e2", "e3"] {
            j.prepare(id);
            j.admit(id);
            j.consume(id);
        }
        let enacted = reconciled_enact(&mut j);
        assert!(
            enacted.is_empty(),
            "after-consumed: nothing to do, nothing lost, nothing re-enacted"
        );

        // (4) The dedup rule cross-cuts every boundary: re-delivering an already-consumed effect
        //     id is acknowledged-not-re-enacted.
        let mut j = EffectJournal::new();
        j.prepare("e1");
        j.admit("e1");
        j.consume("e1");
        assert!(
            !j.prepare("e1"),
            "a re-delivered consumed effect is acked, not re-prepared"
        );
        assert_eq!(j.admit("e1"), None, "and never re-enacted");
    }

    /// Enact a full delivery of the given effect ids (prepare -> admit -> consume each), observing
    /// how many distinct effects were enacted (exactly `ids.len()`, deduped).
    #[derive(Debug)]
    struct Delivery {
        enacted: Vec<String>,
    }

    fn run_delivery(ids: &[&str]) -> Delivery {
        let mut j = EffectJournal::new();
        let mut enacted = Vec::new();
        for id in ids {
            if j.prepare(id) {
                if j.admit(id).is_some() {
                    enacted.push(id.to_string());
                    j.consume(id);
                }
            }
        }
        Delivery { enacted }
    }

    /// Reconcile a journal after a crash and return the effects enacted THIS recovery (the caller
    /// enacts each exactly once; a second reconcile would enact nothing new).
    fn reconciled_enact(j: &mut EffectJournal) -> Vec<String> {
        reconcile(j)
    }
}
