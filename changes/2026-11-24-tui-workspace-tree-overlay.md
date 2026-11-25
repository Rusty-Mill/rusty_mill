---
category: Added
changelog: The mux TUI opens a read-only workspace tree of the focused block with the prefix key `e`.
---
## 2026-11-24 - TUI workspace tree overlay (card #1573)

- **Added:** prefix `e` opens a read-only tree of the focused block's workspace over its pane, listing it with `terminal.workspace-list` (D337, D349): j/k or arrows move, Enter or `l` opens a directory (read once, on demand), `h` goes up, `q` or Esc leaves. It needs the input lease and a server that offers the `workspace explorer` feature (Linux today); otherwise the key rings the bell. A refusal shows as one line. No wire change, and no actions yet (copy path, cd in the block, open in an editor, stat and touched-by each need their own go).
- **Changed:** a block's own connection now asks for the existing `block calls` and `workspace explorer` feature bits (no new bit, D349) and sends the listing call itself, since the server authorises a call by the connection that is attached and holds the lease. Losing the lease closes the tree.
