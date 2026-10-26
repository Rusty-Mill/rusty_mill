---
category: Changed
changelog: remind_me_search returns compact hits by default (`detail=compact|full`), names budget-trimmed ids, and remind_me_get fetches several ids at once.
---
## 2026-10-26 - remind_me_search: compact results, full text on demand

- **Changed:** `remind_me_search` takes `detail` (`compact` by default, `full` for the previous output). A compact hit is one line: id, category, date, tags, score, marks and the first 160 characters. The token budget is charged at the chosen detail, so the default 800-token budget now fits a full page of hits instead of a few bodies.
- **Added:** a trimmed search lists the dropped ids (`trimmed_ids` in JSON) and says how to fetch them. `token_budget` is now advertised in the tool schema.
- **Added:** `remind_me_get` accepts `memory_ids` (1 to 20) and returns `{memories, missing}`. Its schema no longer marks `memory_id` as required, since either form is accepted.
- **Known limitations:** only the MCP tool defaults to compact. The library, CLI, context brief, saved searches and export keep full detail. Compact mode omits the `refs:` footer and per-memory references; use `detail=full` or `remind_me_get` for them.
