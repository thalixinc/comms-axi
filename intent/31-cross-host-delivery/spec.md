# Spec: cross-host-delivery

From: intent.md. Author: Brainstorm. Status: draft.
Issue: #31. Epic: #31.

## Requirements

### Core build (comms-axi primary)
- **`TransportKind` delivery.** Implement `TransportKind::{ssh, tailscale, herdr-machine}` DELIVERY
  behind `send_event` — today each is enumerated in `src/adapter.rs` with no delivery code; the only
  real transport is local herdr-axi/cmux-axi. `send_event` opens a REAL channel and injects a turn
  into the remote CoS, not just a resolved `Dispatch`.
- **Inbound listener.** A listener on the target host accepts a delivered handoff and enqueues it
  for the target `cof`.
- **Session injection (pull, not push).** The enqueued handoff becomes a NEW TURN in the target
  `cof`'s session via the ALREADY-SHIPPED pull-on-idle + `hasMail` model (epic #19) — NOT eager
  push (no mid-turn interrupt).
- **effect-id dedup.** Reuse the `journal.rs` model: a re-delivery is idempotent, never
  double-injects.
- **Herdr REMAINS the carrier.** comms-axi rides herdr, does not replace it.

### Acceptance
1. Thalixs `cof` ends a turn to `cof` (Christophers-MBP) → it enters the founder's session as a
   turn, no founder relay, no manual SSH read, over herdr as the carrier.
2. Default (unset) = herdr render-only, byte-stable vs today — no flag-day cutover.
3. Old/new version mismatch fails loudly, never half-delivers.
4. Idempotent on effect-id.

## Design

### Delivery path
`send_event(binding, event)` — once `binding.transport` is `ssh`/`tailscale`/`herdr-machine` (not
`local-socket`), it opens a REAL channel to `binding.host` and delivers the envelope. The target
host's inbound listener accepts the delivered handoff and enqueues it for the target `cof`. The
transport is selected by the resolver's `binding.transport` (already in `StationBinding`), never a
surface string the seat types.

### Session injection (pull, not push)
The enqueued handoff is NOT pushed into the target session. It lands in the target station's local
queue and sets `hasMail`; the target's idle heartbeat (the already-shipped S3 wake) sees `hasMail`
and re-wakes, and the station `read`s the queue as one injectable turn. This reuses the epic #19
pull-on-idle model verbatim — no mid-turn interrupt, zero tokens when empty.

### effect-id dedup
Each delivered envelope carries a STABLE effect id. The inbound listener journals the id
(`journal.rs` prepared→admitted→consumed); a re-delivered envelope whose effect id is already
journaled is acknowledged but NOT re-enacted. This is the same effect-level dedup #440 G2 already
models — never exactly-once, but a mid-swap re-send is idempotent.

## Transition plan (hard requirement)

1. **ADDITIVE + default-off.** Cross-host delivery is gated behind `CF_COMMS_MODE=comms` (default
   unset = herdr render-only, byte-stable vs today). The specific transport is selected per-binding
   by the resolver's `transport` field — never a seat-typed surface. Existing factories are
   unchanged until explicitly opted in.
2. **VERSION-GATED handshake.** Delivery requires BOTH ends on a version that speaks it. An
   old↔new mismatch fails loudly and falls back to herdr render — never a silent half-delivery (the
   standing "loud, no silent fork" rule).
3. **CANARY-first, per-factory.** Ship comms-axi v0.3.0 + the thin cf receipt; canary ONE factory
   (the CoS↔Thalixs pair), prove it, then opt in each factory one at a time (`cf update` + flip flag
   + verify; revert = unset). No flag-day cutover. effect-id dedup survives mid-swap.
4. **NO teardown, preserve in-flight.** Crews keep their sessions and queues; the comm layer swaps
   underneath them. effect-id dedup survives a mid-swap re-send (no lost envelope, no double
   injection).

## Areas of concern
- **Cross-repo split.** comms-axi owns transport delivery + listener + session injection.
  codefactory #515 owns the thin cf receipt (accept the inbound turn into the CoS session, wire
  emit). Do NOT spec the cf half here — it is a `Requested-by: thalixinc/codefactory#515` boundary.
- **Herdr-carrier boundary.** comms-axi rides herdr; it must not fork a second comm path, not
  revive cmux, and not replace herdr as the carrier.
- **Transport is delivery, not a new plane.** ssh/tailscale/herdr-machine are additive DELIVERY
  adapters behind the existing `send_event` seam — no new broker, no eager push, no new seat API.
