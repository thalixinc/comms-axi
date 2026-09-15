# Plan: cross-host-delivery slice (epic #31)

From `spec.md` (build contract, 7125efe). Slice: 4 core-build tickets, in order.

## Files that change

| Ticket | Module(s) | What |
|--------|-----------|------|
| #32 (a) | src/adapter.rs, src/event.rs | TransportKind delivery behind send_event (ssh/tailscale/herdr-machine) |
| #33 (b) | listener module | inbound listener — accept + enqueue for target cof |
| #34 (c) | wake/injection | session injection via pull-on-idle/hasMail (not eager) |
| #35 (d) | journal.rs | effect-id dedup (idempotent re-delivery) |

## Order of work

1. #32 (a) — TransportKind delivery behind send_event.
2. #33 (b) — inbound listener (accept + enqueue).
3. #34 (c) — session injection via pull-on-idle/hasMail.
4. #35 (d) — effect-id dedup (journal.rs).

## Dependency graph

```
#32 a ─▶ #33 b ─▶ #34 c ─▶ #35 d
```

## Tickets (spec order)

| # | Title | Dev | Reviewer |
|---|-------|-----|----------|
| #32 | TransportKind delivery behind send_event (ssh/tailscale/herdr-machine) | dev-1 | dev-2 |
| #33 | inbound listener — accept + enqueue for target cof | dev-2 | dev-1 |
| #34 | session injection via pull-on-idle/hasMail (not eager) | dev-1 | dev-2 |
| #35 | effect-id dedup (journal.rs model) | dev-2 | dev-1 |

## Transition rules (cross-cutting, baked into every ticket)

- Default-off: gated behind `CF_COMMS_MODE=comms` (unset = herdr render-only, byte-stable).
- Version-gated handshake: old/new mismatch fails loudly + falls back to herdr render, never half-delivers.
- Canary-first per-factory (CoS-Thalixs pair first), then opt in one at a time; revert = unset.
- No teardown, preserve in-flight; effect-id dedup survives mid-swap.

## Risks

- Herdr REMAINS the carrier — comms-axi rides herdr, does not replace it; no second comm path, no cmux revival.
- cf receipt half (codefactory #515) is OUT of scope — comms-axi owns delivery + listener + injection only.
- Transport is delivery, not a new plane — no new broker, no eager push, no new seat API.

## Proof

All four tickets (#32-#35) closed; cargo test (full suite) green; acceptance: a remote cof handoff enters the target session as a turn over herdr (no relay), default-off render-only stays byte-stable, version mismatch fails loudly, re-delivery is idempotent on effect id.
