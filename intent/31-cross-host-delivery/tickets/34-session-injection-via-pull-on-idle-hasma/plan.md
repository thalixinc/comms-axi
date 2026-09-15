# Plan: ticket #34 — session injection via pull-on-idle/hasMail (not eager)
Epic: #31. Date: 2026-09-15. Status: done.

## Files that change
- src/inject.rs (new) — `inject(state_dir, role, idle) -> InjectOutcome::{Turn, NoOp}` wires the
  S3 wake + S2 read into one turn; tests.
- src/send.rs — `QueueStore::enqueue_with_id` + `send_to_with_id` (enqueue under a caller-supplied
  STABLE effect id, so the journal dedup axis is the delivery envelope's id, not the local seq).
- src/listener.rs — `accept` enqueues via `send_to_with_id` (the STABLE id), not the local seq.
- src/read.rs — `ReadEvent` gains `effect_id` (carry the dedup axis through the turn).
- src/lib.rs — register `pub mod inject`.
- src/main.rs — `inject <role>` verb (`cmd_inject`), `run` dispatch, USAGE/help.

## Order of work
1. Establish the STABLE axis: `enqueue_with_id`/`send_to_with_id` in send.rs; listener enqueues
   under the delivery envelope's stable id (kept intact so #35 is additive).
2. Carry it through: `ReadEvent.effect_id`.
3. Wire the turn: `inject` = `idle_tick` (S3) → `read(all=true)` (S2).
4. Verb, tests, fmt, clippy, smoke, evidence, PR (base `main` @ 339348e, reviewer dev-1).

## Validation Strategy
- Unit: turn on idle+mail (drains + clears hasMail); no-op on no mail (zero tokens, no queue read);
  non-idle never touched (queue + marker intact); consume exactly once (second inject → no-op);
  turn carries the STABLE effect_id; serialization.
- e2e: `deliver` (cross-host) → `inject` turns the handoff; re-delivery of the same envelope stays
  queued=1 (journal `prepare` dedup already keyed on the stable id).
- fmt/clippy clean; coverage target: every inject branch + the stable-axis propagation.

## Proof
`cargo test --workspace` (115 = 114 lib + 1 bin), `cargo fmt --check`, `cargo clippy --workspace --
-D warnings` exit 0. Evidence under `evidence/validate-*.txt`.

## Risks
- **Pull-not-push** is the hard rule — the injection only fires on idle+mail; a non-idle station is
  never touched (no mid-turn interrupt). Herdr stays carrier (the injection rides the shipped
  pull-on-idle model, forks nothing).
- **Stable axis (dev-1's #35 nit)** — the listener now enqueues under the envelope's STABLE effect
  id, and the turn carries it, so #35's journal dedup is additive (no listener/queue restructure).
  The dedup itself (re-delivery acknowledged-not-re-enacted) already works via `prepare`; #35 only
  verifies/acks it.
- **Consume exactly once** — `inject` drains once (S2 read + journal admit); a second injection
  finds an empty queue (no-op), never a double-injection.
