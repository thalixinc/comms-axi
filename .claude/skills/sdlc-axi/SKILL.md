---
name: sdlc-axi
description: Drive GitHub issues/epics through the AI-native SDLC stages (plan/design/build/test/deploy/maintain) with a committed artifact chain. Use when initializing, linting, advancing, or reporting on an SDLC artifact chain for an issue.
user-invocable: false
---

# sdlc-axi

AXI-compliant CLI that owns the deterministic spine of the AI-native SDLC
artifact chain: scaffolding `intent/<issue>-<slug>/`, linting the playbook's
templates, gating stage advances on committed artifacts, and rendering the
GitHub-issue checklist block. The CLI is the source of truth — run it; do not
trust memorized flags or restate its protocols:

```sh
sdlc-axi list          # every chain + stage
sdlc-axi --help        # full command reference
```

Six stages: `plan` → `design` → `build` → `test` → `deploy` → `maintain`.

Per-issue flow:

1. Epic start: `sdlc-axi init <issue> <slug>` — scaffolds the chain, binds the
   issue, lands at `sdlc:plan`.
2. For the current stage: `sdlc-axi protocol <stage>` — follow its output
   exactly.
3. At each transition: `sdlc-axi lint <issue>` then `sdlc-axi advance <issue>
   <next-stage>` — fix what lint names. Human gates are *named* by the tool but
   recorded in your decision ledger; this tool never approves.
4. Update the issue: `sdlc-axi checklist <issue>` emits the body block — post
   it via gh-axi (sdlc-axi never calls GitHub).

Source of truth split: the repo holds the artifacts (content), the GitHub
issue is the index (state + discussion), this CLI owns the stage transitions.

One-time setup: `sdlc-axi setup skill` (install this skill) and `sdlc-axi
setup hooks` (SessionStart chain digest).
