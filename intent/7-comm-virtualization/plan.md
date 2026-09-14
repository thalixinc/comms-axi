# Plan: comm-virtualization slice (epic #7)

From `spec.md` (the build contract, b5d19f2). Slice: 5 dependency-ordered tickets, one at a time.

## Tickets (spec order)

| # | Title | Dev | Reviewer |
|---|-------|-----|----------|
| #10 | resolve_role single resolution source (R2) | dev-1 | dev-2 |
| #11 | send_event + StationBinding + adapter map (R1/R5, #463) | dev-2 | dev-1 |
| #12 | report return-channel producer (R3) | dev-1 | dev-2 |
| #13 | Relocate G1-G5 substrate (move, don't rewrite) | dev-2 | dev-1 |
| #14 | Re-wire producers through comms-axi (R3 hard cutover) | dev-1 | dev-2 |

## Dependency graph

```
#10 resolve ─▶ #11 event+adapter ─▶ #12 report ─▶ #13 G1-G5 ─▶ #14 re-wiring
```

Linear chain, encoded as `blocked-by:` edges (11 by 10, 12 by 11, 13 by 12, 14 by 13).
Dispatch one at a time; exactly one open dispatch per Developer.

## Risks

- **Forward dependency on fleet::resolve.** #10 (resolve.rs) reads `fleet::resolve` (G5), which relocates in #13. resolve.rs imports from codefactory's merged fleet module until #13 lands; #13 flips the import path. Single-source throughout — no second path — but the import must move in #13.
- **Two-path window.** #14 (re-wiring) must land with/after #13 (relocation). If producers re-wire before the relocated substrate is the only copy, a shim or fork opens (R5). #14 blocked by #13 closes this.
- **`route` is new.** StationBinding gains a live `route` (fleet-resolved); the old codefactory binding (surface+session only) is not wire-compatible. #10 establishes the new contract; #11/#12 consume it.
- **Hard cutover, no shims.** Moved primitives are deleted from cf/herdr-axi source in the same changeset that re-wires producers (#14). No forwarding shim (a shim is a second comm path).

## Proof

All five tickets closed; `cargo test` (the full suite) green; a manual `comms-axi emit <role> <event>` / `comms-axi report <event>` round-trip observed.
