---
category: Added
changelog: rusty_term window copies on plain Ctrl+C on Windows when text is selected, and still sends ^C otherwise, as Windows Terminal does.
---
## 2026-11-03 - rusty_term copies on plain Ctrl+C on Windows

- **Added:** in the windowed front-end on Windows, plain Ctrl+C copies the selection and clears it, matching Windows Terminal. With nothing selected the key reaches the shell as ^C, so interrupting is unchanged. Ctrl+Shift+C keeps copying on every platform. If the clipboard is missing or refuses the write, the selection is kept for a retry, the key is not sent to the shell, and the error is reported on stderr; Ctrl+Shift+C now reports such failures too instead of ignoring them. Copying also ends a mouse drag that is still in progress, so the selection cannot come back. **Fixed:** once the window handles a shortcut's key press, that key's repeats and release are no longer sent to an app that reads key releases (win32-input-mode, Kitty event types), each held key tracked separately. Holding Ctrl+C after it copied never turns into ^C. The new `copy_or_send` `[keys]` action names this behaviour so it can be moved to another chord.
- **Known limitations:** while text is selected, the first Ctrl+C copies instead of interrupting; a second press interrupts. Other platforms leave Ctrl+C alone.
