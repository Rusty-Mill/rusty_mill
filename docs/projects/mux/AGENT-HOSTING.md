# Agent hosting: the mux side of mux-hosted agent ptys

**Status.** Rows C75–C78 and the scoped C46 are approved to start by Let Em Loose D82: Nano tapped
decision card `cmsg_01WgyiM31RsSnZTgNuPX2VE41VyDsxTHxmCrXAPxQnCpra` at 2026-11-05T00:00:23Z,
choosing §10 items 4 to 6. The shapes below are still *Proposed* in [DECISIONS.md](DECISIONS.md)
until Nano accepts them on this PR (rusty_term D37), and the Rusty Term pack records its own row
for D82 item 4. There is no code until then.

- **Decision it serves:** Let Em Loose D81 [stated]: agents get mux-hosted ptys. Nano, 2026-11-04
  23:51Z, thread message `cmsg_01WgyiM31RsSnZTgNuPX2VE42EMPL7EXicY3bTWURHCCkG`.
  - orch decides and the mux hosts.
  - The block's program is the jail launcher `lel-jail`, which execs the agent.
  - The mux socket never enters the jail.
  - Human input is off by default, and every takeover is recorded on the BBP board.
- **Card:** Mill Board #732, one cross-project card. The Let Em Loose side is
  `docs/projects/let-em-loose/mux-hosted-agents-design.md` (PR #1496). Its §9 is this document's
  brief, and its §7 covers remote viewing.
- **Split:** this document covers only the mux and rusty_term. Let Em Loose owns the jail, the
  cgroup holder, the ACP plumbing and the board events.
- **Numbers:** these are the next free values on main at 1984826d, with open PRs #1490 (C74,
  bit 33) and #1493 (C70, bit 34) counted:

  | Next free | Value |
  |---|---|
  | Capability | C75 |
  | Features bit | 35 |
  | Client message type | 0x21 |
  | Server message type | 0x8e |
  | Event kind | 33 |

  Each row takes the numbers that are free when it is built (D27), so the values below are
  placeholders.

## Rows

| Row | LEL §9 | Capability | Bit | Size |
|---|---|---|---|---|
| C46 | 8 | Block ownership (the existing row, now scoped): the launching connection owns the block | 35 `OWNERSHIP` | M |
| C75 | 1 | Agent launch: exact environment and passed descriptors | 36 `AGENT_LAUNCH` | M |
| C78 | 5 | Hold on exit: the block and its screen outlive the program | 37 `HOLD` | S |
| C76 | 2 | Signal and release: signal the program's group, and close the pty without a kill | 38 `SIGNAL_RELEASE` | S |
| C77 | 3 | Input gate: owner-only input, owner-granted leases, and an input stream to the owner | 39 `INPUT_GATE` | M |
| C79 | 7 | Windows `--ssh` attach, so rusty_term on Windows can view the BaileyAi mux | none (client side) | M |

**Order.** The table is the build order: C46 first, because C76 and C77 rest on it, and C79
independently. Each row gets its own bit, so a peer knows exactly what it has (D27), and each row
can ship or be reverted on its own.

**Deferred** (LEL §9 and D82 item 4):
- Surviving a mux restart, which needs a handoff of pty masters (size L).
- A program-less feed block. A viewer program is enough.
- Write-ahead input recording.

## C46. Block ownership, scoped

The row exists but is `not-started`. This scopes it to what Let Em Loose needs.

**Shape.** Behind `OWNERSHIP`, `Launch.own: bool`.
- The connection that sends the launch owns the block. Ownership belongs to that **client**, not
  its principal, so a viewer that shares orch's uid gains nothing.
- The client's TAKEOVER token is the ownership credential. A reconnect inside D44's 30 s grace
  keeps the client, and with it the ownership.
- Blocks created without `own` have no owner and behave exactly as today.

**Owner only:** `Rename`, `CloseBlock`, `Split`, `ResizeSplit`, `SwapBlocks`, `MoveBlock`,
`DetachBlock`, and C76 and C77's requests. `ZoomBlock`, `FocusBlock` and `SelectWindow` stay open
to every client, because they only change what that viewer looks at.

**Everyone else** may attach, watch, `list` and call the read-only block methods. A lease needs
a grant (C77).

**C33 for owners.** `disconnect-client` aimed at an owner connection is refused for every other
client. An operator who must cut orch off stops the account server. This narrows D35's "anyone
may disconnect anyone" for owner connections only, which is the first step of card 507.

**Owner gone past the grace** (question A). The block becomes *orphaned*:
- it stays, and so does its program;
- every grant ends, and no new grant is possible;
- nobody may type, rename it or signal it;
- a client with the owner's principal may `CloseBlock` it, as cleanup. Nano's viewer comes in as
  `user:baileyrd`, so only orch-runner-uid clients can clean up.

