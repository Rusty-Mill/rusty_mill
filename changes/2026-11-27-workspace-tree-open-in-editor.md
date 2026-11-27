---
category: Added
changelog: In the `rusty-mux` workspace tree, `o` on a file opens it in the server's `$EDITOR` in a new block.
---
## 2026-11-27 - TUI workspace tree: open in editor

- **Added:** `o` on a file row of the TUI tree (`e`) closes the tree and splits the block to the right with `sh -c '...' editor ./<path>`, which runs the server's `$EDITOR` (`vi` if unset) on the file. The path is an argument to a fixed script, never shell text, and `./` keeps a name that starts with `-` from reading as an option. A path that is not a file there is said and waits for Enter. Names with a control byte, or over 4096 bytes, are refused. Card #1573, D375/D376, Nano's go "Open in editor" 2026-11-27. No wire change.
- **Known limitations:** needs a server with layouts and `LAUNCH`. The path is relative to the workspace root, and the new block starts in the server's default directory, so it opens the right file only for a block launched there. The editor runs on the server, not the client machine.
