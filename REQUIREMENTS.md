# comms-axi — surface-agnostic agent messaging plane

`comms-axi` is the **communication surface** — orthogonal to execution surfaces (cmux/herdr/sbox
host the *agent*; `comms-axi` owns how every seat *talks* to every other seat and to the CoS).

## Origin / authority

Evolved from the Founder's directive ("make comm its own surface + repo + CLI") applied to the
comm-virtualization layer already specced at codefactory `research/SPEC-COMM-VIRTUALIZATION.md`
(that spec is the design source; this README is its productization).

## The model

Seats **never** name a comm tool, look up a roster, supply a project token, or pick a surface.
They emit an **event to a role** and the runtime resolves everything:

```
seat emits:  { event: "done: PR #N", to: "coordinator" }
resolve_role(role, run, record) -> StationBinding { adapter, surface_ref, session_id, route }
send_event(binding, event)       // adapter-selected, never a typed literal
```

- **Execution surfaces** (cmux, herdr, sbox): host the agent pane, unchanged.
- **comms-axi**: the messaging plane — `resolve_binding` (single source) + `send_event` (one
  emit primitive) + the return channel + the passive inbox drain.

## Requirements (from the spec, R1–R5)

1. **Event-to-role addressing, not tool invocation.** The seat's only comm surface is an event + a
   recipient role.
2. **One resolution layer is the single source** — `resolve_binding` emits a `StationBinding`;
   no verb re-resolves the surface string.
3. **Every producer path goes through the layer** — session-start text, onboarding dispatch,
   steering, the report verb. Delete hard-coded `cmux-axi send …` / `herdr-axi` literals.
4. **Surface is a hint, not a switch** — `cos_surface`/`--surface` is consumed as a preference;
   the resolver validates against the fleet record (truthful active-run migration).
5. **Backward-compatible seam** — sits behind the existing neutral adapter seam; does not fork a
   second comm path; `cf team dispatch`/`record` stay the only writers.

## Boundaries (unchanged from #440)

- **Admission safety**: comm addressing is NOT attention ownership. The CoS still owns when a
  message enters its context; no producer writes into the human terminal.
- **Not a new broker, not a second inbox, not a cmux revival** — one resolution seam + one event
  emit over the existing adapters.
- The #440 passive inbox + repair gates stay in their own epic; `comms-axi` addresses *over* the
  fixed transport.

## Seed (already shipped, becomes comms-axi's starting base)

The primitives that must MOVE here (currently living in `cf`/`herdr-axi`):
- #440 gates G1–G5 (journal / stream / scheduler / fleet resolver / inbox) — pure modules.
- `resolve_binding` (adapter.rs), `send_event`, the return-channel `report` verb, the passive inbox.
- The `#463` surface→adapter map.

## Out of scope

- The producer-side herdr-axi `report` verb itself (separate, herdr-axi).
- cmux/herdr/sbox *execution* (they stay execution surfaces).
- Story/initiative tiers, team-eval, board UI.
