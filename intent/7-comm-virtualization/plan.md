# Plan: comm-virtualization slice (epic #7) — fleet-wide (host + transport)

From `spec.md` (build contract, f048384 — founder amendment: fleet-wide host + transport). Slice: 5 dependency-ordered tickets, one at a time.

## Files that change

| Ticket | Module(s) | What |
|--------|-----------|------|
| #10 | resolve.rs | resolve_role — host-aware resolution (host → transport → adapter + surface); reads the host registry live |
| #11 | adapter.rs, event.rs | TransportKind, HostRef, host + transport-qualified StationBinding, MESSAGING map; Event + send_event |
| #12 | report.rs | return-channel producer (resolve own binding + delegate) |
| #13 | fleet.rs, journal.rs, stream.rs, scheduler.rs, inbox.rs | G1-G5 relocation, move-as-is (host-agnostic) |
| #14 | cf / herdr-axi producer paths | delete cmux-axi / herdr-axi literals, route through comms-axi |

## Order of work

1. **#10 resolve.rs** — host-first resolution, the single source (R2).
2. **#11 adapter.rs + event.rs** — transport + binding contract + emit primitive (R1/R5).
3. **#12 report.rs** — the return channel (R3).
4. **#13 G1-G5 relocation** — move-as-is delivery substrate.
5. **#14 producer re-wiring** — hard cutover, no shims (R3 + R5).

Each ticket is blocked by the previous (`blocked-by:` edges). Dispatch one at a time; exactly one open dispatch per Developer.

## Dependency graph

```
#10 resolve ─▶ #11 adapter+event ─▶ #12 report ─▶ #13 G1-G5 ─▶ #14 re-wiring
```

## Tickets (spec order)

| # | Title | Dev | Reviewer |
|---|-------|-----|----------|
| #10 | resolve_role host-aware resolution source (R2, host-first) | dev-1 | dev-2 |
| #11 | adapter.rs + event.rs: TransportKind + HostRef + StationBinding + send_event (R1/R5) | dev-2 | dev-1 |
| #12 | report return-channel producer (R3) | dev-1 | dev-2 |
| #13 | Relocate G1-G5 substrate (move-as-is) | dev-2 | dev-1 |
| #14 | Re-wire producers through comms-axi (R3 hard cutover) | dev-1 | dev-2 |

## Risks

- **Local-first, contract host-aware.** Only `local-socket` transport is BUILT now; `tailscale` / `ssh` / `herdr-machine` / `sbox-remote` are named `TransportKind` variants but not built (additive adapters later). StationBinding carries `host` + `transport` from day one so remote transports are additive, not a redesign.
- **Forward dependency on fleet::resolve + host registry.** #10 (resolve.rs) reads `fleet::resolve` (G5, relocates in #13) and the host registry (fleet record `host` field). resolve.rs imports from codefactory's merged fleet module until #13 relocates it; #13 flips the import path. Single-source throughout — no second path.
- **Host registry ownership.** The host registry extends the fleet/resolver record with a `host` field; #10 establishes the host-aware contract, #13 carries the record as-is. No duplicate registry.
- **Two-path window.** #14 (re-wiring) must land with/after #13 (relocation); blocked by #13.
- **`host` / `transport` / `route` are new.** The old codefactory StationBinding (surface + session only) is not wire-compatible; #10 / #11 establish the new contract.
- **Hard cutover, no shims.** Moved primitives are deleted from cf / herdr-axi source in the same changeset that re-wires producers (#14). No forwarding shim (a shim is a second comm path).

## Proof

All five tickets closed; `cargo test` (the full suite) green; a manual `comms-axi emit <role> <event>` / `comms-axi report <event>` round-trip observed over the local-socket transport.
