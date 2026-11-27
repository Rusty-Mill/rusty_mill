---
category: Added
changelog: rusty-mux-ctl upload and download move one file to and from a block's server host (C82, C83).
---
## 2026-11-25 - rusty-mux-ctl upload and download

- **Added:** `rusty-mux-ctl upload [--block B] [--name N] FILE` sends a file to the block's server and prints its path there; `download [--block B] [--out FILE] PATH` copies a server file to stdout or `--out`. Both take the block's input lease like `forward`, are refused on an agent's block, and are not for `+` chains. `--out` is written beside itself and renamed into place on success, so a failure never touches an existing file; `--json download` without `--out` is refused. `--` lets a file name start with a dash.
- **Added:** `rusty_mux_client::Conn::with_pending`, so the client helpers can continue on a stream whose first bytes were already read.
- **Known limitations:** in `--json` mode the preparing `attach` and `acquire-lease` lines come first; the result is the last line. No GUI drop or paste, no machine-to-machine copy.
