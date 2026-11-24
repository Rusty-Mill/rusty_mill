---
category: Added
changelog: The mux server can list a directory under a block's launch directory for the workspace explorer (terminal.workspace-list, Features bit 53).
---
## 2026-11-24 - Workspace explorer: terminal.workspace-list (#1573)

- **Added:** block method `terminal.workspace-list` behind `Features::WORKSPACE` (bit 53). It lists a directory in `terminal.list-dir`'s text form, rooted at the directory the block's program was launched in, and needs the input lease. `..`, an absolute path or a link in the path is refused. On Linux each component is opened from its parent with `O_NOFOLLOW` and the listing reads that descriptor; a link in a listing is shown as `l` and never followed. The first line is the path under the workspace, and a refusal names nothing outside it. `rusty-mux-ctl terminal.workspace-list [--path P]` runs it, and the MCP tool attaches and takes the lease first.
- **Known limitations:** Linux only: elsewhere the call is refused until it can be confined atomically. The workspace root is the launch directory, held open from the moment the block starts (its identity, not its path), while the server runs; after a restart it is unknown and the call is refused (keeping it would change the bincode block record, an owner format call). List only: stat, touched-by files, the TUI tree and the GUI panel are later slices.
