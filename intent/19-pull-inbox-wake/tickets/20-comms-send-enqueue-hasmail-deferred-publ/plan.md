# Plan: ticket #20 — comms send enqueue + hasMail (deferred publish) — S1
Epic: #19. Date: 2026-09-14. Status: done.

## Files that change
- src/send.rs — NEW: `QueueStore` (per-station file-backed queue) reusing the relocated
  `stream::Stream` (append-ordered envelope body) + `journal::EffectJournal` (prepared ledger);
  `enqueue`, `send_to`, `has_mail`, `queue_path`/`has_mail_path` + tests.
- src/lib.rs — register `pub mod send;`.
- src/main.rs — add the `send` verb (sender from $CF_ROLE; `--state-dir` for the queue root).

## Order of work
1. Define the queue store: `QueueStore { stream, journal, next_seq }` with `enqueue`/`is_empty`/
   `len`/`to_record`/`from_record`.
2. File-backed wrapper: `send_to` = load → enqueue → atomic save → set `hasMail` marker; `has_mail`
   = pure file-stat.
3. Wire `send <role> <event>` into the CLI (deferred publish, no resolve-and-fire).
4. Test, fmt, clippy, smoke test, evidence.

## Validation Strategy
- Unit (`cargo test`): send enqueues + sets hasMail + journals `Prepared` (never `admit`); per-station
  isolation; append order + distinct effect ids; send is deferred (only queue + marker files, no
  session state); empty mailbox has no flag; corrupt queue record is loud; store round-trips;
  missing-key record is loud; journal dedup axis.
- Lint/format: `cargo fmt --check` and `cargo clippy --workspace -- -D warnings`.
- e2e (smoke): `CF_ROLE=dev-1 comms-axi send coordinator "done: PR #9" --state-dir <tmp> --json`
  then stat the queue file + `hasMail` marker; a second send appends with a distinct effect id.
- Coverage target: every branch in `enqueue`/`send_to`/`load_queue`/`save_queue` exercised (9 tests).

## Proof
`cargo test` → 72 passed, 0 failed. `cargo fmt --check` and `cargo clippy --workspace -- -D warnings`
exit 0. `send` prints `{ queued, effect_id, has_mail }`; the queue record carries stream + journal
`Prepared`; the `.hasmail` marker exists.

## Risks
- Hard bans preserved: herdr-bound (no surface flip); no ThalixRuntime webhook/acp; no broker; no
  cmux pub/sub. `send` never resolves or fires — publish only.
- The enqueue shape is explicit for S2: `QueueStore` record (`next_seq` + `stream` + `journal`) and
  the `hasMail` marker path are the contract `read`/`drain` will consume.
- Single writer per station (atomic temp+rename), matching the relocated `inbox`/`state` rule.
