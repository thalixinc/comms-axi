---
name: cf
description: The codefactory crew tools by role — run `cf skill --role $CF_ROLE` for your card (what each seat runs, by stage and artifact) and `<tool> --help` for flags. Use at session start and whenever unsure which verb a seat owns.
user-invocable: false
---
# cf — your tools by role

Your seat is `$CF_ROLE` (a crew pane has it set; `cof` is the Chief of Staff). Print your card:

```sh
cf skill --role $CF_ROLE        # TOON tools[N]{tool,verbs,stage,artifact,when}; --json, --markdown
cf skill --role all             # every seat, when you route work to another
```

Rules of thumb the card assumes:
- **Do not memorise flags.** Run `<tool> --help`; the CLI is the source of truth and this card names verbs only.
- **The `never` rows are hard boundaries** (a stage move is the Planner's, the ledger is the Chief of Staff's, `cf ask` is never the Chief of Staff's).
- **The issue is the record** (`cf-queue` over GitHub issues; `cf sdlc` for the artifact chain and the board). Questions for the founder go through `cf ask <n> --q …` (rule 8), never chat.
- The full matrix as a document: `docs/ROLE-TOOLS.md` in the codefactory repo (generated from the binary).
