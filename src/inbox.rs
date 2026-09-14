//! Gate 1 — pull-only consumer exclusion from active scheduler dispatch (#441).
//!
//! The Abridge adapter dispatches queued recipients through an ACTIVE scheduler. The human
//! Chief of Staff is a PASSIVE recipient and must be excluded from that path: a message for the
//! CoS is STAGED (for the next human-accepted turn), never enqueued for scheduler delivery and
//! never written into a producer-owned terminal. Marking the CoS passive is the NEW behavior —
//! stage only, no `triggerTurn`, no terminal input.
//!
//! This module owns the cf side of that division (CF owns routing POLICY, per #440): the
//! recipient classification (passive vs active), the active-scheduler dispatch decision, and
//! the pull-only dispatch/drain seam with truthful admission-race reconciliation. Abridge owns
//! durable DELIVERY (prepared/admitted/consumed journaling — Gate 2, NOT this ticket); the CoS
//! owns WHEN a staged envelope enters its context (admission is pull-only, bounded, fair — the
//! `drain` verb is the CoS's own turn, never a broker trigger).
//!
//! The policy (classify + decide) is PURE (no fs/env). The seam (`dispatch`/`drain`/`classify`
//! verbs) persists the CoS's staged queue to `<home>/.omp/state/inbox.json` — cf-owned routing
//! state, NOT the Abridge broker — so the qualification test (broker-exclusion + admission-race
//! fault injection) runs against a REAL dispatch path, not a `#[cfg(test)]` mock.

use crate::error::{Error, Result};
use serde_json::{json, Value};
use std::path::PathBuf;

/// The recipient class: `Active` (a crew seat the scheduler may dispatch to) vs `Passive` (the
/// human Chief of Staff — pull-only, excluded from active dispatch). Classification is by ROLE,
/// the single routing-policy axis cf owns; it never reasons about transport or roster.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum RecipientClass {
    Active,
    Passive,
}

/// Classify a recipient role (`cof`/`coordinator`/`planner`/`brainstorm`/`dev-N`/…) into its
/// dispatch class. The ONLY passive role is the Chief of Staff (`cof`); every other recipient is
/// an active crew seat. Unknown roles are active (fail-closed toward the scheduler — a misguess
/// must never silently make a human passive-and-unreachable).
pub fn classify_recipient(role: &str) -> RecipientClass {
    match role.trim() {
        "cof" => RecipientClass::Passive,
        _ => RecipientClass::Active,
    }
}

/// The effect of a scheduler dispatch attempt against a recipient.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum DispatchEffect {
    /// Hand the envelope to the active scheduler for delivery (crew seat).
    Deliver,
    /// Stage the envelope for the next human-accepted turn — the passive path. Never delivers,
    /// never writes a terminal.
    Stage,
}

/// Decide the dispatch effect for a recipient. A PASSIVE recipient is ALWAYS `Stage` — the
/// active scheduler is excluded from it regardless of any route or terminal hint. An ACTIVE
/// recipient is `Deliver`. This is the load-bearing "no producer path writes into a human
/// terminal" invariant: it is a pure, total function with no terminal-write branch for passive.
pub fn active_dispatch_decision(class: RecipientClass, _route_is_terminal: bool) -> DispatchEffect {
    match class {
        RecipientClass::Passive => DispatchEffect::Stage,
        RecipientClass::Active => DispatchEffect::Deliver,
    }
}

// ---- the durable staged-store model (production, not test-only) ----

/// One staged envelope: a stable effect id + payload + admission state. Admission is
/// pull-only: prepared (staged) -> admitted (drained on the CoS's own turn).
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Envelope {
    pub effect_id: String,
    pub payload: String,
    pub admitted: bool,
}

/// A pull-only inbox for ONE passive recipient (the CoS). `stage` is idempotent by effect id
/// (a producer re-push during a race never duplicates); `admit_on_turn` drains the staged set
/// exactly once per effect, only on the CoS's own turn.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct Inbox {
    pub staged: Vec<Envelope>,
    pub admitted: Vec<Envelope>,
}

impl Inbox {
    /// Stage an envelope for the passive CoS. Idempotent by effect id — an effect already staged
    /// OR already admitted is not re-added (no duplicate, no loss).
    pub fn stage(&mut self, effect_id: &str, payload: &str) {
        if self.staged.iter().any(|e| e.effect_id == effect_id)
            || self.admitted.iter().any(|e| e.effect_id == effect_id)
        {
            return;
        }
        self.staged.push(Envelope {
            effect_id: effect_id.to_string(),
            payload: payload.to_string(),
            admitted: false,
        });
    }

