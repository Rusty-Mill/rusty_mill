---
category: Added
changelog: rusty-mux-ctl copy moves a file from one saved host's server to another's, streaming through your machine (card #1586).
---
## 2026-11-25 - rusty-mux-ctl copy (C83, card #1586)

- **Added:** `rusty-mux-ctl copy --block B --to-block B [--name N] HOST:PATH HOST`. It downloads the file from the first host's server and uploads it to the second's per-block folder, chunk by chunk, so nothing is kept on the machine running it. It prints the path on the destination. A failed download never commits a short file. No wire change, new bit, crate or dependency.
- **Known limitations:** both blocks must be named; refused on agent blocks like `upload` and `download`; the GUI drop and the Windows and macOS download paths are not part of it.
