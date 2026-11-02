---
category: Added
changelog: rusty_term window pastes on plain Ctrl+V on Windows, as Windows Terminal does; Ctrl+Shift+V still pastes everywhere.
---
## 2026-11-02 - rusty_term pastes on plain Ctrl+V on Windows

- **Added:** in the windowed front-end on Windows, plain Ctrl+V pastes the clipboard (bracketed-paste aware), matching Windows Terminal. Ctrl+Shift+V keeps pasting on every platform. Other platforms leave Ctrl+V to the child, where bash/zsh quoted insert and vim block select use it.
- **Known limitations:** on Windows a child no longer sees a raw ^V by default. Rebinding `paste` in `[keys]` (for example `paste = "Ctrl+Shift+V"`) replaces both default chords and gives ^V back. Vim on Windows also accepts Ctrl+Q for block select.