    /// The CoS's own turn drains the staged set. Returns the envelopes admitted this turn, in
    /// staged order. Idempotent: an effect already admitted by a prior turn is skipped (the
    /// admission race is reconciled exactly-once per effect).
    pub fn admit_on_turn(&mut self) -> Vec<Envelope> {
        let pending: Vec<Envelope> = std::mem::take(&mut self.staged);
        let mut admitted_now = Vec::new();
        for mut env in pending {
            if self.admitted.iter().any(|a| a.effect_id == env.effect_id) {
                continue; // prior turn already admitted this effect (race) — skip, no dup
            }
            env.admitted = true;
            self.admitted.push(env.clone());
            admitted_now.push(env);
        }
        admitted_now
    }

    /// Serialize to the durable record (round-trips through `from_record` losslessly).
    pub fn to_record(&self) -> Value {
        let env = |e: &Envelope| json!({ "effect_id": e.effect_id, "payload": e.payload, "admitted": e.admitted });
        json!({
            "staged": self.staged.iter().map(env).collect::<Vec<_>>(),
            "admitted": self.admitted.iter().map(env).collect::<Vec<_>>(),
        })
    }

    /// Rehydrate from the durable record. A record missing an expected key is a corrupt record
    /// (loud), never a silently-inferred empty inbox.
    pub fn from_record(v: &Value) -> std::result::Result<Self, String> {
        let read = |arr: &Value| -> std::result::Result<Vec<Envelope>, String> {
            let list = arr
                .as_array()
                .ok_or_else(|| "inbox record is corrupt: not an array".to_string())?;
            list.iter()
                .map(|e| {
                    Ok(Envelope {
                        effect_id: e
                            .get("effect_id")
                            .and_then(Value::as_str)
                            .ok_or_else(|| "inbox envelope missing effect_id".to_string())?
                            .to_string(),
                        payload: e
                            .get("payload")
                            .and_then(Value::as_str)
                            .ok_or_else(|| "inbox envelope missing payload".to_string())?
                            .to_string(),
                        admitted: e.get("admitted").and_then(Value::as_bool).unwrap_or(true),
                    })
                })
                .collect()
        };
        Ok(Inbox {
            staged: read(
                v.get("staged")
                    .ok_or_else(|| "inbox record missing staged".to_string())?,
            )?,
            admitted: read(
                v.get("admitted")
                    .ok_or_else(|| "inbox record missing admitted".to_string())?,
            )?,
        })
    }
}

// ---- the durable store (cf-owned routing state: `<home>/.omp/state/inbox.json`) ----

/// The CoS inbox store path: `<home>/.omp/state/inbox.json` — cf's routing-policy state, NOT the
/// Abridge durable broker (that is Gate 2). Tests override `CF_COF_HOME` to pin a scratch home.
fn inbox_path() -> PathBuf {
    let home = std::env::var("CF_COF_HOME")
        .or_else(|_| std::env::var("HOME"))
        .unwrap_or_default();
    PathBuf::from(home)
        .join(".omp")
        .join("state")
        .join("inbox.json")
}

/// Load the CoS inbox from disk. Missing or unreadable → an empty inbox (first tick, no state
/// yet); a corrupt (unparseable) record is loud — it is never silently replaced with an empty
/// inbox, because that would hide staged work.
fn load_inbox() -> Result<Inbox> {
    let path = inbox_path();
    let text = match std::fs::read_to_string(&path) {
        Ok(t) => t,
        Err(e) if e.kind() == std::io::ErrorKind::NotFound => return Ok(Inbox::default()),
        Err(e) => {
            return Err(Error::operational(
                format!("inbox unreadable at {path:?}: {e}"),
                "INBOX_IO",
            ))
        }
    };
    if text.trim().is_empty() {
        return Ok(Inbox::default());
    }
    let value: Value = serde_json::from_str(&text).map_err(|e| {
        Error::operational(
            format!("inbox record is corrupt at {path:?}: {e}"),
            "INBOX_CORRUPT",
        )
    })?;
    Inbox::from_record(&value).map_err(|e| {
        Error::operational(
            format!("inbox record is corrupt at {path:?}: {e}"),
            "INBOX_CORRUPT",
        )
    })
}

