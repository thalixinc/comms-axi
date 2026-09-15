# Intent: cross-host-delivery
Author: Brainstorm. Status: draft.
Issue: #31. Epic: #31.

## Problem
comms-axi's `resolve`/`send_event` plane is shipped (local-first), but `send_event` only RESOLVES a
`Dispatch` — it has no DELIVERY code for the remote transports. `TransportKind::{ssh, tailscale,
herdr-machine}` are ENUMERATED in `src/adapter.rs` but unimplemented; the only real transport is
local herdr-axi/cmux-axi. The fleet is now genuinely remote-multi-VM, so "comms between machines"
needs real delivery — open a channel and inject a turn — not just a resolved binding.

## Proposed outcome
Cross-host delivery behind `send_event`: open a real ssh/tailscale/herdr-machine channel to the
target host, and inject the handoff as a NEW TURN in the remote CoS's session — using the
already-shipped pull-on-idle + `hasMail` model (NOT eager push, no mid-turn interrupt) and
`journal.rs` effect-id dedup (a re-delivery is idempotent). **Herdr REMAINS the carrier** —
comms-axi rides herdr, it does not replace it.

## Affected users and systems
- **comms-axi** (primary factory) — `TransportKind` delivery + the inbound listener + session
  injection.
- **codefactory #515** (receipt, cross-repo) — the thin cf side (accept the inbound turn into the
  CoS session, wire emit). NOT specced here.
- **The remote CoS** (target `cof`) — the recipient whose session receives the injected turn.
- **The shipped pull-on-idle/`hasMail` model** (epic #19) and **`journal.rs` effect-id dedup**
  (#440 G2) — reused as the injection + idempotency substrate.

## Constraints
- **Herdr stays the carrier** (the founder's standing constraint): comms-axi rides herdr, it does
  not replace it.
- **Transition plan is a hard requirement**, not an afterthought: ADDITIVE + default-off;
  VERSION-GATED handshake; CANARY-first per-factory; NO teardown / preserve in-flight. (Carried as
  a hard section in spec.md.)
- **cf receipt half is codefactory#515** — out of scope here; comms-axi owns only the delivery +
  listener + injection half.

## Open questions
