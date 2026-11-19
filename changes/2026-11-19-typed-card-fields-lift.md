---
category: Added
changelog: Mill Board has a one-off, dry-run-first command that lifts headed body sections into the typed card fields, with a rollback record.
---
## 2026-11-19 - Typed card fields: lift of headed sections (#1582)

- **Added:** `rusty_mill_board --lift-sections` plans, and with `--lift-record FILE` applies, the lift of `## Acceptance` (and its aliases), `## Scope`, `## Why` and `## Summary` into the typed fields. A field moves only when one section names it, the field is empty and the text fits the cap; the rest stay in the body and are listed on stderr. An `## Out of scope` section joins `scope` as `Out of scope: ...`, so readers never take excluded items for included ones; a wrapped `## Summary` becomes one line, and a multi-paragraph one stays in the body.
- **Added:** it drops `## Parent milestone` where it only repeats the card's parent, and turns `## Prerequisites` codes into `related` links when every code names one card of the project. It uses `related`, not `deps`, because a dependency stops a card being taken, which is a decision for a person. `## Execution gate` is left for a project template.
- **Added:** `--undo-lift FILE` puts back each card still holding what the lift wrote, from the saved record of original bodies. A card changed since is skipped and reported, as is one changed between the plan and the write.
- **Changed:** only the blank lines at each cut are tidied; the rest of a body, code fences included, is left byte for byte.
- **Changed:** with `--db`, the dry run reads the file read-only and immutable (`sqlite::read_cards`): it never creates the file, runs no migration and leaves no `-wal` or `-shm` beside it, and it refuses a missing file, a schema that is not this build's, or sidecars that show a running board.
- **Changed:** the record's directory is synced as well as the file before any card is written. A lift half-written by rusty_tick (body saved, fields not) is finished by a second apply and reversed by `--undo-lift`, and so is a half-failed undo. The dry run resolves a relative `--db` path once and also refuses a `-journal` beside the file. Headings indented up to three spaces end sections, and a resolved `## Prerequisites` loses only its codes line.
- **Known limitations:** nothing is applied by this change; the apply waits for the owner's go, with the board stopped.
