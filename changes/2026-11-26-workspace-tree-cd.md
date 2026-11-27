---
category: Added
changelog: In the `rusty-mux` workspace tree, `c` on a directory types a quoted `cd` into the block's shell after a yes.
---
## 2026-11-26 - TUI workspace tree: cd in the block

- **Added:** `c` on a directory row of the TUI tree (`e`) shows the `cd -- '<path>'` it would type and waits for one key. `y` types it into the block (the tree holds the lease) and closes the tree; any other key cancels. The path is single-quoted, and a name or path with a control byte, or over 4096 bytes, is refused. Card #1573, D375/D376. No wire change.
- **Known limitations:** the path is relative to the workspace root, and the client does not know where the shell is now, so the `cd` lands right only while the shell is still at the root where the block started; the shown line is the check. It cannot tell whether a program or a prompt is reading the keys.
