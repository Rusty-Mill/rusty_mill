# Mux-hosted agent ptys: design

Let Em Loose × rusty_mux, 2026-11-04. Status: **approved 2026-11-05 (D104)**, all eight §10 recommendations (decision card `cmsg_01WgyiM31RsSnZTgNuPX2VE41VyDsxTHxmCrXAPxQnCpra`).

- **Decision it implements:** D103 [stated]. Agents get mux-hosted ptys. It reopens D20 and D66 and supersedes D100. Nano, thread message `cmsg_01WgyiM31RsSnZTgNuPX2VE42EMPL7EXicY3bTWURHCCkG`, 2026-11-04T23:51Z.
- **Builds on:** D101 (the gateway enforces and the agent is jailed; #726 D-a) and D102 (orch and the gateway stay separate; #726 D-b).
- **Experts:** `let-em-loose-expert` covered orch, BBP and the jail; `rusty-term-expert` covered the mux and rusty_term. Its findings are in the Rusty Term pack QA ("What must the mux add for orch-owned agent blocks"), which is delivered on the Rusty Term side.
- **Ownership:** Let Em Loose owns the orch launch, the jail and the board events (this document). Rusty Term owns the mux and rusty_term side (§9 is its brief).

## 1. Shape

```
orch (decides)                      rusty_mux server (hosts)                jail (contains)
──────────────                      ────────────────────────                ───────────────
admit, mint token, make cgroup ──►  Launch{exact env, fds}   ──fork/pty──►  lel-jail ──exec──► agent
hold slot (cgroup holder)           pty master, grid, events                 namespaces, cgroup,
ACP over fd 4  ◄──────────────────────────────────────────────────────────►  overlay + Landlock,
kill/freeze via cgroup              SignalBlock, Release (no kill)           seccomp, token only
board events (BBP)                  input gate: owner grants lease
```

- **orch decides everything:** whom to run, admission, the token, the deadline, grace, kill, reap, and what reaches the board.
- **The mux hosts:** it forks the jail launcher on a pty it owns, renders that pty, and forwards input only when orch has granted it.
- **The agent sees** a tty, its throwaway layer and worktree, and one token scoped to its lens and claim (D101). It never sees the mux socket or the board socket.

## 2. One agent session, step by step (#600 builder)

1. **Admit.** orch picks the claim and checks the budget (600-builder-session-design §4.1). It takes an agent slot under `--slots`, which is an exclusive lock on `DIR/agent-<n>.lock`.
2. **Prepare.** orch creates the worktree, the cgroup `lel/<task>/<claim>/<n>`, and the token. It writes the jail spec to `<dir>/builders/<claim>/<n>/jail.json`: mounts, the Landlock and seccomp profiles, limits, and the token by reference.
3. **Hold.** orch starts a **cgroup holder** (an evolution of `bbp slot-hold`). It is outside the mux and outside the pty, and it holds a dup of the slot fd. It kills the cgroup at the lease deadline and releases the slot only once the cgroup is empty. If orch dies, the holder still enforces both.
4. **Open.** orch sends the mux `Launch` (§9 item 1):
   - program `lel-jail <jail.json>`;
   - an exact environment (`TERM`, `PATH`, `LANG`, and nothing from the server or `RUSTY_MUX_*`);
   - fds: fd 3 is the slot lock and fd 4 is orch's end of an ACP socketpair;
   - `hold: true`;
   - `input: owner-only`;
   - the name `agent:<task>:<claim>:<n>`.
5. **Jail.** `lel-jail` is the block's process, the pty's session leader. It does the following, then **execs** the agent:
   - joins the cgroup;
   - unshares the mount, pid, net, ipc and uts namespaces;
   - mounts the layers;
   - applies Landlock (rusty_sandbox) and seccomp, where seccomp denies `AF_UNIX` sockets except the inherited fd 4;
   - closes every fd except 0–2, 3 and 4.

   The agent is the ACP adapter for the Claude CLI. It speaks ACP on fd 4, and its stderr and any terminal it runs go to the pty.
6. **Run.** orch drives the session over ACP and renders its `session/update` stream to `<dir>/builders/<claim>/<n>/acp.log`. A second, display-only block runs a viewer of that log (§6). Meanwhile the witness records the upper-layer diff, the exec log and the gateway log (D101).
7. **End.** The turn can end because a candidate is stored, the wall clock runs out, or the agent gives up. orch then:
   - sends SIGTERM through `SignalBlock` and waits out the end grace (60 s);
   - writes `cgroup.kill` if anything is left;
   - waits until the cgroup is empty;
   - sends `Release` to the mux, which closes the pty master and never kills on its own.

   The holder then frees the slot. The block stays on screen (held) until orch or Nano closes it.

## 3. D20's guarantees still hold

D20 says there is no fourth executor. In this design orch stays the only executor, and the mux is a pty host, in the same way a terminal emulator hosts a shell. Each guarantee `bbp mod` gives today maps as follows:

| Guarantee today | How it holds here | Who enforces it |
|---|---|---|
| Group kill (the holder leads the process group; the moderator kills it) | `cgroup.kill`. This is stronger than a process group, because `setsid` and `setpgid` can't escape a cgroup. | orch and the holder |
| Slot held while any member lives (the lock is shared through the open file description) | fd 3 is inherited into the jail, and the holder keeps a dup until the cgroup is empty | the kernel lock, and the holder |
| End grace, then kill | SIGTERM through the mux, a 60 s grace in orch, then `cgroup.kill`. The mux has no implicit kill (`Release`, not `CloseBlock`). | orch |
| Lease deadline even if orch dies | The holder kills the cgroup at the deadline | the holder |
| `bbp quiesce` | `quiesce` checks that each recorded cgroup is empty, instead of checking process-group records | `bbp quiesce` |
| Reap and pricing | ACP usage reported on fd 4 and the exit status from the cgroup. stdout no longer carries the `result` line. | orch |
| Crash of the moderator or orch | Unchanged: the holder enforces the deadline and frees the slot, and a restarted orch aborts the stale turn (bbp_host README § The moderator) | the holder and orch |
| **New: the mux restarts** | The pty master closes and the agent gets SIGHUP. orch treats this like a crash: the turn is aborted, nothing is debited, and the claim retries once (600 design §5). Restarts are kept rare (`KillMode=process`). | orch |

So the mux never decides to start or stop an agent. It forks one only when orch asks, and it never kills one on its own. **D20 can then be re-closed as "orch is the only executor; the mux hosts ptys"**, and **D66 can be re-closed** with the builder launch going through `Launch` in place of a direct spawn. Its lease, sandbox, wall clock and mirror are unchanged.

## 4. The mux socket never enters the jail (D13)

- **The pty crosses the boundary; the control socket doesn't.** The jail's channels are the pty (display and takeover input), fd 3 (the slot lock) and fd 4 (ACP to orch).
- **Environment:** `Launch` sends an exact environment, and the server adds no `RUSTY_MUX_*` variables. orch-bbp's `SCRUB` gains `RUSTY_MUX_SOCKET` (one line plus a test), so it never leaks from whatever started orch.
- **Filesystem:** the jail's mount namespace doesn't include the runtime directory that holds the mux socket.
- **Syscalls:** seccomp refuses new `AF_UNIX` sockets and `connect`, so even a leaked path is useless. Internet egress goes only to the gateway (D101).
- **Fds:** `lel-jail` closes everything except 0–4 before it execs.
- **Tests (at build time):** inside the jail, `RUSTY_MUX_SOCKET` is unset, the socket path doesn't exist, and `socket(AF_UNIX)` fails.

## 5. Human input: off by default, explicit takeover, recorded

**Default.** The block is `input: owner-only`. No client can take the input lease unless the owner (orch's connection) grants one (§9 item 3). Watching needs no lease.

**Takeover flow.**
1. Nano asks for it on the human channel: `bbp human takeover CLAIM --session <block> --minutes N --reason "…"`. A rusty_term key binding can run the same command. The board writes `takeover_started` **before** any input is possible.
2. orch sees the event and sends the mux `GrantLease{block, client, until}`. It targets only the client Nano named, for at most N minutes.
3. Each keystroke batch the mux delivers is sent to orch as `InputBatch{block, client, seq, bytes}`. orch appends it to a local journal and posts `takeover_input` batches: one every 2 s, or on Enter, at most 4 KiB.
4. The takeover ends in one of these ways: Nano releases it, it expires, orch revokes it, the turn ends, or the mux is lost. orch then sends `RevokeLease` and posts `takeover_ended`. If orch can't post, it revokes the lease, so the system fails closed. A release, or a `Close`/`Cancel` with a live takeover, first writes `takeover_closing`: input stops at its `at`, and orch has 10 s to post input already delivered before it writes `takeover_ended` (D109).

**Recording mode.** The recommendation is as-delivered with a hash chain: the mux delivers right away, and orch journals and posts. Write-ahead, where the mux holds each batch until orch acks it, is possible, but it adds a round trip per keystroke and makes the mux side size L (rusty-term-expert). Gaps are detectable because `seq` and the chain are on the board.

**Proposed BBP event shapes (ask-first: a BBP schema change; not approved).**

| Event | Writer | Payload |
|---|---|---|
| `takeover_started` | human | `claim`, `epoch`, `session` (block name), `client`, `until`, `reason`, expected `rev` |
| `takeover_input` | host (orch) | `takeover` (msg id), `seq` (from 1), `bytes` (base64, at most 4 KiB), `sha256`, `prev_sha256` (the chain) |
| `takeover_closing` | written by release, or by `Close`/`Cancel` for a live takeover | `takeover`, `reason` (`released` or `turn_ended`), `at` (D109) |
| `takeover_ended` | host only (D109) | `takeover`, `reason`: `released` (only after a closing), `expired`, `revoked`, `turn_ended` or `mux_lost`; `batches`; `last_sha256` |

- **Effect on claims (v0):** a candidate stored from a session that had a takeover carries `human_touched: true` on its manifest and on `claim_card`. The inbox shows it. Scoring is unchanged. The takeover doesn't count Nano as a second author toward `min authors`.
- **Visibility:** the human channel and the host only, never the Agent API (like credentials), so agents can't read Nano's typing back.
- **Budget:** human writes are uncharged.
- **Schema:** the events are added under claims schema 5, as #597's holdout events were (D105): there's no schema bump. An older build refuses a log that contains one, and earlier logs stay byte-identical. It also needs one new human verb, `takeover`. Nothing is written when there's no takeover. `human_touched` is derived in the fold: a candidate for claim c is touched when a `takeover_started` on c in the same epoch precedes it (D106).
- **Caution:** typed text is recorded verbatim, so never type a secret into an agent pane.

## 6. What's displayed and what's recorded

- **Displayed.** Each agent gets one mux window with two blocks: **live pty** (the jail: stderr, the terminals the agent runs, takeover input) and **session** (a viewer of `acp.log`, which is orch's rendered ACP `session/update` stream). Both are held after exit, so the panes outlive the turn.
- **Recorded as evidence (D101).** The witness: the upper-layer diff, the exec log (audit/eBPF by cgroup) and the gateway's tool-call log, posted as hashed digests on the claim. The runner's `test_report` stays the only route to `executed` evidence.
- **Low trust.** ACP `session/update` is self-reported. A mismatch with the witness counts as evidence against the claim (D101). The `acp.log` file is a display aid, not a record.
- **Never evidence.** Mux scrollback, grid snapshots and `terminal.format`.
- **Q13 is answered.** The builder session's files live under `<dir>/builders/<claim>/<n>/` (`jail.json`, `acp.log`, the witness digests), beside its `builder_session` row.

## 7. Topology

- **BaileyAi.** It runs orch as `orch-runner` and one mux account server as `orch-runner`, so orch's connection owns the agent blocks. Agents run in the jail under a user namespace with no route to the socket (§4).
- **Nano's desktop.** rusty_term attaches remotely. **It can't today** (rusty-term-expert):
  - `rusty-mux` builds on Windows, but `--ssh` is Unix-only;
  - UDP (#1270) needs `--ssh`;
  - rusty_term doesn't link the mux client (rusty_term D22);
  - login mode (C26) is Linux-only.
- **Sizing:**
  - **(a) M:** Windows `--ssh`, using OpenSSH on Windows to forward the socket, with the `rusty-mux` client in a rusty_term tab. Nano is a viewer through a C26 login-mode map entry, `nano → orch-runner`, which is a security decision.
  - **(b) L:** a mux client inside rusty_term. That's a new crate edge, and it changes rusty_term D22.

  Recommend (a) first.
- **Identity.** Ownership is per connection, not per uid (C46, not started), so Nano's viewer can't rename, close or kick orch's blocks even under the same principal.

## 8. Scope and order

1. **#600 builders first.** #727's build order puts the full #726 jail sixth. So v0 builds `lel-jail` on what already exists, then grows it:
   - **v0:** rusty_sandbox (Landlock), seccomp, cgroup, worktree, mirror and token;
   - **later, behind the same `jail.json`:** overlay layers, network namespace with gateway-only egress, the witness.

   The launch contract stays the same throughout.
2. **Lens sessions under #726, after that.** This conflicts with D8 and D54 and needs Nano's call when it starts (§10). D8 makes lenses stateless per question, and D54 has the lens shell reach BBP through its own `bbp mcp` child, while a jailed agent reaches the board only through the gateway (D101). Until then lenses stay as now: orch-side, one-shot, with no blocks.
3. **Codex** stays a one-shot lens in v0 and gets no block (D64).
4. **Stage-lifecycle `bbp mod` harnesses** stay unchanged.

## 9. The mux API orch needs (brief for Rusty Term)

From rusty-term-expert. None of it exists today. Each item is a new capability row (C74 onward) that Nano starts (rusty_term D36, D37). Features bits start at 33 (D27); the numbers here are proposals.

| # | Capability | Shape | Size |
|---|---|---|---|
| 1 | **Agent launch** | bit 33 `AGENT_LAUNCH`. `Launch` gains `env: Option<Vec<(k, v)>>` (exact: no inheritance and no `RUSTY_MUX_*`), `fds: Vec<u32>` (target numbers, with SCM_RIGHTS on that frame, Unix socket only, through `rusty_libc`), `hold`, and `input`. The child dup2's its fds and closes the rest. | M |
| 2 | **Signal and release** | `SignalBlock{block, signal}` to the pty's process group. `Release{block}` closes the master with no SIGKILL. bit 34. | S |
| 3 | **Input gate** | bit 35 `INPUT_GATE`. With `input: owner-only`, `AcquireLease` is refused without a grant. Owner-only `GrantLease{block, client, until}` and `RevokeLease`. `InputBatch{block, client, seq, bytes}` goes to the owner only. | M |
| 5 | **Hold on exit** | With `hold`, the grid stays after exit, `ProgramExited` is sent, and the block waits for an explicit close. | S |
| 8 | **Ownership** | The launching connection owns the block (C46). Other clients can't rename, close or kick the owner, or take a lease on it. | M |
| 7 | **Remote viewer** | Windows `--ssh` plus the C26 map entry (§7 a) | M |
| — | Deferred | Surviving a mux restart (handing off pty masters, size L). A program-less feed block (a viewer program is enough). Write-ahead input. | — |

fd passing can't cross the C26 relay or SSH. orch talks to the account server's local socket on BaileyAi, so that isn't a problem.

## 10. Decisions for Nano (approved as recommended, D104)

Recommendations are in **bold**.
1. **Approve this design as a whole.** It re-closes D20 as "orch is the only executor; the mux hosts ptys", and D66 with launch through `Launch`.
2. Approve the BBP takeover events in §5 (an ask-first schema change; built under schema 5, D105): **yes, as shaped**.
3. Recording mode: **as-delivered with a hash chain**, or write-ahead.
4. Start the mux rows 1, 2, 3, 5 and 8 now (rusty-term-expert option a): **yes; defer 4 and 6, and use a viewer program for 6**.
5. Should a granted lease survive a client reconnect (rusty_term D44's 30 s grace) until its `until`, and be refused on re-ask once revoked (D46)? **Yes**, which keeps D44 and D46.
6. Remote attach: **(a) Windows `--ssh` plus the C26 `nano → orch-runner` map entry first**; (b) a client inside rusty_term later.
7. #600 starts on rusty_sandbox, seccomp and cgroup, and adds the #726 layers later behind the same launcher: **yes**.
8. Lens sessions in blocks (D8, D54): **decide when #726 reaches lens sessions; no change now**.

## 11. Work split

- **Let Em Loose PR (after approval):** in orch-bbp, `lel-jail`, the cgroup holder, the `SCRUB` line, the `Launch`-based builder launch, the ACP fd plumbing and the `acp.log` renderer. In `bbp`, takeover events (after decision 2), the human `takeover` verb, and cgroup-based `quiesce`. Then tests: the §4 jail tests, a mux-restart abort, slot release on an empty cgroup, and the deadline holding with orch killed.
- **Rusty Term PR (after approval):** §9 rows 1, 2, 3, 5 and 8 in the mux, and §7 (a) for remote viewing from rusty_term.
- **One cross-project card: Mill Board #732.** Each PR is reviewed by Dottius.
