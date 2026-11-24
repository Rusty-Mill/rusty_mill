---
category: Added
changelog: A client can send a file to a block's server with `terminal.upload`, which saves it under the server's state directory and answers with its path.
---
## 2026-11-24 - File upload to a remote block (C82, card #1585)

- **Added:** `terminal.upload` block call behind `Features::UPLOAD` (bit 55), and `rusty_mux_client::upload::send`. The input-lease holder sends a file in chunks of at most 1 MiB; the server keeps it in a folder for the block under its state directory and answers with the absolute path. A block keeps 2 GiB, counted before every chunk is written, unfinished files included (oldest removed first; at most 16 unfinished, one idle for 10 minutes is cleared), file names are cleaned, a second file of the same name never replaces the first, and the folder goes with the block.
- **Known limitations:** no GUI drop or paste, no `rusty-mux-ctl upload` command and no MCP tool yet; a server with no state directory refuses.
