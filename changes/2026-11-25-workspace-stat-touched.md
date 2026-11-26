---
category: Added
changelog: `rusty-mux-ctl terminal.workspace-stat` and `terminal.workspace-touched` read one entry's size and age and the workspace's `git status`, behind feature bit 57.
---
## 2026-11-25 - Workspace stat and touched-by calls

- **Added:** block methods `terminal.workspace-stat` (kind, size and mtime of one entry under the block's launch directory) and `terminal.workspace-touched` (the directory's `git status` as `XY<TAB>PATH` lines) behind `Features::WORKSPACE_STATUS` (bit 57), with `rusty-mux-ctl` forms and Leased MCP tools. Slice 2 of the workspace explorer (card #1573, D372).
- **Known limitations:** lease holder only and Linux only, as `terminal.workspace-list`; `terminal.workspace-touched` is refused on agent blocks (a repository's filters would run outside the jail); `git` is killed after 4 s and its output is capped at 256 KiB; nothing draws these yet (the TUI tree and GUI panel come in later slices). Wire change: needs the owner gate.
