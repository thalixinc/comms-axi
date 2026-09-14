# Plan: ticket #21 — comms read/drain queue (idempotent, bounded, lossless) — S2
Epic: #19. Date: 2026-09-14. Status: done.

## Files that change
- src/read.rs — NEW: `read(state_dir, role, all) -> ReadResult` + `ReadEvent`/`ReadResult` (serializable) + tests. Dequeues, clears hasMail when empty, renders one injectable turn.
- src/stream.rs — add `Stream::drain_front(limit)` (additive; dequeue front-first, lossless tail).
- src/send.rs — add `QueueStore::drain(limit)` (admit boundary), make `load_queue`/`save_queue` pub, add `clear_has_mail`.
- src/lib.rs — register `pub mod read;`.
- src/main.rs — add `read <role> [--all]` and `drain <role>` (alias for read --all) verbs.

## Order of work
1. Add `Stream::drain_front` (the dequeue primitive).
2. Add `QueueStore::drain` (drain + journal `admit`) and `clear_has_mail`; expose `load_queue`/`save_queue`.
3. Implement `read` (bounded vs `--all`), reusing the S1 queue store + `DRAIN_BOUND`.
4. Wire `read`/`drain` into the CLI; test, fmt, clippy, smoke, evidence.

## Validation Strategy
- Unit (`cargo test`): read drains exactly-once (journal admit dedup); clears hasMail; empty read is a no-op; bounded read drains `DRAIN_BOUND` and preserves the rest (lossless); per-station isolation; serialization shape.
- Lint/format: `cargo fmt --check` + `cargo clippy --workspace -- -D warnings`.
- e2e (smoke): send → read --all (drains, clears marker) → read again (empty no-op) → drain alias.
- Coverage target: every branch in `read`/`QueueStore::drain`/`Stream::drain_front` exercised.

## Proof
`cargo test` → 79 passed, 0 failed (72 prior + 7 read). `cargo fmt --check` and `cargo clippy --workspace -- -D warnings` exit 0. Smoke: send 2 → `read --json` returns both with `has_mail:false` and clears the `.hasmail` marker; a second read returns zero events; `drain` drains the rest.

## Risks
- `drain` = admit (Prepared → Admitted), the expose boundary; `consume` (ack) is a later slice. Drained entries leave the stream but remain in the journal (durable dedup record) — intended, not a leak.
- Bounded cap reuses the relocated `scheduler::DRAIN_BOUND` (20) — the same "one turn" bound the scheduler already enforces.
- The queue/marker contract is S1's (`<state-dir>/queues/<role>.json` + `.hasmail`); read only consumes it, never rewrites the shape.
