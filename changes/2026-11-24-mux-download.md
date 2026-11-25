---
category: Added
changelog: A client can read a file of a block's server with `terminal.download`, in chunks.
---
## 2026-11-24 - File download from a server (C83, card #1586)

- **Added:** `terminal.download` block call behind `Features::DOWNLOAD` (bit 56), and `rusty_mux_client::download::fetch`. The input-lease holder reads a regular file of up to 2 GiB, 192 KiB at a time (size on the first line, then base64). Network paths (also as a path resolves, such as a Windows UNC form), directories and the server's own state directory are refused, judged on the file as opened; if the state directory cannot be resolved, nothing is read. The read runs off the server's main thread, as `terminal.list-dir` does.
- **Agent blocks:** `terminal.download` is refused on an agent's block (Nano, 2026-11-24, "Refuse on agent blocks"), judged by its launch facts or the `lel-jail` program, as `terminal.upload` is, and on a block with no running program (restored read-only or exited), whose launch facts are not known to be safe. Linux only; files that hold the server's own secrets are refused.
- **Changed:** `rusty_base64` is now a normal dependency of `rusty_mux_client` (it was dev only); no new crate.
- **Known limitations:** no GUI, no `rusty-mux-ctl` command, no MCP tool, and no copy between two machines yet.
