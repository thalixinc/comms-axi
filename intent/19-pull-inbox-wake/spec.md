# Spec: pull-inbox-wake

From: intent.md. Author: Brainstorm. Status: draft.
Issue: #19. Epic: #19.

## Requirements

Functional requirements transcribed faithfully from the founder spec
(`intent/pull-inbox-wake-epic.md`, authoritative). Slices S1–S5, in order:

### S1 — `comms send` publishes to a local queue (deferred, not eager)
- Add a local per-station queue store (file-backed, under the station's state dir — reuse the
  relocated `journal`/`inbox`/`stream` modules already in comms-axi).
- `comms send <role> <event>` = **enqueue** + set the recipient's `hasMail` flag. It does NOT
  resolve-and-fire into a live session.
- Keep the existing `comms emit`/`resolve` (the canary-proven plane) intact; `send` becomes the
  *safe, deferred* publish verb; `emit` remains the low-level resolve/dispatch for testing.

### S2 — `comms read` / `comms drain` pulls the queue
- `comms read <role> [--all]` = dequeue the recipient's pending events, clear `hasMail`, render
  them as a single injectable turn.
- Idempotent + bounded: reading an empty queue is a no-op; a partial drain preserves un-drained
  events (no loss).

### S3 — the wake is a `hasMail` flag, folded into the existing idle heartbeat
- The wake layer is **per-harness**, but for omp/pi (priority 1 & 2) the mechanism is: **the idle
  heartbeat checks `hasMail` instead of always re-injecting.**
- `hasMail == true && idle` → re-wake (the station then `read`s). `hasMail == false` → silent no-op
  (a file-stat only, zero tokens).

### S4 — cf-side consumer change (cross-factory, `Requested-by`)
- Replace the cf heartbeat's unconditional `STANDING_PROMPT` re-injection with: **re-inject only
  when the station's queue is non-empty** (consult the `hasMail` flag comms-axi writes), else no-op.
- This *subsumes* the bandaid (quiet-streak backoff) once live — the backoff becomes an obsolete
  fallback, removed cleaner (do NOT keep both in one path).

### S5 — WakeKind scaffold (deferred, NOT this epic's build)
- Reserve the `StationBinding.wake: WakeKind` field shape (`HeartbeatPoll` for omp/pi now;
  `Webhook` for ThalixRuntime later) so the webhook/`acp`-shim instant-wake is additive, not a
  re-design.
- **ThalixRuntime webhook + omp `acp` shim are BACKLOG** — do NOT build them here. This epic is
  heartbeat-flag pull only.

### Acceptance
1. `comms send coordinator "done: PR #N"` enqueues the event and sets the coordinator's `hasMail`;
   it does **not** inject into any live session.
2. An idle station with `hasMail=1`, on its next idle tick, `read`s the queue and gets exactly the
   pending events (once), then `hasMail` clears.
3. An idle station with `hasMail=0` **spends zero LLM tokens** on its tick (file-stat only, silent
   no-op).
4. The CoS no longer has a producer message land mid-turn: content enters context only via the idle
   pull.
5. cf's heartbeat re-wake is gated on the same `hasMail` flag (cross-factory `Requested-by` ticket,
   cf-side PR).

## Design

### The model (publish / pull decoupled)
```
producer (seat A):  comms send to=<role> "<event>"
     └─ writes event to the recipient station's LOCAL queue, sets hasMail=1   (publish; NOT into context)

recipient (seat B, epoch N):  when IDLE, the wake layer notices hasMail=1
     └─ comms read (or the heartbeat's idle tick) PULLS the queue, clears hasMail
     └─ events enter B's context as one turn — B drains, acts, stops
```
Publish and pull are decoupled: the producer never decides *when* the recipient reads; the
recipient's *idle* state does. This is what stops mid-turn interruption.

### Mechanism
- **Local queue store** — file-backed, per-station, under the station's state dir; built on the
  relocated `journal`/`inbox`/`stream` modules (moved as-is in #13, now sharpened here).
- **`hasMail` flag** — a file-stat signal (true iff the queue is non-empty), not a prompt.
- **Idle heartbeat** (omp/pi) — checks `hasMail`: idle + hasMail re-wakes (the station then
  `read`s); empty is a silent no-op with zero tokens.

## Areas of concern
- **Surface discipline (herdr-stays).** comms-axi factory stays on **herdr** throughout; no surface
  flip. cmux is local-only, sunset later — do NOT new-build cmux pub/sub.
- **Out of scope — ThalixRuntime webhook/API.** Different harness, not wired in; separate backlog
  epic. Do not build here.
- **Out of scope — omp `acp`/real-time socket wake.** Deferred; the 60s-idle-drain latency is
  accepted for now. If it proves too slow, that's a follow-on.
- **Out of scope — central cross-location broker.** Not yet. Start local-per-station queue; scale
  via host/transport (already in the binding) *later* when the fleet is actually remote-multi-VM.
- **Cross-factory boundary (S4).** The comms-axi coordinator files a codefactory issue with
  `Requested-by: thalixinc/comms-axi#<n>` and `blocked-by:` edges correctly set; the codefactory
  coordinator owns S4's build/merge. comms-axi does not edit codefactory directly.
- **S5 is scaffold-only.** Reserve `WakeKind`, do NOT build the webhook/`acp`-shim instant-wake;
  keep this epic heartbeat-flag pull only (no gold-plating).
