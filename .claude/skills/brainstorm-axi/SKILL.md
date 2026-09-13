---
name: brainstorm-axi
description: Run AI brainstorming sessions with deterministic state — scaffold, anchor, checkpoint, per-phase protocols. Use when starting, resuming, or persisting a brainstorming session.
user-invocable: false
---

# brainstorm-axi

AXI-compliant CLI that owns the deterministic spine of a brainstorming session:
the anchor (where the work lives), the checkpoint (what state it is in), and the
canonical phase protocols. The CLI is the source of truth — run it; do not trust
memorized flags or restate its protocols:

```sh
brainstorm-axi            # dashboard: sessions + active anchor + next steps
brainstorm-axi --help     # full command reference
```

Four phases: `interview` → `generate` → `score` → `decide`.

Per-session flow:

1. `brainstorm-axi status` — first, always. Tells you the active session, its
   home directory, phase, and the one-home rule.
2. New session: `brainstorm-axi new <slug>`; parallel topic while one is
   active: `brainstorm-axi fork <slug>`.
3. For the current phase: `brainstorm-axi protocol <phase>` — follow its
   output exactly.
4. Before compaction, restart, or handoff: `brainstorm-axi checkpoint`.
   After any of those: `brainstorm-axi resume`.
5. At phase boundaries: `brainstorm-axi lint` — fix what it names.

One-time setup: `brainstorm-axi setup skill` (install this skill) and
`brainstorm-axi setup hooks` (SessionStart/Stop/PreCompact/PostCompact hooks
that keep the anchor re-injected automatically).
