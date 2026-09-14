# Plan: ticket #10 — resolve_role host-aware resolution source (R2) — host-first
Epic: #7. Date: 2026-09-13. Status: done.

## Files that change
- Cargo.toml, src/lib.rs, src/main.rs — new lib+bin crate (`comms_axi` / `comms-axi`).
- src/adapter.rs — AdapterKind, TransportKind, HostRef, Route, StationBinding (host+transport-qualified).
- src/fleet.rs — G5 versioned fleet resolver, vendored as-is from codefactory `team/fleet.rs` (relocates in #13).
- src/resolve.rs — `resolve_role` (host-first), the single resolution source (R2), + tests.

## Order of work
1. Vendor G5 fleet.rs (pure, faithful copy).
2. Define the binding contract types in adapter.rs (host + transport from day one).
3. Implement resolve_role: host → transport → adapter+surface, with R4 surface precedence and stale-hint error.
4. Wire the `resolve` CLI verb; `--json` prints the binding.
5. Test, fmt, clippy, smoke test, evidence.

## Validation Strategy
- Unit: `cargo test` — host-first resolution, transport naming (local + 4 remote variants), R4 precedence
  (override > fleet > hint > default), stale-hint error, sbox/adapter/instance edges, serialization shape.
- Lint/format: `cargo fmt --check` and `cargo clippy --workspace -- -D warnings`.
- e2e (smoke): `comms-axi resolve coordinator --json` (default local) and a host-first `--record`
  (tailscale host) — both print a StationBinding carrying `host` + `transport`.
- Coverage target: all paths in resolve.rs + fleet.rs exercised (24 tests).

## Proof
`cargo test` → 24 passed, 0 failed. `cargo fmt --check` and `cargo clippy --workspace -- -D warnings`
exit 0. `comms-axi resolve coordinator --json` prints a binding with `transport: local-socket`; a
tailscale-hosted record prints `transport: tailscale`; a stale `cos_surface` hint exits non-zero with
"stale cos_surface hint".

## Risks
- Forward dependency on `fleet::resolve`: vendored into `src/fleet.rs` now; #13 relocates the
  module as-is and flips the import path. Single source throughout.
- Remote transports are NAMED but not built (local-first); contract is host-aware from day one so
  they stay additive.
- `host`/`transport`/`route` are new vs the codefactory binding — established here, consumed by #11/#12.
