---
category: Added
changelog: rusty_mux GUI: palette action "Download file…" browses the block's host and saves the file to the Downloads folder (C83).
---
## 2026-11-26 - GUI download file (card #1586)

- **Added:** the command palette's "Download file…" browses the focused block's host as "Insert path" does; Enter on a file fetches it with `terminal.download` and saves it in `~/Downloads` (never replacing a file: a taken name gets `-1`, `-2`), then the notice says "File saved" with the path, or "File not sent" with the reason. `rusty_mux_client::download::fetch_with` is `fetch` for a caller that owns the connection's reads; `Source::downloads`/`download` are the GUI's seam. Decision D100.
- **Known limitations:** no progress or transfer status display, no GUI copy between hosts (use `rusty-mux-ctl copy`), and a file is chosen in the browse, not typed as a path. Attached mode only.
