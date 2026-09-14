# Intent: comm-virtualization
Author: Brainstorm. Status: draft.
Issue: #7. Epic: #7.

## Problem
Two things broke when the fleet moved to herdr, and they are different:

1. **The transport broke** — turn-done stopped being *communicated back*. That is #440
   (re-platform + passive inbox); fixing it restores the closed loop on herdr.
2. **The seat-level coupling is wrong** — every seat is told, in its own session-start text, to
   reach another agent with `cmux-axi send <project> cof "…"`: the seat must know the TOOL, the
   PROJECT token, and the ROLE — three things it should never reason about. When the transport
   changed, that instruction became silently wrong, and *nothing* in the layer absorbed the change.

#440 fixes the broken transport but does not fix the coupling. This epic is the "make it not
recur" half: the seat should declare an EVENT to a ROLE, and the runtime should own
tool/roster/transport resolution, so a future surface/harness change cannot again sever the loop
by invalidating every seat's hard-coded how-to-talk.

## Proposed outcome
Stand up `comms-axi` as the surface-agnostic messaging plane — an agent-addressable messaging
contract over a single comm virtualization layer:

- A seat emits `done: PR #N` (an event) addressed to a ROLE (`coordinator`, `cof`), never a tool.
- The layer resolves, in code: WHO (the role's current binding), HOW (the adapter the resolver
  selected — herdr / cmux / sbox), and ROUTE (the live handle for that binding).
- The seat never names `cmux-axi` / `herdr-axi`, never looks up the roster, never supplies the
  project token, never picks a surface.
- `cos_surface` stops being a transport switch and becomes ONE input the resolver consumes as a
  hint.

Concretely, `comms-axi` becomes a standalone Rust CLI whose core is:

- `resolve_binding` (adapter.rs, generalized) as the single resolution source →
  `StationBinding { adapter, surface_ref, session_id, route }` (R2).
- `send_event(binding, event)` as the one emit primitive over the neutral adapter seam (R1/R5).
- the #440 gates G1–G5 (journal / stream / scheduler / fleet resolver / inbox) re-homed as the
  core delivery substrate it addresses OVER — not re-implements.
- the #463 surface→adapter map folded into `resolve_binding` (surface is a HINT, not a switch, R4).
- the return-channel producer (`report` verb), adapter-selected.

## Affected users and systems
- **Every seat** (agent panes across cmux / herdr / sbox) — they stop hard-coding how-to-talk and
  emit events to roles instead. Execution surfaces themselves are unchanged.
- **Producers in `cf` and `herdr-axi`** re-wired through the layer (R3): session-start text,
  `cf-register` onboarding dispatch, `cf-provision` steering, and the return-channel report producer.
- **Moved primitives** (out of cf/herdr-axi into comms-axi): `resolve_binding` (adapter.rs),
  `send_event`, the #440 G1–G5 modules, the #463 surface→adapter map, the return-channel `report`.
- **The fleet/resolver record** — read live (not a snapshot) so a binding changed mid-run
  reconciles truthfully.

## Constraints
- **Admission safety unchanged** (#440 boundary): comm addressing is NOT attention ownership. The
  CoS still owns WHEN a message enters its context; no producer writes into the human terminal.
- **Not a new broker, not a second inbox, not a cmux revival** — one resolution seam + one
  event-emit primitive over the existing adapters.
- **Backward-compatible seam** (R5): introduced behind the existing neutral adapter seam
  (`adapter.rs`); no fork of a second comm path; `cf team dispatch`/`record` stay the only writers.
- **Sequenced after #440** — addresses over the fixed transport once G1–G5 land; does not
  re-implement the transport.
- **Roster truth** — reads the fleet/resolver record (the same "truthful active-run migration" as
  #440 Gate 5), never a stale snapshot.
- **Out of scope (explicit)**: the producer-side herdr-axi `report` verb itself (separate ticket in
  herdr-axi); cmux/herdr/sbox execution; story/initiative tiers, team-eval, board UI.

## Open questions
1. **Cutover mechanics** — when the moved primitives (`resolve_binding`, `send_event`, G1–G5) land
   in comms-axi, are the cf/herdr-axi copies deleted in the same change, or left as thin shims over
   comms-axi until every producer is re-wired? (R3 "delete the literals" vs R5 "no second path"
   must reconcile.)
2. **CLI surface** — the concrete verbs/args of the `comms-axi` binary (resolve / emit / report)
   and how a producer invokes `send_event`; to be settled in design.
3. **Re-home boundary of `report`** — the return-channel producer moves to comms-axi, while the
   producer-side herdr-axi `report` verb stays in herdr-axi; the exact seam between the two needs
   pinning in design.
