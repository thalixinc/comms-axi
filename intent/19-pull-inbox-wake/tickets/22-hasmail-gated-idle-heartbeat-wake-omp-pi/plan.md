# Plan: ticket #22 — hasMail-gated idle heartbeat wake (omp/pi) — S3
Epic: #19. Date: 2026-09-14. Status: done.

## Files that change
- src/wake.rs — NEW: `WakeAction` (re-wake / no-op), `wake_decision` (pure rule), `idle_tick`
  (file-stat-only hasMail gate) + tests.
- src/lib.rs — register `pub mod wake;`.
- src/main.rs — add the `wake` verb (idle tick, `--state-dir` for the queue root).

## Order of work
1. Define the pure rule `wake_decision(idle, has_mail)`: re-wake iff idle && hasMail.
2. `idle_tick(state_dir, role, idle)`: a single `has_mail` marker stat, no queue parse.
3. Wire `wake <role>` into the CLI.
4. Test, fmt, clippy, smoke test, evidence.

## Validation Strategy
- Unit (`cargo test`): wake_decision total (4 cases); idle+mail re-wakes; idle+empty silent no-op;
  non-idle never wakes (even with mail); idle_tick is file-stat-only (corrupt queue + marker →
  re-wake, corrupt queue + no marker → no-op, both WITHOUT a parse error); WakeAction serializes.
- Lint/format: `cargo fmt --check` and `cargo clippy --workspace -- -D warnings`.
- e2e (smoke): `wake` on an empty dir → `no-op`; `send` then `wake` → `re-wake`.
- Coverage target: every branch of wake_decision + both idle_tick outcomes exercised (6 tests).

## Proof
`cargo test` → 78 passed, 0 failed. `cargo fmt --check` and `cargo clippy --workspace -- -D warnings`
exit 0. `wake` prints `no-op` (empty) / `re-wake` (mail), file-stat only.

## Risks
- SIMPLER hasMail-gated wake, NOT the #496 backoff/streaks/ladder (founder REPLACED it).
- The wake does NOT re-inject; it emits the decision the harness (S4, cf-side) consumes.
- `idle_tick` is infallible by construction (one `exists()` syscall) — no partial-failure path.
