# Spec: comm-virtualization

From: intent.md. Author: Brainstorm. Status: draft.
Issue: #7. Epic: #7.

## Requirements

R1–R5 from REQUIREMENTS.md (authoritative), decoded against the moved primitives as they exist in
`codefactory/src/team/` and `herdr-axi/src/` today.

### R1 — Event-to-role addressing, not tool invocation
The seat's only comm surface is `Event { to: Role, payload: String }`. The seat invokes
`comms-axi emit <role> <event>` (or `comms-axi report <event>` for its own turn result). The tool
name, project token, roster index, and surface are runtime concerns the layer supplies — the seat
names none of them. Today that coupling lives in `MessagingAdapter` templates
(`cmux-axi send {PROJ} {TARGET} "{TEXT}"`, `herdr agent prompt {TARGET} "{TEXT}"`) and in the
session-start text; both disappear from the seat's surface.

### R2 — One resolution layer is the single source
`resolve.rs::resolve_role(role, run, record) -> StationBinding` is the ONLY producer of a
`StationBinding`. It generalizes `codefactory/src/team/adapter.rs::resolve_binding`, reading the
same inputs (run, station, adapter kind, allocation) plus the `cos_surface` HINT (R4) and per-team
overrides, and emits `StationBinding { adapter, surface_ref, session_id, route }`. No verb
re-resolves the surface string; `cf team dispatch` / `cf team record`, session-start text,
onboarding, steering, and `report` all consume THIS result.

### R3 — Every producer path goes through the layer
`cf-crew-session-start.sh` / `cf-session-start.sh` stop printing `cmux-axi send <project> cof …`;
they print "when you finish a turn, emit your result (addressed to `coordinator` / `cof`); the
runtime routes it." `cf-register` onboarding dispatch, `cf-provision` steering, and the return
channel all emit events through the layer. Every `cmux-axi send …` / `herdr-axi …` literal is
deleted.

### R4 — Surface is a hint, not a switch
`--surface` / `cos_surface` is consumed by the resolver as a preference, validated against the
fleet record (`fleet::resolve` — #440 Gate 5, the versioned fleet resolver with truthful
active-run migration). Precedence: explicit override > fleet record > `cos_surface` hint > default
(`omp`). A stale hint that contradicts the binding loses loudly (error + diagnostic), never
silently forks.

### R5 — Backward-compatible seam
The layer sits behind the existing neutral adapter seam (`adapter.rs`). It does not fork a second
comm path, does not revive cmux, and does not change the receipt/dispatch spine: `cf team dispatch`
and `cf team record` stay the only writers.

## Design

### Module layout (`comms-axi`, lib + bin crate)
- `resolve.rs` — `resolve_role(role, run, record) -> StationBinding` (R2), generalized from
  codefactory `team/adapter.rs`. The single resolution source.
- `event.rs` — `Event { to, payload }` + `send_event(binding, event)` (R1/R5): the one emit
  primitive; the adapter selected by `binding.adapter`, never a seat-typed literal. It collapses
  today's producer-side verbs (`cmux-axi send`, `herdr agent prompt`, `herdr-axi report`,
  `abridge::send`).
- `adapter.rs` — `AdapterKind` (cmux/sbox/herdr), `StationBinding`, and the `MESSAGING`
  SURFACE→ADAPTER map (the #463 map, folded in: one entry per surface; `omp` default, `herdr`
  explicit; `tool`/`report_tool`/`send`/`report` templates).
- `report.rs` — the return-channel producer: resolves the seat's OWN binding and delegates to the
  adapter's report mechanism (`herdr-axi report` on herdr, `cmux-axi status` on cmux), never
  re-implementing the adapter's transport.
- `fleet.rs` (G5) / `journal.rs` (G2) / `stream.rs` (G3) / `scheduler.rs` (G1/G4) / `inbox.rs`
  (passive inbox) — the #440 gates, moved as-is as the delivery substrate the layer addresses OVER
  (not re-implements). These are already merged in codefactory; the re-home relocates, it does not
  rewrite them.