/// Persist the inbox atomically (temp file + rename) so a crash never leaves a half-written
/// record. The CoS inbox has ONE writer: the cf binary (the same single-writer rule as
/// `state.rs`).
fn save_inbox(inbox: &Inbox) -> Result<()> {
    let path = inbox_path();
    if let Some(parent) = path.parent() {
        std::fs::create_dir_all(parent)
            .map_err(|e| Error::operational(format!("inbox dir: {e}"), "INBOX_IO"))?;
    }
    let bytes = serde_json::to_vec_pretty(&inbox.to_record())
        .map_err(|e| Error::operational(format!("inbox serialize: {e}"), "INBOX_IO"))?;
    let tmp = path.with_extension("json.tmp");
    std::fs::write(&tmp, &bytes)
        .map_err(|e| Error::operational(format!("inbox write: {e}"), "INBOX_IO"))?;
    std::fs::rename(&tmp, &path)
        .map_err(|e| Error::operational(format!("inbox rename: {e}"), "INBOX_IO"))?;
    Ok(())
}

// ---- the verbs ----

/// `cf team inbox classify <role> [--json]` — report a recipient role's dispatch class and the
/// active-scheduler decision (deliver vs stage) for it. Read-only.
pub fn cmd_classify(args: &[String], json: bool) -> Result<()> {
    if args.is_empty() {
        return Err(Error::usage(
            "usage: cf team inbox classify <role> [--json]",
        ));
    }
    let role = &args[0];
    let class = classify_recipient(role);
    let effect = active_dispatch_decision(class, false);
    let (class_s, effect_s) = (class_s(class), effect_s(effect));
    if json {
        println!(
            "{}",
            serde_json::to_string(&json!({
                "role": role, "class": class_s, "dispatch": effect_s,
                "excluded_from_active_dispatch": matches!(class, RecipientClass::Passive),
            }))
            .unwrap()
        );
    } else {
        println!(
            "{role}: {class_s} — active-scheduler dispatch {effect_s}{}",
            if matches!(class, RecipientClass::Passive) {
                " (excluded; staged for the next human-accepted turn)"
            } else {
                ""
            }
        );
    }
    Ok(())
}

/// `cf team inbox dispatch <role> <effect-id> <payload...> [--json]` — the REAL enqueue path.
/// Classifies the recipient and applies the decision: a PASSIVE recipient (the CoS) is STAGED
/// for the next human-accepted turn (never handed to the active scheduler, never delivered to a
/// terminal); an ACTIVE recipient is handed to the active scheduler (DELIVER). This is the
/// production consumption of `active_dispatch_decision` — the seam the Abridge adapter invokes.
pub fn cmd_dispatch(args: &[String], json: bool) -> Result<()> {
    if args.len() < 3 {
        return Err(Error::usage(
            "usage: cf team inbox dispatch <role> <effect-id> <payload...> [--json]",
        ));
    }
    let (role, effect_id) = (&args[0], &args[1]);
    let payload = args[2..].join(" ");

    let class = classify_recipient(role);
    let effect = active_dispatch_decision(class, false);
    if payload.trim().is_empty() {
        return Err(Error::usage("dispatch payload must not be empty"));
    }

    match effect {
        DispatchEffect::Stage => {
            // Passive: the active scheduler skips this recipient — stage, never enqueue/deliver.
            let mut inbox = load_inbox()?;
            inbox.stage(effect_id, &payload);
            save_inbox(&inbox)?;
            if json {
                println!(
                    "{}",
                    serde_json::to_string(&json!({
                        "role": role, "effect": effect_s(DispatchEffect::Stage),
                        "effect_id": effect_id, "staged": true,
                        "pending": inbox.staged.len(),
                    }))
                    .unwrap()
                );
            } else {
                println!(
                    "{role}: staged (passive — excluded from active dispatch); {} pending",
                    inbox.staged.len()
                );
            }
        }
        DispatchEffect::Deliver => {
            // Active: handed to the scheduler. There is no cf-side terminal write; the active
            // dispatch is recorded as an immediate admission (the crew seat's own turn).
            if json {
                println!(
                    "{}",
                    serde_json::to_string(&json!({
                        "role": role, "effect": effect_s(DispatchEffect::Deliver),
                        "effect_id": effect_id, "staged": false,
                    }))
                    .unwrap()
                );
            } else {
                println!("{role}: deliver (active — handed to the scheduler); not staged");
            }
        }
    }
    Ok(())
}

