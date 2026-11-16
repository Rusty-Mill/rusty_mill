---
category: Added
changelog: rusty_mux_server answers the block method `terminal.list-dir`, a directory listing behind feature bit 47 (C08).
---
## 2026-11-16 - terminal.list-dir block method (C08)

- **Added:** `terminal.list-dir [--path DIR]` lists a directory on the server's host for a client's directory browse: the resolved path, then `KIND<TAB>NAME` lines, directories first, at most 1000 entries and a `+<TAB>N` count of the rest. Needs `Features::DIR_LISTING` (bit 47) and is refused to a watch-only share.
- **Known limitations:** no path allow-list (the server reads as its own user); the GUI command palette that uses it follows in its own PR.