### CLI surface (the verb/arg contract)
```
comms-axi resolve <role> [--run <id>] [--surface <hint>] [--json]
    # → StationBinding; the diagnostic + single-source verb (R2)

comms-axi emit <role> <event> [--run <id>] [--surface <hint>] [--json]
    # → resolve + send_event(binding, event) (R1/R5); the seat's one comm surface

comms-axi report <event> [--run <id>] [--surface <hint>] [--json]
    # → the return-channel producer, adapter-selected (R3)
```
`--run` defaults to the current run derived from the runtime context (session/env), never a
seat-typed token; `--surface` is the R4 hint. A seat's only invocation is `emit <role> <event>` /
`report <event>` — no tool, no roster, no project token, no surface.

### Open questions (carried forward from intent.md, resolved)

**Cutover mechanics** — when the moved primitives (`resolve_binding`, `send_event`, G1–G5) land in comms-axi, are the cf/herdr-axi copies deleted in the same change, or left as thin shims over comms-axi until every producer is re-wired? (R3 "delete the literals" vs R5 "no second path" must reconcile.)

Resolved: **delete, no shims.** Hard cutover. The moved primitives are RELOCATED — deleted from
cf/herdr-axi source and imported from the comms-axi lib crate — in the SAME changeset that
re-wires the producers. No forwarding shim, because a shim is a second comm path (R5). R3's
"delete the literals" and R5's "no second path" are the same change: one `resolve_binding`/
`send_event` pair, one relocation, zero forks. `cf team dispatch`/`record` remain the only
writers; they consume the comms-axi lib for resolution.

**CLI surface** — the concrete verbs/args of the `comms-axi` binary (resolve / emit / report) and how a producer invokes `send_event`; to be settled in design.

Resolved: `resolve <role>` / `emit <role> <event>` / `report <event>` (+ optional `--run`,
`--surface`, `--json`), as specified above. `resolve_role` is the library form of `resolve`; `emit`
is the seat-facing form of `send_event`; `report` is the return channel. A producer invokes
`send_event` by calling the lib crate (`comms_axi::event::send_event(binding, event)`) — never by
re-deriving the adapter from the surface string.

**Re-home boundary of `report`** — the return-channel producer moves to comms-axi, while the producer-side herdr-axi `report` verb stays in herdr-axi; the exact seam between the two needs pinning in design.

Resolved: comms-axi's `report` is the return-channel PRODUCER (the seat's entry point): it
resolves the seat's own binding and delegates to the adapter's concrete report transport — herdr →
`herdr-axi report`, cmux → `cmux-axi status`. The producer-side herdr-axi `report` verb STAYS in
herdr-axi (out of scope per REQUIREMENTS.md); comms-axi addresses it via the adapter, it does not
absorb or re-implement it.

## Areas of concern
- **Admission safety unchanged.** This layer is transport addressing, NOT attention ownership.
  #440's "CoS owns WHEN a message enters its context" and "no producer writes into the human
  terminal" stay intact; a smarter addressing layer must not grant the producer admission.
- **Scope discipline.** Not a new broker, not a second inbox, not a cmux revival. G1–G5 move as
  the existing delivery substrate; the layer addresses OVER them, it does not re-implement them.
- **Sequencing with #440 / #442.** #440's G1–G5 are already merged to codefactory main, and
  codefactory #442 (the cf-side "comm virtualization" slice) was planned but not built — this epic
  supersedes that in-repo plan with the standalone re-home. The G1–G5 relocation reads the merged
  modules; the producer re-wiring (R3) must land with the relocation so no window has two paths.
- **Roster truth (G5).** The resolver reads `fleet::resolve` (the versioned fleet resolver), not a
  frozen snapshot, so a binding changed mid-run reconciles truthfully (truthful active-run
  migration). `route` — the live handle — is read from the fleet record, never derived statically.
- **`route` is new.** The current `StationBinding` (codefactory) carries `surface` + `session` but
  no live `route`; comms-axi's binding contract adds `route` as the fleet-resolved live handle,
  with `surface`→`surface_ref` and `session`→`session_id` to match the SPEC contract.
