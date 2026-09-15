# Plan: ticket #33 — inbound listener — accept delivered handoff, enqueue for target cof
Epic: #31. Date: 2026-09-15. Status: done.

## Files that change
- src/listener.rs (new) — `accept` (parse envelope → handshake → enqueue), `Accepted`, `REMOTE_SENDER`, tests.
- src/lib.rs — register `pub mod listener`.
- src/main.rs — `deliver <envelope-json>` verb (`cmd_deliver`), `run` dispatch, USAGE/help.

## Order of work
1. `accept(envelope_json, state_dir)`: parse the #32 `Envelope`, enforce the version-gated handshake, then `send_to` the payload for `envelope.to`.
2. Wire the `deliver` verb into the bin (`cmd_deliver` → `resolve_state_dir` → `accept`).
3. Tests, fmt, clippy, smoke, evidence, PR (base `main` @ f3e13c9, reviewer dev-1).

## Validation Strategy
- Unit: valid v1 envelope enqueues + sets hasMail; version mismatch rejected loudly (nothing enqueued); malformed JSON rejected; pull-not-push (still queued, journal Prepared, hasMail set); per-station (no cross-writes); append order + stable ids.
- e2e: sender's `emit` (CF_COMMS_MODE=comms) envelope byte-compatible → `deliver` accepts → `read` pulls the turn.
- fmt/clippy clean; coverage target: every accept branch (parse, handshake reject, enqueue) plus the verb.

## Proof
`cargo test --workspace` (109 = 108 lib + 1 bin), `cargo fmt --check`, `cargo clippy --workspace -- -D warnings` exit 0. Evidence under `evidence/validate-*.txt`.

## Risks
- **Sender attribution gap (honest).** The #32 envelope carries no `from`; the listener records `REMOTE_SENDER` (`"remote"`) as the stream `source_id` rather than inventing a role. Adding an explicit `from` to the envelope is a follow-up.
- **Shell-transport encoding.** The sender's ssh/herdr argv passes the envelope as one argv token; a payload with spaces can split under the remote shell (a #32/#34 transport-encoding concern, not the listener's).
- **Dedup deferred.** The envelope's STABLE `effect_id` is carried in `Accepted` for #35's journal dedup, but #33 only enqueues via S1 `send_to` (which journals `Prepared` with a local per-station id). #35 adds stable-id dedup on top.
