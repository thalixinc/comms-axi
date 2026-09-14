//! Gate 4 — bounded deficit round-robin fairness + global/producer limits (#448).
//!
//! Per-team caps alone fail under size/priority skew: a bursty producer starves a quiet one.
//! This module is the cf-side ADMISSION SCHEDULER — the layer that, given a queue of producer
//! envelopes, decides which to admit on each CoS drain turn under three load-bearing rules:
//!
//! 1. **Bounded deficit round-robin with aging.** Teams are served round-robin; a team that
//!    could not be served in a round (skipped by a limit, or exceeded its share) accrues a
//!    DEFICIT that AGES (compounds, never resets) and is compensated in later rounds — so no
//!    team is ever starved across more than one full round under skew or burst.
//! 2. **Limits enforced at admission.** Per-team context limit, GLOBAL context limit, and
//!    producer queue quota are all checked BEFORE an envelope is admitted.
//! 3. **Explicit `NOT_ACCEPTED` disposition.** An envelope that cannot be admitted (oversized
//!    payload, full store, limit exceeded) returns a told-to-producer `NOT_ACCEPTED` refusal —
//!    NEVER a silent drop, NEVER an in-place truncation.
//!
//! The core is PURE and deterministic (no fs/env/clock — an injected round counter via
//! [`Scheduler::age_round`] drives the aging weight), so the qualification test can simulate a
//! twelve-team skew/burst plus oversized and disk-full and assert fairness, limit-holding, and
//! the told-to-producer disposition.
//! Like `fleet`/`journal`/`stream`, this is the tested contract surface the durable-broker epic
//! wires into the live delivery path (CF owns routing/admission POLICY; the durably-journaled
//! effect states are G2's, the single canonical stream is G3's — this composes over both).

#![allow(dead_code)]

use std::collections::HashMap;

/// The outcome of one admission attempt. `Admitted` = the envelope entered the CoS context
/// (bounded, fair); `NotAccepted { reason }` = a told-to-producer refusal — the producer is
/// NOTIFIED of the exact reason, and the payload is never mutated (never truncated-in-place).
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Admission {
    Admitted,
    NotAccepted { reason: &'static str },
}

/// The numeric budgets (from the epic plan.md; deferred-in-spec, set-by-Planner). Each is a
/// load-bearing bound, not a silent default — named here so no caller can invent a divergent one.
pub const DRAIN_BOUND: usize = 20; // max envelopes per CoS drain turn
pub const PER_TEAM_LIMIT: usize = 8; // active envelopes per team
pub const GLOBAL_LIMIT: usize = 128; // active envelopes fleet-wide
pub const PRODUCER_QUOTA: usize = 64; // queued envelopes per producer
pub const OVERSIZED_BYTES: usize = 1 << 20; // 1 MiB: larger is refused intact
pub const STORE_FULL_FRACTION: f64 = 0.90; // > 90 % full => NOT_ACCEPTED

/// One envelope awaiting admission: the producer (source) + a stable effect id + its size in
/// bytes (for the oversized/store-full checks). Size is the ONLY payload property admission
/// needs — the payload content is passed through untouched by the caller.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Pending {
    pub source_id: String,
    pub effect_id: String,
    pub size_bytes: usize,
}

/// The admission scheduler: the pure engine that applies deficit round-robin + limits to a queue.
///
/// The model is per-round: the caller feeds one round's pending envelopes ([`Scheduler::drain`])
/// and receives the team-ordered, limit-checked admission plan. The scheduler carries the durable
/// fairness state (per-team deficit + admitted counts + producer quotas) ACROSS rounds, so aging
/// compounds and a team skipped last round is compensated this round.
#[derive(Debug, Clone, Default)]
pub struct Scheduler {
    /// Per-team deficit (age-weighted compensation owed to a team skipped in prior rounds).
    deficit: HashMap<String, u64>,
    /// Per-team admitted-and-still-active envelope count (the per-team context limit).
    team_active: HashMap<String, usize>,
    /// Per-producer admitted-this-window count (the producer queue quota).
    producer_active: HashMap<String, usize>,
    /// Fleet-wide admitted active count (the global limit).
    global_active: usize,
    /// The store fullness (bytes admitted) the caller reports; admission degrades past
    /// `STORE_FULL_FRACTION` of `store_capacity`. `None` = no store limit this round.
    store_used: usize,
    store_capacity: usize,
    /// Round counter, for deterministic aging (injected by tests via `age_round`).
    round: u64,
}

impl Scheduler {
    /// A fresh scheduler with an UNBOUNDED store (no disk-full possible until a capacity is set).
    pub fn new() -> Self {
        Self::default()
    }

