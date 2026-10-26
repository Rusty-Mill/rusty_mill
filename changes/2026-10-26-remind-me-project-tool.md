---
category: Added
changelog: New MCP tool remind_me_project, /api/projects route and a dashboard Projects view.
---
## 2026-10-26 - One surface over project records (ADR-0027 slice 3)

- **Added:** MCP tool `remind_me_project` with a closed action set: `list` (default), `get`, `history`, `proposals`, `confirm`, `reject`, `set`, `revert`, `backfill`. It ships in the `standard` tool profile rather than `core`, because `core` is deliberately held below twenty tools and what leaves is the tool-surface audit's call.
- **Added:** `GET /api/projects` for every read and `POST /api/projects` for every write; a writing action over `GET` is refused with 405. Both the tool and the route call one entry point, `project_tool::call`, so a model and the dashboard cannot be told different things.
- **Added:** a Projects view on the dashboard: a column per stage with each project's status and anchors, a Pending lane that confirms or rejects a proposal, and a list of anything the projection could not read.
- **Known limitations:** `list` reports evidence counts, not ids; the ids come from `get` or `history`. The backfill is reachable through the tool and the route but has no CLI subcommand yet.
