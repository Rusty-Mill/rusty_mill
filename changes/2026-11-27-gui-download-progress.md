---
category: Added
changelog: The GUI's Download file command shows how much of the file has arrived.
---
## 2026-11-27 - Download progress in the GUI

- **Added:** the palette's "Download file…" shows "Receiving file" with a percent in the notice, as an upload does, and `rusty_mux_client::download::fetch_progress` reports bytes written and the file's size per chunk. `Source::download` takes a progress callback (public trait; the mux GUI source is its only implementor).
- **Known limitations:** the server still reads files on Linux only; Windows and macOS download needs `unsafe` (not approved).
