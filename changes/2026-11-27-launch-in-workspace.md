---
category: Added
changelog: A block can be launched in another block's held workspace, for the workspace tree's open-in-editor.
---
## 2026-11-27 - mux: `Launch.workspace_of` (card #1573, D378)

- **Added:** feature bit 58 `LAUNCH_IN_WORKSPACE` and `Launch.workspace_of`. The new block starts in the directory the server holds for the named block, by handle, so a rename of its path changes nothing. Refused without the bit, with a `cwd`, for an unknown block, without the input lease on it, for an agent's block, after a restart and where the directory can't be entered; it never falls back to the configured directory. Offered on Linux only, with `WORKSPACE`.
- **Known limitations:** this establishes the workspace only, not which file a program then opens (file identity is unresolved, D377). Nothing in the TUI uses it yet. `rusty-mux-ctl` has no form for it.
