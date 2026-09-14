# Plan: ticket #12 — report return-channel producer (R3)
Epic: #7. Date: 2026-09-13. Status: done.

## Files that change
- src/report.rs — NEW: `report(role, run, record, text)` + `Report` + `ReportError` (+ tests).
- src/lib.rs — register `pub mod report;`.
- src/main.rs — add the `report` verb (seat's own role from $CF_ROLE); extend `--help`.

## Order of work
1. Write report.rs: resolve the seat's OWN binding via `resolve_role`, then delegate to the
   adapter's report mechanism via `messaging_for_adapter(binding.adapter).report_tool`.
2. Register the module; wire `report <event>` into the CLI, role from `$CF_ROLE`.
3. Test, fmt, clippy, smoke test, evidence.

## Validation Strategy
- Unit (`cargo test`): cmux → `cmux-axi`, herdr → `herdr-axi`, follows `binding.adapter` (never the
  surface string), sbox has no return channel and errors loudly, stale-hint resolve error surfaces
  (not swallowed), serialization shape.
- Lint/format: `cargo fmt --check` and `cargo clippy --workspace -- -D warnings`.
- e2e (smoke): `CF_ROLE=dev-1 comms-axi report <event> --json` (default → cmux-axi), a herdr record
  → herdr-axi, and unset `$CF_ROLE` → loud error.
- Coverage target: every branch in `report` + the two error variants exercised (6 tests).

## Proof
`cargo test` → 41 passed, 0 failed. `cargo fmt --check` and `cargo clippy --workspace -- -D warnings`
exit 0. `report` delegates cmux→`cmux-axi`, herdr→`herdr-axi`; sbox errors; unset `$CF_ROLE` errors.

## Risks
- `report` resolves via `resolve_role` — the single resolution source; it does NOT re-derive the
  surface or absorb the producer-side herdr-axi `report` verb (which stays in herdr-axi).
- The report verb is rendered, not executed (local-first), mirroring `send_event`'s `Dispatch`.
- Delegation is by `binding.adapter` (never the surface string), pinned by a test.
