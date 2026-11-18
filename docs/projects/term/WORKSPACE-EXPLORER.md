# Workspace explorer (card #1573)

Status: design, proposed; not authorized to build. Source: Nano, voice, 2026-11-18 (remind-me mem_797b5707, card #1573). Nano gates it.
Pack ids to reserve: handoff D337, Q96; gui-design D95, Q93.

## What

A read-only side panel scoped to the session's workspace (the repo or worktree the launch profile checks out). It answers "who is changing what".

- Tree, with a badge or colour per agent on files that agent's block has touched.
- Actions: copy path, reveal/cd in a block, open in an external editor. **No editing, no LSP, no file tabs.** Term stays a terminal.
- Scope: the session workspace by default. Selecting a card in the [session overview](SESSION-OVERVIEW.md) widens it to every worktree that card's blocks touch (a library crate and its consumers).
- TUI: a text tree plus a touched-by list.

## Data

| Need | Source |
|---|---|
| workspace path per session | launch profile (LEL #1571); `terminal.pwd` / `PwdChanged` (D105) as a fallback |
| directory listing | `terminal.list-dir` (`DIR_LISTING`, bit 47; handoff D303, gui-design D77-D78), lease-holder only |
| touched files per block | `git status` of the block's worktree, or the overlay/throwaway layer diff; polled, inotify later |
| block to card | Mill Board execution link, as in the overview |

The mux has no worktree or git state today. Touched-file data is a new server call.

## Wire (not allocated here)

A server call for tree and touched files needs a message and a Features bit: a protocol and security change (reads under the workspace root only, no path escape, owner gate). Alternative: the GUI reads its own host filesystem, which needs no wire change but only works locally.

## Order

1. Server call and ctl form (cloud-buildable, owner gate).
2. TUI text tree (cloud-buildable).
3. GUI panel on the D60 panel tokens (needs a Windows GUI build; not cloud-buildable).

## Open questions (proposed only; implementation gated until Nano's answer is recorded)

Every item below is a recommendation, not an authorized default. Nano gates this design (card tag `needs-nano`). Wire, security, privacy, retention and export behaviour must not be built until his answer is recorded in the pack by the expert.

- **Q96** Wire call or local-only? a) wire call scoped to the block's workspace, works remote; b) local-only first. **Recommend a**: the mux is often remote and `list-dir` already shows the pattern.
- Panel look goes to `rusty-term-design-expert`.
