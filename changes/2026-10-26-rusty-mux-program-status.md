---
category: Added
changelog: rusty_mux program status: programs report what they are doing with OSC 7501, the server keeps those records per block, announces state changes as `block-status` events, and `terminal.status` reads them.
---
## 2026-10-26 - rusty_mux program status (OSC 7501)

- **Added:** `rusty_mux_status`, a parser and record store for OSC 7501, the public Program Status Protocol (draft 0.4). It handles the states idle, working, blocked, done, error and clear, plus `id` paths, `kind`, `progress`, `app` (children inherit it), and base64 `title` and `msg`, within the protocol's size limits. It keeps at most 64 records and evicts the least recently updated one.
- **Added:** `rusty_term_core` collects OSC 7501 reports, shell prompts (OSC 133/633 `A`) and full resets into `Grid::program_status`, but only when the host sets `collect_program_status`. The queue is bounded at `PROGRAM_STATUS_MAX`. Nothing changes for other hosts.
- **Added:** `Event::BlockStatus { block }` (event kind 13). The server emits it when a block's record states change; progress or text changes alone don't trigger it. It reaches only clients that negotiate `Features::STATUS` (bit 8), live and on replay, so other clients' event numbers skip it. `rusty-mux-ctl events` asks for it and prints `block-status`.
- **Added:** the block method `terminal.status`, which returns one canonical OSC 7501 body per record, the root record first.
- **Known limitations:** there is no reply to the protocol's `?` query. A block that sends more than 256 status signals in one read drops the extras.
