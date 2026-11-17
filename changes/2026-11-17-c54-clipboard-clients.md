---
category: Added
changelog: The mux GUI answers a program's OSC 52 clipboard read by its own `clipboard` policy through the server, and the mux TUI denies it (C54).
---
## 2026-11-17 - Clipboard reads reach the mux GUI and TUI (C54, slice 2)

- **Added:** the mux GUI and TUI ask for `Features::CLIPBOARD` when they attach a pane. The GUI answers a `ClipboardRead` from the system or primary clipboard only under `clipboard = "read-write"`; otherwise it answers with nothing (D309). A pane served this way no longer answers the read from its own grid, so the program gets one answer (D312).
- **Security:** reads stay denied by default. This is the change that lets real clients answer reads: on an owner-only block the owner is sent the answer as input (D313). An answer over 1 MiB is denied by the client. The TUI has no clipboard policy and denies every read with an empty answer so the program does not wait for the 30 s limit.
- **Public API:** `rusty_term` gains `Feed::ClipboardRead` (a new variant: exhaustive matches need an arm), `PaneIo::serves_clipboard` (a new field: struct literals need it) and `BackendHandle::clipboard_reply` (default does nothing); `rusty_mux_proto` exports `MAX_CLIPBOARD`. The server now asks the holder after sending the output that holds the query. `attach::PANE` now includes the clipboard bit, so a GUI from before this change cannot take over from one after it.
- **Known limitations:** the TUI does not relay reads to its host terminal (Q83). Server-side selection and a script or `ctl` read are not built (Q80, Q81).
