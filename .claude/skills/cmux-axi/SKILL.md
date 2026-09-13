---
name: cmux-axi
description: Provision and steer a crew of cmux terminal sessions (coordinator, brainstorm, planner + N developers) for AI agents. Use when creating, messaging, reading, or tearing down cmux workspaces for agent crews.
user-invocable: false
---

# cmux-axi

AXI-compliant wrapper over `cmux` that provisions a crew of agent sessions and lets you message them by *name* instead of by workspace/pane/surface refs.

The CLI is the source of truth — run it for the always-current surface; do not trust memorized flags:

```sh
cmux-axi            # dashboard: current fleet map + next steps
cmux-axi --help     # full command reference
```

Core commands:

- `cmux-axi provision <project> [--layout <name>]` — create the crew (one workspace, N surfaces) in a layout template (default `3by2`: Coordinator | Planner | Brainstorm over two developer panes; `2by2` is the old quad).
- `cmux-axi layout list` / `layout show <name>` — the layout templates (structure only; the crew is seated into it).
- `cmux-axi layout create <name> --rows 3,2 | --from-workspace <ref>` / `layout rm <name>` — your own templates.
- `cmux-axi provision <project> --spec crew.json` — seat an arbitrary crew (`seats: [{role, slot?, title?, command?}]`) into a layout.
- `cmux-axi status [--project <p>]` — fleet map, drift-flagged.
- `cmux-axi send <project> <role> "<text>"` — type-and-submit to a role's surface.
- `cmux-axi read <project> <role>` — read a role's screen.
- `cmux-axi dev add <project>` / `dev rm <project> <dev-id>` — disposable developers.
- `cmux-axi teardown <project>` — remove the whole crew.

Roles: `coordinator`, `brainstorm`, `planner`, `dev-N`.

One-time setup: `cmux-axi setup skill` (install this skill) and `cmux-axi setup hooks` (install a SessionStart hook that surfaces the live fleet map at the start of every session).