orch treats its own restart as a crash anyway: it aborts the turn and kills the cgroup (LEL §3).

**Tests.**
- A second client of the same uid can't rename, close, split, signal or disconnect the owner.
- The owner can.
- Ownership survives a 5 s reconnect, and the block is orphaned after 30 s.

## C75. Agent launch: exact environment and passed descriptors

**Today.**
- `Launch` carries only `program`, `args` and `cwd` (`proto/src/message.rs` `Launch`, bit 11).
- `command()` builds `EnvSpec::Explicit` from the server's environment plus
  `RUSTY_MUX_{SESSION,WINDOW,BLOCK,SOCKET}` (`server/src/pty/mod.rs:38`, `state.rs` `block_env`).
- The pty child is started by `posix_spawn` with `POSIX_SPAWN_SETSID` (rustils
  `platform-linux/src/sys/pty.rs:115` `spawn_attached`), so there is no hook between fork and exec.
- Nothing in the mux passes descriptors.

**Shape.** Behind `AGENT_LAUNCH`, `Launch` gains two fields.
- **`env: Option<Vec<(bytes, bytes)>>`.** `None` keeps today's behaviour. `Some(list)` makes the
  child's environment exactly `list`: nothing is inherited, and the server adds no `RUSTY_MUX_*`.
  A caller entry named `RUSTY_MUX_*` is refused (`bad_launch(env)`), so a caller's mistake can't
  bring the socket path back. This is a new field on the existing `EnvSpec::Explicit` path, with
  no platform work.
- **`fds: Vec<u32>`.** These are target descriptor numbers in the child. The frame carrying the
  `Launch` brings exactly that many descriptors as `SCM_RIGHTS`, in order. They are accepted only
  on a direct `Peer::Allowed` socket connection, and refused on `Peer::Forwarded` (the C26 relay,
  `server/src/lib.rs:181`) and on datagram streams. The server also refuses the launch, and closes
  whatever arrived, when the counts differ, a target is below 3 or repeated, or there are more
  than 8 targets (`bad_launch(fds)`). An `ssh -L` forward can't carry descriptors at all, since
  sshd never sends `SCM_RIGHTS`.

**Child side (Unix).**
- `platform::process::Command` (`platform/src/process.rs:122`) gains an fd map.
- The Linux spawn adds `posix_spawn_file_actions_adddup2` for each target and
  `posix_spawn_file_actions_addclosefrom_np` above the highest target.
- The server closes its own copies once the spawn has returned, whether it succeeded or failed.

**Windows.** `env` works. `fds` is refused (`unsupported`), because AF_UNIX on Windows has no
`SCM_RIGHTS`.

**C45.** The persisted `Started` record (`store/record.rs:19`) does not gain `env`, so a caller's
environment, which may hold a token reference, never goes to disk (D74). A restored block that was
launched with `env` comes back read-only, like any command block (D72).

**Platform work.**
- `rusty_tokio`'s `UnixStream` gets `recv_with_fds` and `send_with_fds`, built on new
  `sendmsg`/`recvmsg`/`cmsg` support in `rusty_libc`. `rusty_libc` already has `dup2` and
  `close_range` (`fd.rs:473,520`).
- rustils `platform` and `platform-linux` get the fd map.

This extends first-party crates where the mux needs it (2026-10-22) and adds no third-party
dependency, but it is public API in three crate sets (question B).

**Framing.** ctl sends the whole `Launch` frame in one `sendmsg`. The server takes descriptors
only at a frame boundary, and refuses a frame that brings descriptors it didn't ask for.

**ctl.** `rusty-mux-ctl run --env-exact K=V … --fd 3=FD …`. orch-bbp calls ctl in-process
(`rusty_mux_ctl::run_until`) or as a child process, so it needs no new crate edge.

**Tests.**
- The child sees exactly the given environment, with no `RUSTY_MUX_SOCKET`.
- A passed lock file is still held while the child lives: `flock` from a second process fails.
- No other descriptor is open in the child (it lists `/proc/self/fd`).
- Each refusal case is refused and leaks no descriptor.

## C78. Hold on exit

**Today.** When the program exits, `exited()` (`server/src/state.rs:1679`) closes the block, and
its screen and scrollback go with it. A restored command block already has a read-only state
(`BlockRt.live = None`, `state.rs:120-226`; D72).

**Shape.** Behind `HOLD`, `Launch.hold: bool`. When the program of a held block exits, or the
block is released (C76), the block:
- reuses that read-only state, so it keeps its screen and scrollback;
- sends `program-exited` as today;
- shows as `exited` in `list` and `terminal.shell`;
- refuses input;
- stays until `CloseBlock`.

