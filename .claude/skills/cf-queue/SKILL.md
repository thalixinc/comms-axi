---
name: cf-queue
description: GitHub-issue work queue for AI agents — high-level verbs (add/list/start/done/hold/block/ready) over gh-axi/gh, with GitHub issues as the source of truth. Use when adding, listing, starting, completing, holding, or unblocking tracked work.
user-invocable: false
---

# cf-queue

AXI-compliant wrapper over `gh-axi`/`gh` that turns GitHub issues into a durable work queue. GitHub issues are the single source of truth.

Run the CLI for the always-current surface — do not trust memorized flags:

```sh
cf-queue            # dashboard: queue summary
cf-queue --help     # full command reference
```

Core verbs:

- `cf-queue add <title> [--body …] [--label …] [--assignee …] [--epic <n>] [--requested-by owner/repo#<m>]` — file work (creates an issue; `--epic` writes `Parent epic: #<n>` first and links the sub-issue).
- `cf-queue list [--state queued|in-flight|done|hold|blocked]` — the queue.
- `cf-queue ready` — work that is dispatchable now (open, unassigned, un-held, un-blocked).
- `cf-queue start <n>` — claim it (assign yourself → in-flight).
- `cf-queue done <n> [--pr <url>]` — close it.
- `cf-queue show <n>` — one issue with `epic`, `requested_by`, `artifacts`, `blocked_by` (each edge resolved).
- `cf-queue hold <n> [--kind founder]` / `unhold <n>` — pause / resume (`hold:founder` = waiting on the founder).
- `cf-queue block <n> --by <m>|owner/repo#<m>` / `unblock <n> --by …` — dependency edges, same-repo or cross-repo.

State is derived from the issue: `queued` = open + unassigned · `in-flight` = open + assigned · `done` = closed · `hold` = `hold` label · `blocked` = `blocked-by: #<n>` or `blocked-by: owner/repo#<m>` line in the body with that issue still open (an unresolvable cross-repo edge blocks and is reported under `warnings:`).