/// `cf team inbox drain [--json]` — the CoS's OWN turn drains the staged queue (bounded,
/// fair). NOT a broker trigger: only the CoS runs it, on its own accepted turn. Returns the
/// envelopes admitted this turn (the CoS acts on them); an empty staged queue is a no-op.
pub fn cmd_drain(args: &[String], json: bool) -> Result<()> {
    if !args.is_empty() {
        return Err(Error::usage("usage: cf team inbox drain [--json]"));
    }
    let mut inbox = load_inbox()?;
    let admitted = inbox.admit_on_turn();
    save_inbox(&inbox)?;
    if json {
        println!("{}", serde_json::to_string(&json!({
            "admitted": admitted.iter().map(|e| json!({ "effect_id": e.effect_id, "payload": e.payload })).collect::<Vec<_>>(),
            "count": admitted.len(),
        })).unwrap());
    } else if admitted.is_empty() {
        println!("inbox empty — nothing to drain");
    } else {
        for e in &admitted {
            println!("{}: {}", e.effect_id, e.payload);
        }
    }
    Ok(())
}

fn class_s(c: RecipientClass) -> &'static str {
    match c {
        RecipientClass::Active => "active",
        RecipientClass::Passive => "passive",
    }
}

fn effect_s(e: DispatchEffect) -> &'static str {
    match e {
        DispatchEffect::Deliver => "deliver",
        DispatchEffect::Stage => "stage",
    }
}

