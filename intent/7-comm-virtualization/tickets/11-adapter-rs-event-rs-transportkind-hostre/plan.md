# Plan: ticket #11 — adapter.rs + event.rs: TransportKind + HostRef + StationBinding + send_event (R1/R5)
Epic: #7. Date: 2026-09-13. Status: done.

## Files that change
- src/adapter.rs — add the MESSAGING SURFACE→ADAPTER map (fold #463): `MessagingAdapter`, `MESSAGING`, `messaging_for`, `messaging_for_adapter` (+ tests). `TransportKind` / `HostRef` / `StationBinding` landed in #10 — EXTENDED, not duplicated.
- src/event.rs — NEW: `Event { to, payload }` + `send_event(binding, event)` + `Dispatch` + `SendError` (+ tests).
- src/lib.rs — register `pub mod event;`.
- src/main.rs — add the `emit` verb (resolve + send_event); refactor the shared `--run`/`--surface`/`--record`/`--json` parsing and record loading.

## Order of work
1. Fold the #463 SURFACE→ADAPTER map into adapter.rs (`MESSAGING` + `messaging_for` + `messaging_for_adapter`).
2. Add event.rs: `Event` + `send_event` dispatching by `binding.adapter` (never a seat-typed literal) + `Dispatch`/`SendError`.
3. Register the module; wire `emit` into the CLI (resolve + send_event), with shared flag/record parsing.
4. Test, fmt, clippy, smoke test, evidence.

## Validation Strategy
- Unit (`cargo test`): `send_event` routes to the right adapter (cmux vs herdr) and follows `binding.adapter` over the surface string; sbox has no messaging template and errors loudly; MESSAGING map lookup (surface→adapter, unknown→omp default, one entry per surface, adapter→tool); `TransportKind` name round-trip and local-socket-is-built.
- Lint/format: `cargo fmt --check` and `cargo clippy --workspace -- -D warnings`.
- e2e (smoke): `comms-axi emit <role> <event> --json` (default local → cmux), a herdr record → herdr, an sbox record → loud error.
- Coverage target: every branch in `send_event` / `messaging_for` / `messaging_for_adapter` exercised.

## Proof
`cargo test` → 35 passed, 0 failed. `cargo fmt --check` and `cargo clippy --workspace -- -D warnings` exit 0. `comms-axi emit coordinator "PR #9 done" --json` prints a Dispatch `{ adapter: cmux, tool: cmux-axi, target: cf-subway-default:coordinator, text: "PR #9 done" }`; a herdr record prints `tool: herdr`; an sbox record exits 1 with "adapter Sbox has no messaging entry in the MESSAGING map".

## Risks
- Dispatch by adapter (not surface) is load-bearing: an explicit `instances[role].adapter: herdr` over an `omp` surface must still route herdr — covered by `send_event_follows_binding_adapter_not_surface_string`.
- Sbox has no messaging template in the folded map (its send transport is the sandbox adapter's concern); `send_event` errors loudly rather than silently re-routing through cmux.
- `{PROJ}` in the cmux `send` template is NOT rendered by `send_event` (the seat names no project; it is the runtime/transport's concern) — the `Dispatch` carries tool/target/text, which is the collapsed seam.
