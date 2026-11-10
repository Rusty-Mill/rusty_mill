---
category: Added
changelog: On Windows, mux blocks on our own console host report their foreground process (the newest one on the console) and send process-changed (C31).
---
## 2026-11-10 - Foreground process on Windows (own console host)

- **Added:** on our console host, `terminal.process` lists the processes attached to a block's console, newest first with the program last, and `process-changed` fires when the newest changes. `terminal.shell` reports `idle`/`busy` there too. Names come from each process's image, `.exe` kept.
- **Added:** `rusty_conhost::OwnPtyMaster::attached` and `platform_windows::process_image_path` (admits `QueryFullProcessImageNameW`).
- **Known limitations:** the command line is empty on Windows (pack Q42). ConPTY blocks still list only the program, with no event. A GUI program a shell starts is not attached to the console, so it is not reported.
