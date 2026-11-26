---
category: Added
changelog: The mux GUI source can now read a block's workspace tree for the explorer panel.
---
## 2026-11-26 - Workspace explorer panel: the mux GUI adapter

- **Added:** `rusty_mux_gui::workspace` (`Workspace`, `Port`): the tree for the panel, on the source side. It owns the shared `rusty_mux_client::workspace::Explorer`, makes the list, touched and stat calls on the block's own connection (which holds the lease), and pushes flat `Snapshot`s through `Remote::explorer`. `MuxSource` implements `Source::explores` (`WORKSPACE`, plus `WORKSPACE_STATUS` for marks) and `Source::explore`. `Block::list_dir` now goes through a general `Block::call`. A block this client does not hold shows "take over first" and sends no call.
- **Known limitations:** nothing opens the panel yet; drawing and keys come next. Tested with a fake port (7 tests); the socket path is the same as the palette's folder browse.