This is tmux's remain-on-exit, and it is useful beyond agents. A held block is saved and restored
read-only like any command block (D72). C70's spool, once merged, keeps its scrollback readable.

**Tests.**
- After the program exits, `terminal.format` still returns its last screen.
- Input to a held, exited block is refused.
- `CloseBlock` removes the block.
- A held block survives a server restart read-only.

## C76. Signal and release

**Today.** Closing a block runs `rt.release` and then `reap()`, which sends SIGHUP to the
program's group and then SIGKILL after the fixed 1 s `GRACE` (`server/src/pty/linux.rs:57-86`).
`Process::drop` also sends SIGKILL (`linux.rs:111`). For an agent, that kill must never happen:
orch owns the end grace and kills through the cgroup.

**Shape.** Behind `SIGNAL_RELEASE`, there are two new requests. Both are owner-only on an owned
block, and both are refused on Windows (`unsupported`); orch runs on Linux.
- **`SignalBlock{block, signal}`.**
  - It signals the program's own process group through the existing pidfd-safe `kill_tree`.
  - `signal` is a closed set (`hup`, `int`, `term`, `kill`, `stop`, `cont`), so no arbitrary
    numbers.
  - If the program is gone, it answers `exited`.
- **`ReleaseBlock{block}`.** The name avoids confusion with `ReleaseLease` (0x08).
  - It closes the pty master and waits for the program without ever escalating: no SIGKILL after
    a grace, and none from `Process::drop`.
  - On a master close, Linux sends SIGHUP, then SIGCONT, to the session leader, which is the
    spawned program (`POSIX_SPAWN_SETSID`). That is the kernel's rule, and nothing the mux adds.
  - orch sends `ReleaseBlock` only after the cgroup is empty, so normally nothing is left to
    signal.
  - A held block then stays read-only (C78). An unheld block closes once the wait ends.

**Tests.**
- `term` reaches a trap in the program's group.
- After `ReleaseBlock`, a program that ignores SIGHUP is still alive 2 s later. That shows there
  was no implicit kill.
- A non-owner is refused on an owned block.

## C77. Input gate

**Today.** Any client may take a free input lease (`core/src/mux.rs:899` `acquire_lease`). There
is no read-only attach in the server; the TUI takes the lease of the focused block. The requests
that need the lease are `Input` (paste included), `Resize`, `terminal.send-keys`,
`terminal.reset`, `terminal.set-theme` and `terminal.clear` (`state.rs:1138,1164,1191,1430,1542`).

