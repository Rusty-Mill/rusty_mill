---
category: Fixed
changelog: rusty_term --gui: Ctrl+= and Ctrl+- zoom the font again instead of typing into the shell.
---
## 2026-11-01 - rusty_term font zoom keys

- **Fixed:** In the `--gui` window, Ctrl+=, Ctrl+Shift+= and Ctrl+- now change the font size. The shortcut lookup had no entry for the `=` and `-` keys, so those chords never matched their default bindings and the presses reached the shell as typed characters. Ctrl+0 already worked.
- **Known limitations:** the numpad + and - keys are still not bound to zoom.
