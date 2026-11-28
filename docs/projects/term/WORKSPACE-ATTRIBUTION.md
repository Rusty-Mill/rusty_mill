# Workspace explorer: which agent changed which file (card #1573)

Status: design note only. No code, no `Features` bit and no wire change until Nano's later go and the owner gate on the PR that builds it. Scope is Nano's, project thread, 2026-11-28 (decision card on card #1573: per-agent badges wait for server-side per-block attribution; D372's refusal stays; remind-me mem_660f21b1). The pack row for it is the rusty-term-expert's to write; this note carries no D or Q number.

## The problem

The tree can show that a file changed (`terminal.workspace-touched`, D372, bit 57) but not which agent block changed it. Two facts stand in the way.

- `terminal.workspace-touched` is refused on agent blocks (D372, review of #1898). It runs `git status` as the server's account in a tree the agent controls, so the tree's own clean filters would run with the server's rights. That refusal stays.
- Without it the TUI has no per-block data to badge. D376 (3) settles for "a badge only when exactly one agent block shares the workspace", and with touched refused for those blocks it has nothing to read.

Attribution must therefore come from the server and hold without running git over the agent's tree.

## What exists

- An agent block is a `lel-jail` child (AGENT-HOSTING.md); the server tells it by program name or `Entry::Agent` (`refuse_agent`).
- A builder gets its own writable worktree. Today that is a clone with an alternates link to the base (726-sandbox-design.md, per-session layout); each builder has its own, so one worktree is one block.
- The planned jail layer (726 §9 step 2, #1412) adds an overlay of base, layers and a per-session `upper/`. It is not built: `orch-jail`'s `spec.rs` has no mounts yet.
- The server holds each block's launch directory as a directory handle (D342) and `Features::LAUNCH_IN_WORKSPACE` (bit 58, D378) already starts a block in another block's held workspace.
- No kernel facility used here names the writer: inotify gives no pid, and fanotify with a pid needs `CAP_SYS_ADMIN`.

## Candidates

**A. Per-block worktree, read as data (recommended once each builder has its own tree).** The key is the block, because each builder's workspace is the block's own held root. Attribution is "this file differs in block B's worktree", with no second block able to write there. The server answers it without `git`: it compares the worktree to its base by walking the held handle with the `O_NOFOLLOW` walk `workspace-list` uses (D344, D345) and checking size and mtime against the base index it reads natively. Nothing agent-written is executed, so D372's reason does not apply; the refusal stays for the `git` call. Cost: a native index reader (a new first-party parser, no new dependency) and a stat per tracked file. Limits: renames show as delete plus add (as D372); a file touched and restored reads as unchanged, which is right for "who is changing what".

**B. The session overlay's `upper/`.** When #1412 lands, `upper/` holds exactly what the session wrote, so attribution is by construction and a walk of it is cheap (whiteouts are deletes). It needs the server to be given the upper's handle at launch (a new jail-to-server fact, so a `Launch` field or a ctl hand-over) and waits on #1412. Best long-term, not buildable now.

**C. Write tracking in the server.** inotify on the workspace, recorded per block. Rejected as the base: no pid, so it attributes to a block only when exactly one block shares the tree, which A already gives without a watcher; watches miss writes before the watch starts and burn a descriptor per directory.

**D. Run `git status` inside the jail.** It would run with the agent's own rights, so the clean-filter concern ends. It needs a jail entry point that does not exist and puts a reply from the agent's side of the trust line on the wire, so the output is untrusted text. Left open, as D372 left it.

## Shape if built (for the later go; not decided)

- A block method `terminal.workspace-touched-by`, or a new reply form on the existing call, answering `XY<TAB>PATH` for an agent block's own workspace from A, or from B once it exists. The existing git path and its refusal are unchanged.
- One new `Features` bit, the next free one after bit 58, granting `WORKSPACE_STATUS` too. Old peers see the bit, so the same owner gate as D336 applies on its PR.
- Lease holder only, Linux only, refused after a restart (D347), bounded in time and size as D372.
- Per-block agent identity for the badge comes from `AGENT_META` (D336); the card text is a launcher's unverified label (D379), so the badge says the block, never a proof of who wrote the file.

## Not decided here

- A over B: A works now but needs a base-index reader; B waits for #1412.
- Whether the badge is the agent name or the role (look: the design expert's).
- Card-wide widening is unchanged (D379, D380); attribution adds nothing to it.

## For Nano

Which source to build first: (a) A, per-block worktree compared natively (recommended: buildable now, no jail change); (b) B, the overlay upper, after #1412; (c) neither yet, keep badges off. Any of them is a new wire call and `Features` bit, so it also needs your words on the PR that builds it.
