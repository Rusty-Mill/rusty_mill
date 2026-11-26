# Workspace explorer (card #1573)

Status: design; slices 1 (list), 2 (stat, touched-by calls) and 3 (TUI tree shows them) built; server call accepted (handoff D337, Nano 2026-11-18). The wire slice still passes the owner gate when its PR is opened. Source: Nano, voice, 2026-11-18 (remind-me mem_797b5707, card #1573).

## What

A read-only side panel scoped to the session's workspace (the repo or worktree the launch profile checks out). It answers "who is changing what".

- Tree, with a badge or colour per agent on files that agent's block has touched.
- Actions: copy path, reveal/cd in a block, open in an external editor. **No editing, no LSP, no file tabs.** Term stays a terminal.
- Scope: the session workspace only. The server call (D337) never leaves it, and selecting a card does not widen it. **Deferred:** showing every worktree a card's blocks touch (a library crate and its consumers) needs its own decision on how to cross workspaces, for example one session-scoped call per workspace, aggregated by the client. Nano's card asks for it; it is not authorized by D337.
- TUI: a text tree plus a touched-by list.

## Data

| Need | Source |
|---|---|
| workspace path per session | launch profile (LEL #1571); `terminal.pwd` / `PwdChanged` (D105) as a fallback |
| directory listing | `terminal.list-dir` (`DIR_LISTING`, bit 47; handoff D303, gui-design D77-D78), lease-holder only |
| touched files per block | `git status` of the block's worktree, or the overlay/throwaway layer diff; polled, inotify later |
| block to card | Mill Board execution link, as in the overview |

The mux has no worktree or git state today. Touched-file data is a new server call.

## Wire (D337)

A server call limited to list, stat and touched-by, scoped to the session workspace. No file contents: opening is the editor's job. Paths are canonicalised so a symlink cannot escape the workspace. Slice 1 (Nano, 2026-11-24, decision card in the project thread) is a block method instead of a new message: `terminal.workspace-list`, behind `Features::WORKSPACE` (bit 53), rooted at the block's launch directory, held open from launch so a renamed or swapped path changes nothing (an identity contract, not a path-prefix one), lease holder only. Slice 2 (D372) adds `terminal.workspace-stat` and `terminal.workspace-touched` behind `Features::WORKSPACE_STATUS` (bit 57); the tree shows them in a later slice. Old peers see the bit, so the same owner gate as D336 applies on its PR.

## Order

1. Server call and ctl form (cloud-buildable, owner gate).
2. TUI text tree (cloud-buildable): lists, then shows the size and age of the row under the cursor and a letter beside each changed file (`*` on a directory with a change below it). Tree actions (copy path, cd, open in an editor) follow, each with its own gate.
3. GUI panel on the D60 panel tokens (needs a Windows GUI build; not cloud-buildable).

## Decided

- **Q96** (D337, Nano 2026-11-18): wire call, list/stat/touched-by only, canonicalised paths, same gate as Q95.
- Panel look goes to `rusty-term-design-expert`.
