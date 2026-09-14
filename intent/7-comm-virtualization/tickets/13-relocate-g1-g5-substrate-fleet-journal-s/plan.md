# Plan: ticket #13 — Relocate G1-G5 substrate: fleet/journal/stream/scheduler/inbox (move-as-is)
Epic: #7. Date: 2026-09-13. Status: done.

## Files that change
- src/fleet.rs — formalize the #10-vendored G5 copy (doc: re-homed in #13; no re-copy).
- src/journal.rs — NEW: Gate 2, relocated as-is from `codefactory/src/team/journal.rs`.
- src/stream.rs — NEW: Gate 3, relocated as-is from `codefactory/src/team/stream.rs`.
- src/scheduler.rs — NEW: Gate 4, relocated as-is from `codefactory/src/team/scheduler.rs`.
- src/inbox.rs — NEW: Gate 1, relocated as-is from `codefactory/src/team/inbox.rs`.
- src/error.rs — NEW: the `Error`/`Result` dependency inbox.rs imports (`crate::error`), vendored as-is from `codefactory/src/error.rs`.
- src/lib.rs — register `pub mod error/journal/stream/scheduler/inbox;`.

## Order of work
1. Copy the five modules + error.rs byte-identical from codefactory.
2. Vendor `error.rs` — the only cross-crate dependency (inbox.rs's `use crate::error::{Error, Result}`).
3. Register the modules in lib.rs; formalize fleet.rs's re-home note.
4. `cargo fmt` (codefactory's copies are not fmt-clean; comms-axi CI requires `cargo fmt --check`) — formatting only, no logic change.
5. Test, fmt, clippy, evidence.

## Validation Strategy
- Unit (`cargo test`): the relocated modules' existing tests pass UNCHANGED (no edits to any test); behavior identical.
- Lint/format: `cargo fmt --check` + `cargo clippy --workspace -- -D warnings`.
- e2e: `cargo build` (the crate links all nine modules).
- Coverage target: every relocated module's test suite green, zero logic edits (rustfmt reflow only).

## Proof
`cargo test` → 57 passed, 0 failed (35 prior + 22 relocated). `cargo fmt --check` and `cargo clippy --workspace -- -D warnings` exit 0. Only import-path adjustments (inbox.rs already uses `crate::error`, satisfied by vendoring error.rs); no logic change — rustfmt reflow only.

## Risks
- inbox.rs depends on codefactory's `error` module — vendored as `src/error.rs` (move-as-is), not re-implemented.
- codefactory's G1-G5 are not rustfmt-clean; comms-axi CI requires `cargo fmt --check` → run `cargo fmt` (formatting only, semantics preserved).
- fleet.rs was vendored early in #10 and is already the canonical copy; #13 only formalizes its doc note (no re-copy).