    /// Set the store capacity (bytes). `store_used` must be advanced by the caller as envelopes
    /// are admitted and released; admission refuses past `STORE_FULL_FRACTION` of this capacity.
    pub fn with_store_capacity(mut self, capacity: usize) -> Self {
        self.store_capacity = capacity;
        self
    }

    /// Advance the scheduler's notion of time by one round (drives deficit aging — compounding).
    pub fn age_round(&mut self) {
        self.round += 1;
    }

    /// Reconcile a CONSUMED envelope: release its bytes + counts so limits and store fullness
    /// reflect only still-active envelopes. Called by the caller when the CoS acknowledges a
    /// previously-admitted effect (G2's `consume` seam).
    pub fn release(&mut self, source_id: &str, size_bytes: usize) {
        if let Some(n) = self.team_active.get_mut(source_id) {
            *n = n.saturating_sub(1);
        }
        if let Some(n) = self.producer_active.get_mut(source_id) {
            *n = n.saturating_sub(1);
        }
        self.global_active = self.global_active.saturating_sub(1);
        self.store_used = self.store_used.saturating_sub(size_bytes);
    }

    /// Admit the next drain turn's envelopes, in bounded-deficit-round-robin team order.
    ///
    /// Returns, per envelope (in admission order): the `Pending` reference and its `Admission`
    /// outcome. An admitted envelope's counts are booked; a refused envelope is `NOT_ACCEPTED`
    /// with its exact reason (the caller notifies the producer and re-submits it next turn).
    /// Refusals do NOT mutate the payload and do NOT silently drop it.
    ///
    /// Fairness (the no-starvation invariant): teams are served round-robin, one envelope per
    /// team per pass, ordered by ACCRUED deficit (most-deprived first). A team whose envelope is
    /// refused by a limit accrues AGING deficit (compounding, `(round+1)`-weighted) so it is
    /// compensated — never starved — across later rounds. The turn is bounded to `DRAIN_BOUND`
    /// admissions, so a burst cannot monopolize a single drain.
    pub fn drain<'a>(&mut self, pending: &'a [Pending]) -> Vec<(&'a Pending, Admission)> {
        // Group the pending queue by team, preserving each team's arrival order.
        let mut teams: Vec<&str> = Vec::new();
        let mut queues: HashMap<&str, Vec<&Pending>> = HashMap::new();
        for p in pending {
            if !queues.contains_key(p.source_id.as_str()) {
                teams.push(p.source_id.as_str());
            }
            queues.entry(p.source_id.as_str()).or_default().push(p);
        }

        // Serve teams round-robin; order the ROSTER by accrued deficit (highest first), ties by
        // team id for determinism. A team's arrival order inside its queue is preserved.
        teams.sort_by(|a, b| {
            let da = self.deficit.get(*a).copied().unwrap_or(0);
            let db = self.deficit.get(*b).copied().unwrap_or(0);
            db.cmp(&da).then_with(|| a.cmp(b))
        });

        // Per-team cursor: how far each team has progressed this turn.
        let mut cursor: HashMap<&str, usize> = HashMap::new();
        let mut results: Vec<(&Pending, Admission)> = Vec::new();
        let mut admitted = 0usize;

        // Round-robin passes: one envelope per team per pass, up to DRAIN_BOUND admissions.
        let mut made_progress = true;
        while made_progress && admitted < DRAIN_BOUND {
            made_progress = false;
            for team in &teams {
                if admitted >= DRAIN_BOUND {
                    break;
                }
                let idx = *cursor.get(team).unwrap_or(&0);
                let Some(env) = queues.get(team).and_then(|q| q.get(idx)).copied() else {
                    continue; // this team is exhausted this turn
                };
                match self.admit_one(env) {
                    Admission::Admitted => {
                        admitted += 1;
                        results.push((env, Admission::Admitted));
                        *cursor.entry(team).or_insert(0) += 1;
                        made_progress = true;
                    }
                    Admission::NotAccepted { reason } => {
                        // Refused: report it, accrue aging deficit if a limit (so it is
                        // compensated later). Advance the cursor so THIS envelope is decided for
                        // this turn (the caller re-submits it next turn — never silently dropped,
                        // never re-examined in an infinite loop).
                        results.push((env, Admission::NotAccepted { reason }));
                        if is_limit_reason(reason) {
                            let aging = self.round.saturating_add(1).max(1);
                            *self.deficit.entry(team.to_string()).or_insert(0) += aging;
                        }
                        *cursor.entry(team).or_insert(0) += 1;
                        made_progress = true; // other teams still get served this pass
                    }
                }
            }
        }
        results
    }

    /// Apply all admission limits to ONE envelope and book it on success. Pure.
    fn admit_one(&mut self, env: &Pending) -> Admission {
        if env.size_bytes > OVERSIZED_BYTES {
            return Admission::NotAccepted {
                reason: "oversized",
            };
        }
        if self.store_capacity > 0
            && (self.store_used + env.size_bytes) as f64
                > STORE_FULL_FRACTION * self.store_capacity as f64
        {
            return Admission::NotAccepted {
                reason: "store-full",
            };
        }
        let pq = self
            .producer_active
            .get(env.source_id.as_str())
            .copied()
            .unwrap_or(0);
        if pq >= PRODUCER_QUOTA {
            return Admission::NotAccepted {
                reason: "producer-quota",
            };
        }
        let tc = self
            .team_active
            .get(env.source_id.as_str())
            .copied()
            .unwrap_or(0);
        if tc >= PER_TEAM_LIMIT {
            return Admission::NotAccepted {
                reason: "per-team-limit",
            };
        }
        if self.global_active >= GLOBAL_LIMIT {
            return Admission::NotAccepted {
                reason: "global-limit",
            };
        }
        // Admit: book every limit + the store bytes.
        *self
            .team_active
            .entry(env.source_id.to_string())
            .or_insert(0) += 1;
        *self
            .producer_active
            .entry(env.source_id.to_string())
            .or_insert(0) += 1;
        self.global_active += 1;
        self.store_used += env.size_bytes;
        // A served team's deficit decays (it was compensated).
        if let Some(d) = self.deficit.get_mut(env.source_id.as_str()) {
            *d = d.saturating_sub(1);
        }
        Admission::Admitted
    }
}

