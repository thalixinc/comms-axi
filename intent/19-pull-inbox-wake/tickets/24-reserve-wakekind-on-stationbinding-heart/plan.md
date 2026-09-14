# Plan: ticket #24 — reserve WakeKind on StationBinding (HeartbeatPoll, no webhook build) — S5
Epic: #19. Date: 2026-09-14. Status: done.

## Files that change
- src/adapter.rs — add `WakeKind { HeartbeatPoll, Webhook }` (HeartbeatPoll default) + the
  `wake: WakeKind` field on `StationBinding` (+ tests).
- src/resolve.rs — emit `wake: WakeKind::HeartbeatPoll` in `resolve_role`.
- src/event.rs — the test `binding` helper gains `wake: HeartbeatPoll`.

## Order of work
1. Define `WakeKind` (HeartbeatPoll wired; Webhook named/backlog) with `Default` + `as_str`.
2. Add `StationBinding.wake`, defaulting HeartbeatPoll at the single production site (resolve_role).
3. Fix the test fixture; assert serialization emits `wake: heartbeat-poll`.

## Validation Strategy
- Unit (`cargo test`): WakeKind default is HeartbeatPoll; both variants serialize; resolve_role
  emits `wake: heartbeat-poll` in the binding JSON; the event.rs fixture compiles (field added).
- Lint/format: `cargo fmt --check` and `cargo clippy --workspace -- -D warnings`.
- e2e (smoke): `comms-axi resolve coordinator --json` shows `"wake": "heartbeat-poll"`.
- Coverage target: both WakeKind variants named, the default, and the binding serialization (5 tests
  added/changed).

## Proof
`cargo test` → 74 passed, 0 failed. `cargo fmt --check` and `cargo clippy --workspace -- -D warnings`
exit 0. `resolve --json` includes `wake: heartbeat-poll`; Webhook is NAMED but no webhook/acp/socket
code exists (hard bans).

## Risks
- Scaffold only: `Webhook` is a named placeholder for ThalixRuntime; do NOT build any
  webhook/acp/socket/API — it is backlog. HeartbeatPoll is the only wired variant (S3's
  hasMail-gated heartbeat).
- Field addition is a compile-breaking contract change; every StationBinding construction site is
  updated in the same commit (resolve.rs + event.rs fixture).
