---
category: Fixed
changelog: rusty-mux (TUI) floats: a closed window takes its floats with it, a held focus cycle keeps the keys typed after it, and a float edge no longer leaves half of a wide character.
---
## 2026-11-17 - rusty_mux TUI float fixes (C42)

- **Fixed:** closing a window that has floats no longer leaves them on screen; `Ctrl-] n` typed while a move is under way is replayed with the keys after it; a float border that cuts a double-width character blanks the other half so rows keep their width.
