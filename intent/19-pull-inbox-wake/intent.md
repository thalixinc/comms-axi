# Intent: pull-inbox-wake
Author: Brainstorm. Status: draft.
Issue: #19. Epic: #19.

## Problem
Two bugs, stated precisely (from the founder directive):

1. **The interruption bug.** A producer's message reaches the CoS *eagerly* — it can land mid-turn
   while the founder is typing. Content must never be pushed into an active turn.
2. **The token-eat bug.** The cf heartbeat re-injects a standing "drain the inbox" prompt every N
   minutes *even when the inbox is empty*, so an idle-but-quiet CoS pays a full LLM turn that
   resolves to "nothing to do" every few minutes.

The bandaid for #2 (quiet-streak backoff) already landed in cf `heartbeat.rs`; this epic replaces
it with the correct mechanism (a real `hasMail` signal) so the re-wake is condition-gated, not
merely slowed.

## Proposed outcome
Pull-based inbox wake — pub/sub without the push, without the poll cost:

- Producers **publish** to a station's local queue (enqueue + set `hasMail`); content is never
  pushed into a live session.
- The station **pulls** the queue only when it is idle and ready; events enter context as one turn.
- The wake is a `hasMail` flag folded into the existing idle heartbeat: idle + hasMail → re-wake;
  empty → silent no-op (file-stat, zero tokens).

Publish and pull are decoupled: the producer never decides *when* the recipient reads; the
recipient's *idle* state does. This stops mid-turn interruption.

## Affected users and systems
- **comms-axi** (primary factory, w4A) — S1 (`send` publishes), S2 (`read`/`drain`), S3 (`hasMail`
  heartbeat), S5 (`WakeKind` scaffold). Stays on **herdr** throughout; no surface flip.
- **codefactory** (consumer factory, w48) — S4, the cf heartbeat consumer change, via a
  `Requested-by` cross-factory ticket.
- **The CoS** — the recipient whose mid-turn interruption is fixed, and whose idle-quiet turns stop
  burning tokens.
- **The relocated `journal`/`inbox`/`stream` modules** — already in comms-axi `src/` (from #13);
  reused as the local queue store, not re-written.
- **cf `heartbeat.rs`** — the standing-prompt re-injection, replaced by hasMail-gated re-wake (S4).

## Constraints
- **Surface discipline**: comms-axi stays on **herdr** throughout; no surface flip.
- **Out of scope (explicit, so nobody gold-plates)**: ThalixRuntime webhook/API; omp `acp`
  real-time socket wake; central cross-location broker; cmux pub/sub.
- **Reuse, don't re-write**: the relocated `journal`/`inbox`/`stream` modules back the queue store.
- **`emit`/`resolve` stay intact** (the canary-proven plane); `send` becomes the safe, deferred
  publish verb; `emit` remains the low-level resolve/dispatch for testing.
- **S4 is cross-factory** (`Requested-by: thalixinc/comms-axi#<n>`); the comms-axi coordinator
  files the codefactory issue, does not edit codefactory directly.
- **S5 is scaffold-only**: reserve `StationBinding.wake: WakeKind`; do not build the webhook /
  `acp`-shim instant-wake here.

## Open questions
