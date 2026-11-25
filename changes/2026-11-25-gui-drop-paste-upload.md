---
category: Added
changelog: The mux GUI sends a file dropped or pasted on a remote pane to that pane's server and types its path there.
---
## 2026-11-25 - GUI file drop and paste upload (C82 slice 3, card #1585)

- **Added:** dropping a file on a pane of `rusty-mux-gui`, or pasting files copied in a file manager, sends it to the block's server with `terminal.upload` (the pane must hold the input lease) and types the server's path, quoted, where a local drop types the local path. A refusal shows a "File not sent" notice. `rusty_mux_client::upload::send_with` is the chunking of `send` for a caller that owns the connection's reads. No wire change or new dependency.
- **Known limitations:** no progress display; the path appears when the upload ends. Checked against a real server in `gui/tests/attach.rs`; real-display drag and drop, the clipboard file list and Windows are unchecked.
