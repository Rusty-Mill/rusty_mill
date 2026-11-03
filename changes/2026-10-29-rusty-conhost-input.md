---
category: Added
changelog: rusty_conhost takes terminal input: input records, cooked and raw ReadConsole, and Ctrl+C/Ctrl+Break, so interactive cmd works on our own console host.
---
## 2026-10-29 - rusty_conhost: console input

- **Added:** host PR 3 of the console-host plan
  (`docs/projects/term/conhost/design.md` §10). Terminal input, plain VT
  or win32-input-mode, becomes input records; GetConsoleInput (read and
  peek), ReadConsole (cooked and raw), RAW_READ, RAW_FLUSH,
  GetNumberOfInputEvents and FlushConsoleInputBuffer serve them, and
  reads with nothing to return wait. The input-available event follows
  the queue. The host asks the terminal for win32-input-mode and focus
  reports at start.
- **Added:** Ctrl+C and Ctrl+Break from the keyboard, GenerateConsoleCtrlEvent
  and process groups. Events go out on a ctrl worker thread through a
  bounded, coalescing queue (§7.4), never from the engine thread.
- **Added:** `platform_windows::condrv::key_for`, the key that types a
  character on the active layout (`VkKeyScanW`).
- **Known limitations:** cooked editing is typing, Backspace, Escape and
  Enter; the cursor keys, history and F-key popups come later. Scan
  codes are 0 and mouse reports aren't turned into records (§5.10 V8).
  Code pages other than 65001 still follow in PR 9.
- **CI repair:** fixed the shared-workspace TickStore write/observe race that
  could duplicate board events, and made the mux reconnect lifecycle check
  observe the repaint from before the successful takeover completes.
