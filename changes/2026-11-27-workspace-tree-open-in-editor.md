---
category: Added
changelog: In the `rusty-mux` workspace tree, `o` on a file opens it in the server's `$EDITOR` in a new block.
---
## 2026-11-27 - TUI workspace tree: open in editor

- **Added:** `o` on a file row of the TUI tree (`e`) closes the tree and splits the block to the right with `sh -c '...' editor ./<path>`, which runs the server's `$EDITOR` (`vi` if unset) on the file. The path is an argument to a fixed script, never shell text, and `./` keeps a name that starts with `-` from reading as an option. The script is given the size and time the tree read for the file and opens it only if the file there matches both; otherwise it says so and waits for Enter, so a same-named file in another directory never opens. `o` waits until the row's size shows. Names with a control byte, or over 4096 bytes, are refused. Card #1573, D375/D376, Nano's go "Open in editor" 2026-11-27. No wire change.
- **Known limitations:** needs a server with layouts and `LAUNCH`. The client cannot tell the workspace root (the server holds it), so the new block starts in the server's default directory and the file is found only if that is the workspace root; any other directory is refused by the size and time check. Opening it from the true root needs a server-side root (new wire). The editor runs on the server, not the client machine.
