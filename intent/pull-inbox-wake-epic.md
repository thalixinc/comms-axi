# Epic — Pull-based inbox wake (pub/sub without the push, without the poll cost)

**Primary factory:** comms-axi (w4A)
**Consumer factory:** codefactory (w48) — via `Requested-by` cross-factory ticket, per the boundary rule
**Surface discipline:** comms-axi factory stays on **herdr** throughout. No surface flip.
**Authority:** founder directive — "the agent window must *pull* its inbox instead of a producer *pushing* into it mid-turn" (the CoS interruption bug), plus "don't burn tokens when there's nothing to do."

---

## Problem (the two things this fixes, stated precisely)

1. **The interruption bug.** Today a producer's message reaches the CoS *eagerly* — it can land mid-turn while the founder is typing. The fix: producers **publish** to a station's local queue; the station **pulls** the queue only when *it* is idle and ready. Content is never pushed into an active turn.

2. **The token-eat bug.** The cf heartbeat re-injects a standing "drain the inbox" prompt every N minutes *even when the inbox is empty*, so an idle-but-quiet CoS pays a full LLM turn that resolves to "nothing to do" every few minutes. The fix: the re-wake fires **only when the queue is non-empty** (a `hasMail` flag).

The bandaid for #2 (quiet-streak backoff) already landed in cf `heartbeat.rs`; this epic replaces it with the *correct* mechanism (a real `hasMail` signal) so the re-wake is condition-gated, not merely slowed.

---

## The model

```
producer (seat A):  comms send to=<role> "<event>"
     └─ writes event to the recipient station's LOCAL queue, sets hasMail=1   (publish; NOT into context)

recipient (seat B, epoch N):  when IDLE, the wake layer notices hasMail=1
     └─ comms read (or the heartbeat's idle tick) PULLS the queue, clears hasMail
     └─ events enter B's context as one turn — B drains, acts, stops
```

Key property: **publish** and **pull** are decoupled. The producer never decides *when* the recipient reads; the recipient's *idle* state does. This is what stops mid-turn interruption.

---

## Scope of this epic (in order)

### S1 — `comms send` publishes to a local queue (deferred, not eager)
- Add a local per-station queue store (file-backed, under the station's state dir — reuse the relocated `journal`/`inbox`/`stream` modules already in comms-axi, currently unsharpened).
- `comms send <role> <event>` = **enqueue** + set the recipient's `hasMail` flag. It does NOT resolve-and-fire into a live session.
- Keep the existing `comms emit`/`resolve` (the canary-proven plane) intact; `send` becomes the *safe, deferred* publish verb; `emit` remains the low-level resolve/dispatch for testing.

### S2 — `comms read` / `comms drain` pulls the queue
- `comms read <role> [--all]` = dequeue the recipient's pending events, clear `hasMail`, render them as a single injectable turn.
- Idempotent + bounded: reading an empty queue is a no-op; a partial drain preserves un-drained events (no loss).

### S3 — the wake is a `hasMail` flag, folded into the existing idle heartbeat
- The wake layer is **per-harness**, but for omp/pi (priority 1 & 2) the mechanism is: **the idle heartbeat checks `hasMail` instead of always re-injecting.**
- `hasMail == true && idle` → re-wake (the station then `read`s). `hasMail == false` → silent no-op (a file-stat only, zero tokens).

### S4 — cf-side consumer change (cross-factory, `Requested-by`)
- Replace the cf heartbeat's unconditional `STANDING_PROMPT` re-injection with: **re-inject only when the station's queue is non-empty** (consult the `hasMail` flag comms-axi writes), else no-op.
- This *subsumes* the bandaid (quiet-streak backoff) once live — the backoff becomes an obsolete fallback, removed cleaner (do NOT keep both in one path).

### S5 — WakeKind scaffold (deferred, NOT this epic's build)
- Reserve the `StationBinding.wake: WakeKind` field shape (`HeartbeatPoll` for omp/pi now; `Webhook` for ThalixRuntime later) so the webhook/`acp`-shim instant-wake is additive, not a re-design.
- **ThalixRuntime webhook + omp `acp` shim are BACKLOG** — do NOT build them here. This epic is heartbeat-flag pull only.

---

## Out of scope (explicit, so nobody gold-plates)

- **ThalixRuntime webhook / API** — different harness, not wired in; separate backlog epic.
- **omp `acp` / real-time socket wake** — deferred; the 60s-idle-drain latency is accepted for now. If it proves too slow, that's a follow-on.
- **Central cross-location broker** — not yet. Start local-per-station queue; scale via host/transport (already in the binding) *later* when the fleet is actually remote-multi-VM.
- **cmux surface** — local-only, sunset later. Do not new-build cmux pub/sub.

---

## Acceptance (what "done" means)

1. `comms send coordinator "done: PR #N"` enqueues the event and sets the coordinator's `hasMail`; it does **not** inject into any live session.
2. An idle station with `hasMail=1`, on its next idle tick, `read`s the queue and gets exactly the pending events (once), then `hasMail` clears.
3. An idle station with `hasMail=0` **spends zero LLM tokens** on its tick (file-stat only, silent no-op).
4. The CoS no longer has a producer message land mid-turn: content enters context only via the idle pull.
5. cf's heartbeat re-wake is gated on the same `hasMail` flag (cross-factory `Requested-by` ticket, cf-side PR).

---

## Cross-factory mechanics (how the coordinator drives this)

- **Primary: comms-axi coordinator (w4A).** Files the S1–S3 slices on comms-axi and dispatches to w4A devs.
- **For S4 (the cf heartbeat change):** the comms-axi coordinator files a **codefactory** issue with `Requested-by: thalixinc/comms-axi#<this-epic-issue>` and `blocked-by:` edges correctly set; the codefactory coordinator owns that ticket's build/merge. The comms-axi coordinator does NOT edit codefactory directly.
- **The CoS (proxy) role:** routing + approval + merge-gate policy only. The CoS does not hand-write product code across the boundary.
- **Report** progress through the return channel after each slice (S1 → S5) with the merge SHA + test count.
