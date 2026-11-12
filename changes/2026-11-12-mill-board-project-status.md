---
category: Added
changelog: Mill Board: project status (active, paused, experiment, archived) gates new cards and work, with a board filter by status.
---
## 2026-11-12 - Mill Board: project status and a status filter

- **Added:** a project is `active`, `paused`, `experiment` or `archived`, kept in `project-status.json` in the settings directory and set from Settings > Repositories (`settings_project_status_set`, admin). The projects file is unchanged.
- **Added:** paused and archived projects refuse `card_add`, forge imports, open cards moved in, claims and starting work (a move into a work column); `card_take` hands out none of their work. Work already under way may finish. Experiments take cards and hand out work.
- **Added:** `board_list` shows projects that take work (active and experiment) by default; `project_status` lists others, and a named `project` is listed whatever its status. `board_projects` reports `status`. The board has a status picker beside the repository picker, starting on Active.
- **Added:** a paused or archived project must be made active before it is removed or renamed.
- **Known limitations:** metrics and the roadmap still count every project. The Inbox and a repository's own project have no status control in Settings.
