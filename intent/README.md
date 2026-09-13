# intent/ — the artifact chain

Every epic in this repo has a chain here; the GitHub issue is the index, the board is the view, this folder holds the content (crew rule 9).

```
intent/
  <epic>-<slug>/
    intent.md  spec.md  plan.md   # stages plan → design → build (sdlc-axi templates)
    evidence/                     # stage test: the epic's roll-up (full suite run)
    REVIEW.md                     # stage deploy: review findings per PR
    .sdlc                         # sdlc-axi state — never hand-edited
    tickets/<n>-<slug>/
      plan.md                     # the ticket's plan: Files / Order / Validation Strategy / Proof / Risks
      evidence/                   # validate-*.txt, screenshots, implementation-report.md
      rca.md                      # bugs only
```

The five verbs (run in this checkout; each writes the chain, the issue and the board together):

| Verb | Who | What |
|---|---|---|
| `cf sdlc init <epic> <slug>` | Brainstorm, first act of an epic | scaffolds the chain, sets `sdlc:plan`, writes the issue's checklist block |
| `cf sdlc advance <epic> <stage>` | Planner only | moves the stage after the gate (a Chief-of-Staff ledger decision or the acceptance comment), swaps the `sdlc:*` label, rewrites the block, sets the board Stage |
| `cf sdlc checklist <epic>` | anyone, after a commit | rewrites the checklist block between the markers |
| `cf sdlc ticket <n>` | Developer, at take | creates `tickets/<n>-<slug>/`, writes the `Artifacts:` line, puts the ticket on the board |
| `cf sdlc lint <epic>` | anyone | sdlc-axi lint plus placeholders, open questions, Proof, Validation Strategy |

Stage protocols: `sdlc-axi protocol <stage>`. Review policy: `REVIEW.md` at the repo root. Verify commands: `CLAUDE.md`, "Verifying your work".
