# Workspace explorer (card #1573)

Status: design; slices 1 (list), 2 (stat, touched-by calls) 3 (TUI tree shows them) and the copy-path action built; server call accepted (handoff D337, Nano 2026-11-18). The wire slice still passes the owner gate when its PR is opened. Source: Nano, voice, 2026-11-18 (remind-me mem_797b5707, card #1573).

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
2. TUI text tree (cloud-buildable): lists, then shows the size and age of the row under the cursor and a letter beside each changed file (`*` on a directory with a change below it). Tree actions follow, each with its own gate: copy path is built (`y`, OSC 52, D374); `c` cd is built with a confirm step (D375/D376, needs Nano's confirmation on its PR, Q107); open in an editor is not.
3. GUI panel on the D60 panel tokens (needs a Windows GUI build; not cloud-buildable).

## Open in editor needs the block's own workspace (D377)

The action is not built. The tree names a file relative to the block's workspace root, which the server holds as a directory handle (D342) and no call returns. A new block starts in the server's configured directory, which can be another directory with a same-named file. The client cannot prove the root, so a launch from the client would be a guess: PR #1929 tried a size and modification-time check, the review rejected it (neither is file identity), and its code was reverted. Nano chose to scope a server-side root first (D377, decision card 2026-11-27): this is that scope, with no wire, code or Feature bit until a later go and the owner gate on its own PR (D337, D375).

**Candidates**

- **A (recommended): `Launch.workspace_of: Option<BlockId>`**, a new optional launch field behind a new Feature bit (implies `WORKSPACE`). The new block starts in the held workspace of that block: the server takes that block's workspace handle where `create_block` now opens `config.cwd`, and `spawn_cwd` gives the directory. It excludes `cwd` (both is a bad launch). Nothing from the client names a path, so there is nothing to traverse.
  - Gates, as the list call: the input lease on the source block (D316) and on the new block's window; refused for an agent block (D372), after a server restart (D347), and off Linux (D346). The client must send the field only to a peer that negotiated the new bit, and the server refuses it without the bit: an old server would drop an unknown field and start in the wrong directory, so there is no fallback to `cwd` or to the default directory.
  - **Two guarantees, kept apart.** (1) *The workspace:* the new block starts in the held root of the named block, by handle, so a rename or swap of the path changes nothing (the D342 contract). That is the whole of what this design establishes. (2) *The file:* resolving the relative path under that root with the `O_NOFOLLOW` walk and refusing a link or a missing file validates the path at one moment. It does not prove that the separately launched editor then opens the same file: the file can be replaced or renamed between the check and the editor's own open, and the program resolves the name itself. File identity is therefore **unresolved**, not solved by (1) or by any check; a design that needs it (for example handing the program an already-open descriptor, or a launch the server performs on the resolved file) must be scoped on its own and is not part of this candidate.
  - The file name goes to the program as argv, never shell text.
  - The TUI hides `o` and says why when the bit is absent. The GUI panel uses the same call (its look is the design expert's).
- **B: a block call `terminal.workspace-edit <path>`** that launches the editor itself beside the block. Old peers get a refusal, but a launch inside a block call returns a new block id, an odd shape, and the server would pick the program.
- **C (rejected): a call returning an absolute path.** The held handle's `/proc` path means nothing to a client, and a real path can be renamed or swapped after it is read, which is the race D342 exists to prevent.
- **D (rejected): expose the configured directory.** Not authoritative once a shell has moved or the directory was renamed.

**First slice, when Nano says go:** the Feature bit and `Launch.workspace_of` in `rusty_mux_proto`, the server resolution with a test per refusal (no lease, agent block, unknown root, both `cwd` and `workspace_of`, bit not negotiated), and the `rusty-mux-ctl` form. It establishes the workspace only (guarantee 1). The TUI `o` follows and may claim no more than that, so it does not call the opened file the one picked while file identity is unresolved; the GUI last.

**For Nano:** which program runs. (a) the client names the program and arguments in `Launch` (recommended: it already travels there, no server policy; a remote mux needs the editor installed, and `sh -c 'exec ${EDITOR:-vi} "$1"'` uses the server account's `$EDITOR`); (b) a server-side rule picks the editor; (c) drop the editor and keep copy path (D374). Also the wire itself: the bit and field are an owner gate, with Nano's words on the PR that builds them.

## Decided

- **Q96** (D337, Nano 2026-11-18): wire call, list/stat/touched-by only, canonicalised paths, same gate as Q95.
- Panel look goes to `rusty-term-design-expert`.