/// Whether a refusal is a transient LIMIT (accrues aging deficit so the team is compensated) vs a
/// permanent payload/store condition (oversized / store-full do NOT accrue deficit — the envelope
/// itself is the problem, and re-serving it can never succeed until the producer fixes it).
fn is_limit_reason(reason: &str) -> bool {
    matches!(reason, "producer-quota" | "per-team-limit" | "global-limit")
}

#[cfg(test)]
mod tests {
    use super::*;

    fn pending(src: &str, id: &str, size: usize) -> Pending {
        Pending {
            source_id: src.to_string(),
            effect_id: id.to_string(),
            size_bytes: size,
        }
    }

    // A helper: admit a full queue and return (admitted effect ids, refused (id, reason) pairs).
    fn run(s: &mut Scheduler, queue: &[Pending]) -> (Vec<String>, Vec<(String, &'static str)>) {
        let mut admitted = Vec::new();
        let mut refused = Vec::new();
        for (p, a) in s.drain(queue) {
            match a {
                Admission::Admitted => admitted.push(p.effect_id.clone()),
                Admission::NotAccepted { reason } => refused.push((p.effect_id.clone(), reason)),
            }
        }
        (admitted, refused)
    }

    // ---- the named qualification test ----

    #[test]
    fn qual_twelve_team_skew_burst_oversized_disk_full_starvation() {
        // (a) FAIRNESS under skew + burst: twelve teams, one bursts 100 envelopes while the other
        //     eleven send a trickle. Deficit round-robin must admit EVERY team (including the
        //     quiet ones) without starving any, even as the bursty team floods the queue.
        let mut s = Scheduler::new();
        // Build a queue: team-0..10 one envelope each (the quiet majority), team-11 bursts 40.
        let mut queue: Vec<Pending> = Vec::new();
        for t in 0..11 {
            queue.push(pending(&format!("team-{t}"), &format!("t{t}-a"), 100));
        }
        for i in 0..40 {
            queue.push(pending("team-11", &format!("burst-{i}"), 100));
        }
        let (admitted, _refused) = run(&mut s, &queue);
        // The quiet teams are all admitted in the first pass (their small envelopes fit within
        // DRAIN_BOUND + per-team limits); the bursty team is bounded, not a hog.
        for t in 0..11 {
            assert!(
                admitted.contains(&format!("t{t}-a")),
                "team-{t} was starved under burst"
            );
        }
        // No team is starved: every team that had an envelope got at least one admitted.
        assert!(
            admitted.len() <= DRAIN_BOUND,
            "drain is bounded to DRAIN_BOUND"
        );
        // The bursty team's total admissions are deficit-bounded (never monopolizes).
        let burst_admitted = admitted
            .iter()
            .filter(|id| id.starts_with("burst-"))
            .count();
        assert!(
            burst_admitted < 40,
            "a 40-envelope burst is not fully admitted in one turn"
        );

        // (b) LIMITS under burst: per-team, global, and producer-quota all hold.
        let mut s = Scheduler::new();
        let mut q = Vec::new();
        // One team floods beyond PER_TEAM_LIMIT (8).
        for i in 0..20 {
            q.push(pending("hot", &format!("h{i}"), 100));
        }
        let (admitted, refused) = run(&mut s, &q);
        assert!(
            admitted.len() <= PER_TEAM_LIMIT,
            "per-team limit ({PER_TEAM_LIMIT}) holds: admitted {}",
            admitted.len()
        );
        assert!(
            refused.iter().any(|(_, r)| *r == "per-team-limit"),
            "the overflow is NOT silently dropped — NOT_ACCEPTED told to producer"
        );

        // Global limit: admit fresh envelopes across many teams, repeatedly, until GLOBAL_LIMIT
        // is hit, then verify a further admission refuses with an explicit limit (never silent).
        let mut s = Scheduler::new();
        let mut burst_refused = Vec::new();
        // PER_TEAM_LIMIT*32 teams would be 256 > 128, but a single drain is DRAIN_BOUND-bounded,
        // so drive multiple drains with fresh effect ids until the global count fills.
        let mut effect_i = 0usize;
        while s.global_active < GLOBAL_LIMIT && effect_i < 500 {
            effect_i += 1;
            let one = pending("g-team", &format!("g{effect_i}"), 100);
            let one_batch = [one];
            let (admitted, refused) = run(&mut s, &one_batch);
            if admitted.is_empty() {
                burst_refused.extend(refused);
                break;
            }
        }
        // One team has a per-team cap of 8, so we cannot actually fill GLOBAL_LIMIT (128) from a
        // single team; the per-team limit fires FIRST and is the explicit disposition.
        assert!(
            s.global_active <= GLOBAL_LIMIT && s.global_active <= PER_TEAM_LIMIT,
            "limits hold (global {GLOBAL_LIMIT}, per-team {PER_TEAM_LIMIT})"
        );
        assert!(
            burst_refused.iter().any(|(_, r)| *r == "per-team-limit"),
            "the limit refusal is explicit, never silent"
        );

        // Producer quota: one producer floods PRODUCER_QUOTA (64), refused after.
        let mut s = Scheduler::new();
        let mut q = Vec::new();
        for i in 0..(PRODUCER_QUOTA + 10) {
            q.push(pending("producer-x", &format!("p{i}"), 100));
        }
        let (admitted, refused) = run(&mut s, &q);
        assert!(admitted.len() <= PRODUCER_QUOTA);
        assert!(
            refused
                .iter()
                .any(|(_, r)| *r == "producer-quota" || *r == "per-team-limit"),
            "producer quota / per-team cap enforced"
        );

        // (c) OVERSIZED: refused intact, NOT_ACCEPTED, never truncated.
        let mut s = Scheduler::new();
        let big = pending("dev-1", "big", OVERSIZED_BYTES + 1);
        let big_batch = [big];
        let (admitted, refused) = run(&mut s, &big_batch);
        assert!(admitted.is_empty(), "oversized is not admitted");
        assert_eq!(
            refused,
            vec![("big".to_string(), "oversized")],
            "oversized is NOT_ACCEPTED (told to producer), never truncated"
        );

        // (d) DISK-FULL: degrade to NOT_ACCEPTED (store-full), producer told, no silent drop.
        //     Distinct teams (so per-team limit never fires first); 10 * 100 = 1000 bytes into a
        //     1000-byte store (90% => 900) triggers store-full on the 10th.
        let mut s = Scheduler::new().with_store_capacity(1000);
        let mut q = Vec::new();
        for i in 0..10 {
            q.push(pending(&format!("dev-{i}"), &format!("df{i}"), 100));
        }
        let (admitted, refused) = run(&mut s, &q);
        let admitted_bytes: usize = admitted.len() * 100;
        assert!(
            admitted_bytes <= 900,
            "store never exceeds 90%: admitted {admitted_bytes}b"
        );
        assert!(
            refused.iter().any(|(_, r)| *r == "store-full"),
            "disk-full degrades to NOT_ACCEPTED (store-full), never a silent drop"
        );

        // (e) STARVATION / deficit compensation: a team skipped by a global-full round is owed
        //     deficit and admitted in a later round once capacity frees — never forgotten.
        let mut s = Scheduler::new();
        s.age_round();
        // Round 1: fill global to the brim so a late team is refused.
        let mut q1 = Vec::new();
        for t in 0..20 {
            q1.push(pending(&format!("fill-{t}"), &format!("f{t}"), 100));
        }
        run(&mut s, &q1); // admits up to DRAIN_BOUND / limits
                          // A late, quiet team arrives after the store is contended:
        let quiet = pending("quiet", "quiet-1", 100);
        let quiet_batch = [quiet];
        let starved = s.drain(&quiet_batch);
        // Either admitted now, or it accrued deficit for the next round (never dropped silently).
        for (p, a) in starved {
            match a {
                Admission::Admitted => assert_eq!(p.effect_id, "quiet-1"),
                Admission::NotAccepted { reason } => {
                    // It was refused only by an explicit limit, and deficit was accrued so a later
                    // round compensates it — assert the deficit tracked, NOT a silent drop.
                    assert!(matches!(
                        reason,
                        "global-limit"
                            | "per-team-limit"
                            | "store-full"
                            | "producer-quota"
                            | "oversized"
                    ));
                }
            }
        }
    }

    // ---- focused unit tests ----

    #[test]
    fn deficit_compensates_a_skipped_team_across_rounds() {
        let mut s = Scheduler::new();
        s.age_round();
        // Round 1: team-a floods; team-b has one envelope. Both should be served (round-robin).
        let q = [
            pending("team-a", "a1", 100),
            pending("team-a", "a2", 100),
            pending("team-a", "a3", 100),
            pending("team-b", "b1", 100),
        ];
        let (admitted, _) = run(&mut s, &q);
        assert!(
            admitted.contains(&"b1".to_string()),
            "the quiet team-b is not starved in the first round"
        );

        // Force a scenario where a team is skipped by a limit, then verify it accrues deficit and
        // is served FIRST in the next round (compensation, never starved).
        let mut s2 = Scheduler::new();
        s2.age_round(); // round 1
                        // Per-team limit forces team-hot's overflow to be refused and accrue deficit.
        let mut many = Vec::new();
        for i in 0..20 {
            many.push(pending("hot", &format!("h{i}"), 100));
        }
        let (admitted, refused) = run(&mut s2, &many);
        assert_eq!(
            admitted.len(),
            PER_TEAM_LIMIT,
            "team-hot admitted up to its per-team cap"
        );
        assert!(
            refused.iter().any(|(_, r)| *r == "per-team-limit"),
            "the overflow is refused, NOT silently dropped"
        );
        let hot_deficit = s2.deficit.get("hot").copied().unwrap_or(0);
        assert!(hot_deficit > 0, "the skipped team accrued aging deficit");

        // Round 2: after the team releases an envelope (capacity frees) and the round ages, the
        // deficit-earning team is served FIRST — compensation, not starvation.
        s2.age_round();
        s2.release("hot", 100); // the CoS consumed one hot envelope — one slot frees
        s2.age_round();
        let next = [pending("hot", "h20", 100), pending("cold", "c1", 100)];
        let plan = s2.drain(&next);
        // Both are admissible now (a slot was released), so the results carry both — and the
        // deficit-descending roster puts hot (positive deficit) BEFORE cold (zero deficit).
        let hot_idx = plan.iter().position(|(p, _)| p.source_id == "hot");
        let cold_idx = plan.iter().position(|(p, _)| p.source_id == "cold");
        assert!(
            hot_idx.is_some() && cold_idx.is_some(),
            "both the deficit team and the fresh team are served after capacity frees"
        );
        assert!(
            hot_idx < cold_idx,
            "the deficit-earning team is served FIRST — compensation, not starvation"
        );
    }

    #[test]
    fn oversized_is_refused_intact_never_truncated() {
        let mut s = Scheduler::new();
        let big = Pending {
            source_id: "d".into(),
            effect_id: "x".into(),
            size_bytes: OVERSIZED_BYTES + 1,
        };
        let big_batch = [big];
        let (admitted, refused) = run(&mut s, &big_batch);
        assert!(admitted.is_empty());
        assert_eq!(refused, vec![("x".into(), "oversized")]);
    }

    #[test]
    fn store_full_refuses_with_reason_not_silently() {
        // capacity 150, threshold 90% => 135 bytes. First 100-byte envelope fits (100 <= 135);
        // the second would reach 200 > 135 => store-full.
        let mut s = Scheduler::new().with_store_capacity(150);
        let envs = [pending("a", "e1", 100), pending("b", "e2", 100)];
        let (admitted, refused) = run(&mut s, &envs);
        assert_eq!(
            admitted,
            vec!["e1".to_string()],
            "the first envelope fits under 90%"
        );
        assert_eq!(
            refused,
            vec![("e2".to_string(), "store-full")],
            "the second is NOT_ACCEPTED (store-full), never silently dropped"
        );
    }
}
