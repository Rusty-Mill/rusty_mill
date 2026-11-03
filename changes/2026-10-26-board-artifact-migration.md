---
category: Added
changelog: A one-shot script moves the Mill Board artifact's cards into rusty_tick, with a dry run and a rollback (ADR-0012, step 5).
---
## 2026-10-26 - Mill Board: artifact-to-rusty_tick migration script (ADR-0012 step 5)

- **Added:** `crates/apps/rusty_mill_board/tools/migrate_artifact_board.py` reads an export of the artifact board's cards and creates one rusty_tick task per card in the `rusty_mill` list: title, notes (summary, prompt and notes as sections), column as `board:` tag, `kind:`/`track:` tags, manual order, and dependencies in the task's `board` document. Every card is checked against rusty_tick's limits before anything is written. Each task carries the tag `migrated:mill-board` and a notes line naming its card, written in the create request, so a re-run finishes an interrupted one without duplicates; a tagged task without exactly one such line stops the run. It is a dry run unless given `--run`. `--rollback --yes` moves every migrated task to the trash (restorable; nothing is purged) and refuses, writing nothing, while a migrated task has a subtask it did not create. Dependencies are deduplicated and must fit one board document.
- **Added:** rusty_tick's `DELETE /tasks/{id}?leaf=true` trashes a task only if it has no subtasks, checked in the same step as the write (409 otherwise), so a client can trash its own task without taking someone else's subtask along. Rollback uses it, and refuses a rusty_tick that does not know it.
- **Changed:** the ADR's rollback is now reversible: the trashed tasks keep their markers, so migrating again means restoring them, or deleting them from the trash by hand, first. The artifact is only read.
- **Known limitations:** standard-library Python, run by hand; its tests run with `python3 -m unittest test_migrate_artifact_board` from `tools/`, not in CI.
