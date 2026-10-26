---
category: Added
changelog: Mill Board can keep its cards in rusty_tick (`--tick-url`, `RUSTY_TICK_TOKEN`): tasks tagged `agent` are the board (ADR-0012, step 3).
---
## 2026-10-26 - Mill Board: rusty_tick as the card store (ADR-0012 step 3)

- **Added:** `--tick-url URL` with the token in `RUSTY_TICK_TOKEN` runs the board over rusty_tick instead of `--store`. The board is the tasks tagged `agent`: column and claim are `board:`/`claim:` tags, the project is the task's list (the Inbox is `default`), comments are rusty_tick comments with their author, and dependencies, the forge link and the execution link are one `board` document per task. Every write is a compare-and-swap on the task's version, and a document write is also fenced by that version, so the transition rules hold as before. An edit the owner makes in rusty_tick is what the board reads next.
- **Changed:** within a queue, `card_take` orders equal priorities by the manual order (rusty_tick's `sortOrder`) before age. A card imported from an issue gets an id derived from the issue number, so a retried import never makes a second card, and the sync skips an issue it imported before instead of failing.
- **Known limitations:** needs a rusty_tick with the `board` document kind and `If-Task-Match` (ADR-0012 step 1). With rusty_tick as the store, events are kept in memory only (a restart starts a new log), actor names must be lowercase, card tags come back lowercase, and `--backup-dir` is refused (rusty_tick holds the data).
