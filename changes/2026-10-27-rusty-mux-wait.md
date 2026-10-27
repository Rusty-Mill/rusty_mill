---
category: Added
changelog: rusty_mux wait primitives: `rusty-mux-ctl wait` blocks until a block's program exits and exits with its status, or until an event whose fields match; `run --wait` starts a program in a new block and does the same; `create-block` takes a command, arguments and working directory.
---
## 2026-10-27 - rusty_mux wait primitives (C30)

- **Added:** `create-block` takes an optional `command`, `arg` (repeat the flag for each argument; a list in MCP and `api describe`) and `cwd` (an absolute path), carried in `Message::CreateBlock`'s new `Launch` behind `Features::LAUNCH` (bit 11). The server refuses a relative or missing directory, and a program it cannot start. A client never sends them to a server without the feature, which would start its shell instead.
- **Added:** `rusty-mux-ctl wait --block BLOCK [--timeout SECS]` waits for the block's program to exit, prints the `program-exited` event and exits with the program's status: its exit code, or 128 plus the signal that ended it. A block closed before its program exited exits 1; a code outside 0 to 255 exits 1 too, still printed.
- **Added:** `rusty-mux-ctl wait --event KIND [--FIELD VALUE]... [--timeout SECS]` waits for an event of that kind whose fields have those values, prints it and exits 0. Kinds and fields are those `on` rules use; `session`, `window` and `block` take targets like id flags.
- **Added:** `rusty-mux-ctl run [--window W] [--cols N] [--rows N] [--cwd DIR] [--wait [--timeout SECS]] [--] COMMAND [ARG]...` starts a program in a new block, in ctl's own directory unless `--cwd` says otherwise, and prints the block's id; with `--wait` it then waits as `wait --block` does. It follows events before it creates the block, so an exit is never missed.
- **Added:** a timeout exits 124, as `timeout(1)` does. `Exit` gains `TimedOut` and `Program`, and `Exit::code` replaces casting it to `u8`.
- **Known limitations:** `wait` and `run` are ctl commands, not MCP tools. A config action passes `arg` one value at a time. A program's status can coincide with ctl's own exit codes, as with Rex.
