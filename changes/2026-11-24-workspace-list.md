---
category: Added
changelog: The mux server can list a directory under a block's launch directory for the workspace explorer (terminal.workspace-list, Features bit 53).
---
## 2026-11-24 - Workspace explorer: terminal.workspace-list (#1573)

- **Added:** block method `terminal.workspace-list` behind `Features::WORKSPACE` (bit 53). It lists a directory in `terminal.list-dir`'s text form, rooted at the directory the block's program was launched in, and needs the input lease. `..`, an absolute path or a link in the path is refused. On Linux each component is opened from its parent with `O_NOFOLLOW` and the listing reads that descriptor; a link in a listing is shown as `l` and never followed. The first line is the path under the workspace, and a refusal names nothing outside it. `rusty-mux-ctl terminal.workspace-list [--path P]` runs it, and the MCP tool attaches and takes the lease first. The saved block record gains a `workspace` field (the launch directory, kept across restarts; an older record falls back to its last known directory).
- **Known limitations:** on Windows the walk checks each component with `symlink_metadata`, which stops a link already in place but not a local process racing the check. List only: stat, touched-by files, the TUI tree and the GUI panel are later slices.
