---
category: Added
changelog: ADR-0012: rusty_tick becomes the one task store, Mill Board an agent view over it, remind-me links tasks by id.
---
## 2026-10-26 - ADR-0012: one task store

- **Added:** `docs/adr/0012-one-task-store.md`, accepted by the owner on 2026-10-26: Mill Board keeps its rules and API and swaps its store for a rusty_tick adapter, the card-to-task field mapping, how agents pick the next task, the remind-me `tick:<id>` link, and a migration with rollback for the Mill Board artifact's cards. Design only; no code changes.
- **Known limitations:** under this design the board's event log is no longer durable; see the ADR's consequences.
