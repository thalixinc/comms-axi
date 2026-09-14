# Plan: pull-inbox-wake slice (epic #19)

From `spec.md` (build contract, 4f38ac7 — founder-authored). Slice: S1-S5, in order.

## Files that change

| Ticket | Module(s) | What |
|--------|-----------|------|
| #20 (S1) | send verb, queue store | comms send enqueue + hasMail (deferred publish), reuse journal/inbox/stream |
| #21 (S2) | read/drain verb | comms read/drain (idempotent, bounded, lossless) |
| #22 (S3) | wake layer (omp/pi heartbeat) | hasMail-gated idle wake (empty = silent no-op) |
| #23 (S4) | cf heartbeat (cross-factory) | hasMail-gated re-wake, Requested-by, held |
| #24 (S5) | adapter.rs (StationBinding) | reserve WakeKind (HeartbeatPoll), no webhook build |

## Order of work

1. S1 (#20) — send enqueues + sets hasMail.
2. S2 (#21) — read/drain pulls the queue.
3. S3 (#22) — heartbeat wakes on hasMail, silent no-op when empty.
4. S4 (#23) — cf heartbeat hasMail-gated re-wake (cross-factory, held).
5. S5 (#24) — reserve WakeKind (HeartbeatPoll).

## Dependency graph

```
#20 S1 ─▶ #21 S2 ─▶ #22 S3 ─┬─▶ #23 S4 (cross-factory, held)
                            └─▶ #24 S5 (scaffold)
```

## Tickets (spec order)

| # | Title | Dev | Reviewer |
|---|-------|-----|----------|
| #20 | comms send enqueue + hasMail (deferred publish) | dev-1 | dev-2 |
| #21 | comms read/drain queue (idempotent, bounded, lossless) | dev-2 | dev-1 |
| #22 | hasMail-gated idle heartbeat wake (omp/pi) | dev-1 | dev-2 |
| #23 | cf heartbeat hasMail-gated re-wake (cross-factory) | dev-2 | dev-1 |
| #24 | reserve WakeKind on StationBinding (HeartbeatPoll) | dev-1 | dev-2 |

## Risks

- **Hard bans preserved.** herdr-bound (no surface flip); no ThalixRuntime webhook/API; no omp acp socket; no central broker; no cmux pub/sub. S5 is scaffold-only (HeartbeatPoll); Webhook is named backlog.
- **S4 cross-factory.** cf heartbeat change is Requested-by: thalixinc/comms-axi#19, owned by the codefactory coordinator; comms-axi does not edit codefactory. Held until the codefactory ticket is filed + built.
- **S4 subsumes the bandaid.** cf quiet-streak backoff is removed when hasMail-gating lands (do NOT keep both paths).
- **FOUNDER NOTE.** S3/S4 use the SIMPLER hasMail-gated wake, NOT the #496 backoff/streaks/ladder (replaced).

## Proof

All comms-axi tickets (#20-#22, #24) closed; cargo test (full suite) green; S4 closed via the codefactory Requested-by ticket. Acceptance: send never injects live; idle+hasMail pulls exactly-once; idle+empty spends zero tokens.