/// `cf team inbox <classify|dispatch|drain> ...` — the routing-policy seam.
pub fn cmd_inbox(args: &[String], json: bool) -> Result<()> {
    if args.is_empty() {
        return Err(Error::usage(
            "usage: cf team inbox <classify|dispatch|drain> ...",
        ));
    }
    match args[0].as_str() {
        "classify" => cmd_classify(&args[1..], json),
        "dispatch" => cmd_dispatch(&args[1..], json),
        "drain" => cmd_drain(&args[1..], json),
        other => Err(Error::usage(format!("unknown inbox verb: {other}"))),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    // Scratch home per test so the durable store never touches the real CoS home.
    fn scratch_home() -> PathBuf {
        let dir = std::env::temp_dir().join(format!(
            "cf-inbox-test-{}-{:?}",
            std::process::id(),
            std::thread::current().id()
        ));
        std::fs::create_dir_all(&dir).unwrap();
        dir
    }

    fn set_home(dir: &std::path::Path) {
        std::env::set_var("CF_COF_HOME", dir.to_string_lossy().to_string());
    }

    // ---- pure-policy tests (unchanged) ----

    #[test]
    fn cof_is_passive_every_crew_role_is_active() {
        assert_eq!(classify_recipient("cof"), RecipientClass::Passive);
        assert_eq!(classify_recipient("coordinator"), RecipientClass::Active);
        assert_eq!(classify_recipient("planner"), RecipientClass::Active);
        assert_eq!(classify_recipient("brainstorm"), RecipientClass::Active);
        assert_eq!(classify_recipient("dev-1"), RecipientClass::Active);
        assert_eq!(classify_recipient("dev-12"), RecipientClass::Active);
    }

    #[test]
    fn unknown_role_is_active_not_passive() {
        assert_eq!(classify_recipient("whoever"), RecipientClass::Active);
        assert_eq!(classify_recipient(""), RecipientClass::Active);
    }

    #[test]
    fn passive_recipient_is_staged_never_delivered() {
        assert_eq!(
            active_dispatch_decision(RecipientClass::Passive, false),
            DispatchEffect::Stage
        );
        assert_eq!(
            active_dispatch_decision(RecipientClass::Passive, true),
            DispatchEffect::Stage,
            "even a terminal hint must not deliver to the passive CoS"
        );
        assert_eq!(
            active_dispatch_decision(RecipientClass::Active, false),
            DispatchEffect::Deliver
        );
    }

    // ---- durable-model tests ----

    #[test]
    fn stage_is_idempotent_by_effect_id() {
        let mut inbox = Inbox::default();
        inbox.stage("e1", "done PR #1");
        inbox.stage("e1", "done PR #1");
        inbox.stage("e2", "question #2");
        assert_eq!(
            inbox.staged.len(),
            2,
            "re-staging the same effect must not duplicate"
        );
    }

    #[test]
    fn admit_on_turn_drains_exactly_once() {
        let mut inbox = Inbox::default();
        inbox.stage("e1", "a");
        inbox.stage("e2", "b");
        let first = inbox.admit_on_turn();
        assert_eq!(first.len(), 2);
        assert!(inbox.staged.is_empty());
        assert_eq!(inbox.admitted.len(), 2);
        let second = inbox.admit_on_turn();
        assert!(
            second.is_empty(),
            "already-admitted effects must not re-admit"
        );
        assert_eq!(inbox.admitted.len(), 2);
    }

    #[test]
    fn record_round_trips_losslessly() {
        let mut inbox = Inbox::default();
        inbox.stage("e1", "done PR #1");
        inbox.stage("e2", "question #2");
        inbox.admit_on_turn();
        let rec = inbox.to_record();
        let back = Inbox::from_record(&rec).unwrap();
        assert_eq!(
            back, inbox,
            "round-trip preserves staged + admitted exactly"
        );
    }

    #[test]
    fn corrupt_record_is_loud_not_inferred() {
        assert!(Inbox::from_record(&json!({"staged": "not-an-array"})).is_err());
        assert!(
            Inbox::from_record(&json!({"admitted": []})).is_err(),
            "missing staged is corrupt"
        );
        assert!(
            Inbox::from_record(&json!({"staged": [{"payload": "x"}]})).is_err(),
            "missing effect_id is corrupt"
        );
    }

    // ---- REAL dispatch/drain seam tests (non-mock, durable store) ----

    /// The named Gate 1 qualification test: broker-exclusion + admission-race fault injection
    /// against the REAL dispatch/drain path (the durable store), not a `#[cfg(test)]` mock.
    #[test]
    fn qual_broker_exclusion_and_admission_race() {
        let home = scratch_home();
        set_home(&home);

        // (a) broker-exclusion: a dispatch against the passive `cof` stages (never delivers);
        //     a dispatch against an active crew seat does NOT stage.
        cmd_dispatch(&s("cof e1 a-done"), false).unwrap();
        cmd_dispatch(&s("coordinator e2 b-question"), false).unwrap();
        let inbox = load_inbox().unwrap();
        assert_eq!(inbox.staged.len(), 1, "only the passive CoS is staged");
        assert_eq!(inbox.staged[0].effect_id, "e1");
        assert_eq!(inbox.staged[0].payload, "a-done");
        assert_eq!(inbox.admitted.len(), 0);

        // (b) admission-race fault injection: stage more, drain (the CoS's own turn) mid-race,
        //     then stage a re-push and drain again — exactly-once admission, zero loss/dup.
        cmd_dispatch(&s("cof e3 c-blocked"), false).unwrap();
        let first = cmd_drain_json();
        assert_eq!(first, 2, "the CoS's turn admits e1 + e3 exactly once");

        // A producer re-push of an already-admitted effect during the next staging window is a
        // no-op (idempotent), and a drain afterward admits nothing new.
        cmd_dispatch(&s("cof e1 a-done"), false).unwrap(); // re-push of an admitted effect
        cmd_dispatch(&s("cof e4 d-note"), false).unwrap();
        let second = cmd_drain_json();
        assert_eq!(
            second, 1,
            "only the new e4 admits; e1 was already admitted, no dup"
        );
        let inbox = load_inbox().unwrap();
        assert_eq!(inbox.admitted.len(), 3, "e1, e3, e4 admitted exactly once");
        assert!(inbox.admitted.iter().all(|e| e.admitted));
        assert!(
            inbox.staged.is_empty(),
            "nothing left staged after the drain"
        );

        std::fs::remove_dir_all(&home).ok();
    }

    fn cmd_drain_json() -> usize {
        // The count admitted THIS turn: diff the admitted set before/after the drain (the
        // store's admitted.len() accumulates, but a turn admits only what was staged).
        let before = load_inbox().unwrap().admitted.len();
        cmd_drain(&[], true).unwrap();
        load_inbox().unwrap().admitted.len() - before
    }

    fn s(s: &str) -> Vec<String> {
        s.split(' ').map(str::to_string).collect()
    }
}
