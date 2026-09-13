# Review instructions

Applied to every pull request in this repo. The reviewer seat is never the author; findings inform, the founder merges.

## Passes
Run three passes and tag each finding with its pass:
- Bugs: logic errors, broken edge cases, subtle regressions
- Security: injection risks, authentication gaps, PII in logs
- Compliance: the change matches `spec.md`, `plan.md` and the design principles in `CLAUDE.md`

## What Important means here
Reserve Important for findings that would break behavior, leak data or breach a policy. Style and naming are nits.

## Cap the nits
Report at most five nits per review; summarize the rest as a count.

## Do not report
Generated files and anything CI already enforces.

## Record
Findings go on the PR review — its body starts `## Review — <your seat>` (every seat posts as one login; the seat lives in the review, and `cf sdlc ready` looks for it) — and, per epic, into `intent/<epic>-<slug>/REVIEW.md`. A mistake flagged twice becomes a line in `CLAUDE.md`.
