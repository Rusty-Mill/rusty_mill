---
category: Added
changelog: Mill Board has a one-off, dry-run-first command that lifts headed body sections into the typed card fields, with a rollback record.
---
## 2026-11-19 - Typed card fields: lift of headed sections (#1582)

- **Added:** `rusty_mill_board --lift-sections` plans, and with `--lift-record FILE` applies, the lift of `## Acceptance` (and its aliases), `## Scope`, `## Why` and `## Summary` into the typed fields. A field moves only when one section names it, the field is empty and the text fits the cap; the rest stay in the body and are listed on stderr.
- **Added:** it drops `## Parent milestone` where it only repeats the card's parent, and turns `## Prerequisites` codes into `related` links when every code names one card of the project. It uses `related`, not `deps`, because a dependency stops a card being taken, which is a decision for a person. `## Execution gate` is left for a project template.
- **Added:** `--undo-lift FILE` puts back each card still holding what the lift wrote, from the saved record of original bodies. A card changed since is skipped and reported, as is one changed between the plan and the write.
- **Known limitations:** nothing is applied by this change; the apply waits for the owner's go, with the board stopped.
