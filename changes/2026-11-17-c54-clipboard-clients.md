---
category: Added
changelog: The mux GUI answers a program's OSC 52 clipboard read by its own `clipboard` policy through the server, and the mux TUI denies it (C54).
---
## 2026-11-17 - Clipboard reads reach the mux GUI and TUI (C54, slice 2)

- **Added:** the mux GUI and TUI ask for `Features::CLIPBOARD` when they attach a pane. The GUI answers a `ClipboardRead` from the system or primary clipboard only under `clipboard = "read-write"`; otherwise it answers with nothing (D309). A pane served this way no longer answers the read from its own grid, so the program gets one answer (D312).
- **Security:** reads stay denied by default. The TUI has no clipboard policy and denies every read with an empty answer so the program does not wait for the 30 s limit.
- **Public API (additive):** `Feed::ClipboardRead`, `PaneIo::serves_clipboard` and `BackendHandle::clipboard_reply` in `rusty_term`, with a default that does nothing.
- **Known limitations:** the TUI does not relay reads to its host terminal (Q83). Server-side selection and a script or `ctl` read are not built (Q80, Q81).
