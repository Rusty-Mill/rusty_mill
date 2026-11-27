---
category: Added
changelog: The workspace tree in `rusty-mux` shows each row's size and age and marks files git says changed.
---
## 2026-11-26 - TUI workspace tree: size, age and changed files

- **Added:** the TUI tree (`e`) asks `terminal.workspace-stat` for the row under the cursor and shows its size and age, and asks `terminal.workspace-touched` once to put a letter beside each changed file (`?` untracked, `M`, `A`, `D`) and `*` beside a directory with a change below it. Slice 3 of the workspace explorer (card #1573, D373).
- **Known limitations:** needs a server with `WORKSPACE_STATUS` (bit 57) and Linux; the marks are `git status` only, with no per-agent attribution, and are not shown on agent blocks (the server refuses there, D372); the tree actions come in a later slice.
