---
category: Added
changelog: The workspace tree's `o` key opens the selected file in an editor, in a new block that starts in that block's own workspace.
---
## 2026-11-27 - mux-tui: open in editor from the workspace tree (#1573, D378)

- **Added:** `o` on a file starts `$EDITOR` (or `vi`) on it in a new block of the same window, over `Launch.workspace_of` on the block's own connection, which holds the lease. The file goes to the program as an argument, never as shell text. Needs a server with the launch-in-workspace bit; without it, or without the lease, the tree says so and sends nothing.
- **Known limitations:** the new block starts in the tree block's held workspace; that the editor then opens the very file picked is not proven (file identity, D377). It runs the server account's `$EDITOR`, so a remote mux needs one installed. The block's connection now also asks for `LAUNCH` and `LAUNCH_IN_WORKSPACE`.
