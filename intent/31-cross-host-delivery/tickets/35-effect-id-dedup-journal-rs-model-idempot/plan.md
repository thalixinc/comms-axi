# Plan: ticket #35 — effect-id dedup (journal.rs model): idempotent re-delivery
Epic: #31. Date: 2026-09-15. Status: done.

## Files that change
- src/send.rs — `Enqueued.duplicate: bool`; `send_to` sets `false`; `send_to_with_id` sets
  `!appended` (the journal `prepare` result); tests.
- src/listener.rs — `Accepted.duplicate: bool` (surfaced from `send_to_with_id`); module doc
  transition-rule 4 now states the guarantee; end-to-end idempotency + stable-key tests.
- src/main.rs — `deliver` human output distinguishes a duplicate re-delivery.
- (No change to journal.rs: `prepare` already returns `false` on a seen id — #35 wires and
  surfaces it end-to-end, it does not reimplement the model.)

## Order of work
1. Surface the dedup signal: `duplicate` on `Enqueued` + `Accepted` (true ⟺ `journal.prepare` saw
   the id already — a re-delivery acknowledged but NOT re-enacted).
2. Wire `deliver` output.
3. Tests: pre-drain + post-drain idempotency; fresh-enqueue flag; stable-id-not-local-seq key.
4. fmt, clippy, smoke (deliver→inject→re-deliver→inject across separate processes), evidence, PR.

## Transition rules baked in (hard)
- **NO teardown, preserve in-flight** — the STABLE FNV-1a effect id is the dedup axis (not the
  local `{role}-{seq}`); a re-delivery of the same id is ACKNOWLEDGED but NOT re-enacted, surviving
  a mid-swap re-send (no lost envelope, no double injection).
- **Herdr stays carrier** — dedup is a property of the journal, orthogonal to transport; it forks
  nothing, replaces nothing.

## Validation Strategy
- Unit: `send_to_with_id` fresh → `duplicate: false`; re-delivery (pre- and post-drain) →
  `duplicate: true` + no re-append; marker invariant held.
- Listener e2e: `accept` → `read` → `accept` (same envelope) → `accept` → queue counts 1,1,0;
  post-drain re-delivery never re-enacts; `inject` after re-delivery is a no-op.
- Stable-key: two distinct payloads → distinct stable ids, both enqueued (no false dedup).
- fmt/clippy clean; coverage target: every `duplicate` branch + the stable-key axis.

## Proof
`cargo test` green (count in evidence), `cargo fmt --check` + `cargo clippy --workspace -- -D
warnings` exit 0; evidence under `evidence/validate-*.txt`.

## Risks
- Exactly-once is NOT promised (per #440 option 13): a crash after exposure but before ack is
  reconciled by dedup; the residual is named, not hidden. #35 delivers the idempotency guarantee,
  not exactly-once.