**Shape.** Behind `INPUT_GATE`:
- **`Launch.input`:** `open` (today's behaviour, and the default) or `owner-only`.
- **Lease refusal.** On an `owner-only` block, `AcquireLease` is refused (`input_gated`) unless the
  client holds a live grant. Watching (Attach without a lease) is unchanged.
- **`GrantLease{block, client, until}`.** Owner only. It lets that one client take the lease
  until `until` (unix ms on the server's clock, at most 24 h ahead), matching LEL's shape. A grant
  is per client, not per principal.
- **`RevokeLease{block, client}`.** Owner only. It ends the grant and, if that client holds the
  lease, clears it (holder none).
- **`grant-changed{block, client, until}` (event kind 33).** It tells the grantee when it may
  ask, so it never has to guess (D46). If the owner itself holds the lease, a grantee's ask gets
  `LeaseHeld` as today.
- **`InputBatch{block, client, seq, bytes}` (server message 0x8e).** It goes to the owner only,
  once for each lease-gated write to the pty of an `owner-only` block: `Input` and
  `terminal.send-keys` bytes. `seq` counts from 1 per block.
  - It is a message, not an event, so it never enters the shared event `History` and is never
    replayed to a resuming client.
  - Bytes go to the pty first, then the message goes to the owner, which is the "as delivered"
    mode (D82 item 3). orch batches the messages for the board.
- **Fail closed.** Input on an `owner-only` block is refused (`input_unrecorded`):
  - while the owner's outbox is above a threshold under its 8 MiB `BUDGET` (`outbox.rs:11-14`);
  - while the owner is inside its D44 grace, since nothing could be reported then.

  Input is never delivered without being reported. If the owner is gone for good (C46
  orphaned), every grant ends.
- **Reconnects** (D82 item 5). A grant survives a reconnect inside D44's 30 s grace, until
  `until`. After a revoke or expiry, asking again is refused (D46). D44 and D46 stand.

The owner itself may always take the lease. orch has no reason to type, but refusing it would add
a rule with no use.

**Tests.**
- A client without a grant is refused the lease.
- A granted client can type, and the owner gets every byte in `seq` order, including
  `send-keys`.
- After a revoke, a client that had a grant is refused.
- A resuming third client never sees an `InputBatch`.
- A stalled owner refuses input.
- A grant survives a reconnect inside 30 s, and is gone after `until`.

## C79. Windows `--ssh` attach

**Today.**
- The `rusty-mux` client builds on Windows, but `--ssh` returns "supported on Unix only for now"
  (`tui/src/main.rs` ~851).
- `Conn<S: Read + Write>` is already generic (`tui/src/lib.rs:264`). What blocks Windows is
  `remote()`, `SshPath` and `Route` (all `cfg(unix)`), and the `ssh` module (`lib.rs:16`), which
  uses `std::os::unix` APIs.
- On Unix, `--ssh` forwards a private local socket to the remote one with `ssh -L`, and UDP (C13)
  starts from that forward.
- The owned-forward mode needs OpenSSH 8.7 or later (`ForkAfterAuthentication`). Windows' in-box
  OpenSSH is often 8.1.
- rusty_term has no mux client (LEL §7).

**Path (D82 item 6, option a), size M.**
1. **Spike (S, first).** On Lap or Beast, record the in-box OpenSSH version, and check whether it
   does `-L LOCAL_SOCKET:REMOTE_SOCKET` streamlocal forwarding with a Windows AF_UNIX local
   socket.
   - **If it does,** port `ssh.rs` and `remote()` to Windows. The private directory gets the
     owner-only DACL from R10, and the version floor is checked at start with a clear error.
   - **If it doesn't,** add `rusty-mux-ctl bridge SOCKET`, run as `ssh HOST rusty-mux-ctl bridge
     SOCKET`. It copies stdio to the server socket on the remote side, and the client speaks the
     protocol over the `ssh` child's pipes, which `Conn` can already take. `bridge` is a new ctl
     subcommand with no wire change, and it serves Unix clients without `-L` too.
2. **UDP stays Unix-only** in this row. A Windows client stays on the SSH path. Windows UDP is a
   separate row if it's wanted.
3. **rusty_term.** Nano opens a rusty_term tab running `rusty-mux --ssh baileyai FRONT_DOOR_SOCKET`,
   the login-mode front door's socket. An account socket (`RUNTIME/ACCOUNT/server.sock`) refuses
   any uid that isn't the account's own or the front door's (`server/src/lib.rs:189-197`).
   rusty_term needs no code. Option (b), a mux client inside rusty_term, stays a later L.
4. **BaileyAi topology (approved in D82 item 6; host changes still gated).**
   - A root front door runs `rusty-mux-server --login MAP` with the map entry
     `user:baileyrd → orch-runner`.
   - orch connects directly to the account socket `…/orch-runner/server.sock`, where it may pass
     descriptors (C75).
   - The account server is started as `orch-runner` by its own systemd unit, so it exists before
     orch connects. A front door starts an account server only on first relay.
   - What the map entry grants: every block in orch-runner's account. Nano can type only with a
     grant (C77), and can't act on owned blocks at all (C46). **So the entry goes in only once C46
     and C77 are built.**
   - The unit, the front door and the map file are writes on BaileyAi, so each needs Nano's go
     when it's made.

**Tests.** The Windows CI leg runs `--ssh` against a fake `ssh`, the same way `ssh.rs` tests
`args`. A manual check from Lap to BaileyAi goes in a WINDOWS-MANUAL-CHECK file.

## Gates

D82 starts these rows. These still need Nano's or a reviewer's go on the PR that builds them:
- **Wire review for each row:** the `Launch` fields, client types 0x21 onward, server type 0x8e,
  event kind 33, and bits 35–39 (D27).
- **Public API:** `rusty_libc`, `rusty_tokio`, and rustils `platform`/`platform-linux` (C75), plus
  the new ctl `bridge` subcommand if the C79 spike needs it.
- **Security review:** descriptor passing, the input gate and ownership.
- **Writes on BaileyAi:** the front door, the map file and the account server's unit (C79).

## Questions for Nano

Answer on this PR.

- **A. An owner gone past the 30 s grace.**
  - Recommend **orphaned**, as in C46: the block stays, input is closed, and only the owner's
    principal may close it.
  - Closing the block instead would kill the agent from the mux side, against Let Em Loose D20 as
    re-closed by D82: "the mux never kills one on its own".
  - Letting any same-principal client adopt it reopens the viewer-takes-control hole.
- **B. Descriptor passing adds public API to `rusty_libc`, `rusty_tokio` and rustils `platform`**
  (C75). Recommend **yes**: it is first-party, adds no dependency, and extends those crates where
  the mux needs it (2026-10-22).
- **C. A stopping mux and owned blocks.** Today, stopping the server SIGKILLs every block's group
  (`Process::drop`).
  - Recommend **(b): owned blocks are only hung up**, and the cgroup holder cleans up, since D82
    says the mux never kills an agent on its own.
  - The alternative (a) keeps the kill, and orch sees it as `mux_lost` either way.
