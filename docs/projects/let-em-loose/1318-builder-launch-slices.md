# #1318 builder launch: slice design

Card #1318 (bbp_factory, child of epic #600): orch launches each #600 builder session as a
mux-hosted pty whose process is `lel-jail`, which execs the Claude CLI's ACP adapter, with the
lifecycle held by orch through the cgroup holder. This finishes the Let Em Loose side of #732.

Status: **draft for Nano's approval** (2026-11-25). Nano asked for it on 2026-11-25 ("Yes,
design it now", decision card in the #600 thread). No code until he approves it on the card.

What this decides is only how to build it. The behaviour is already approved:
mux-hosted-agents-design.md §2–§4, §6 and §11 (D104), 600-builder-session-design.md §4 (D66),
the build plan §6 (`600-build-plan.md`), D107 (jail), D108 (holder and slot hand-off), D160–D162
(launch profiles), D165–D167 (credential and the one paid run), D177 (admission) and D180 (#726).
The let-em-loose expert checked this draft on 2026-11-25; its implementation defaults are D187.

## 1. What already exists

| Piece | Where | State |
|---|---|---|
| `lel-jail <jail.json>`: cgroup join, fds 0–4 only, Landlock, seccomp, exec | `orch-jail` (#1506) | merged |
| `JailFile` / `JailSpec`, one type for writer and reader | `orch-jail/src/spec.rs` (D107) | merged |
| `bbp cgroup-open`, `bbp slot-hold --cgroup --deadline-ms`, cgroup `quiesce` | `rusty_bbp_host` (#1507, D108) | merged |
| `SCRUB` drops `RUSTY_MUX_SOCKET` | `orch-bbp` (#1501) | merged |
| `rusty-mux-ctl own`: `launch` with exact `env`, `fds`, `hold`, `input: owner-only`; `signal`; `release`; owner events | rusty_mux C46, C75–C78 (#749/#750/#752 merged 2026-11-12) | merged |
| Builder admission, limits, `builder_session` table | `orch_lens::builder`, orch-store (#1404) | PR #1874 |

## 2. What is missing

1. **The holder taking the slot.** `slot-hold --cgroup` holds a slot it got on stdin; nothing takes
   one in cgroup mode yet (D108 amendment: "slot-taking and the fd hand-off move to slice 6").
2. **The fd hand-off.** orch duplicates the holder's slot descriptor with `pidfd_getfd` (D108
   item 2). `rusty_libc` has `pidfd_open` and `pidfd_send_signal`, not `pidfd_getfd`.
3. **Spawning `own` with fds 3 and 4 in place.** `std::process::Command` can't put a descriptor at a
   chosen number without `pre_exec`, and orch-bbp is `#![forbid(unsafe_code)]`.
4. **An ACP client in orch.** orch drives the session over fd 4: `initialize`, `session/new`,
   `session/prompt`, `session/update`, `session/request_permission`. Nothing in orch speaks ACP
   (rusty_key's ACP is the agent side and async; orch stays sync).
5. **The `acp.log` renderer and viewer block** (design §6).
6. **The launch sequence itself** and an entry point to run one builder on one claim.
7. **The §11 tests in CI.**

## 3. Slices

Each slice is a small PR that builds and passes its own tests. They are stacked: each is based on
the previous one's branch and retargeted to `main` when that merges. Review goes to dottiusmaximus.

### S1. `pidfd_getfd` and spawning with extra fds

- `rusty_libc::pidfd_getfd(pidfd, targetfd) -> Result<i32, Errno>` beside the existing
  `pidfd_open` and `pidfd_send_signal`.
- rustils `platform-linux` `sys/spawn.rs` already wraps `posix_spawn` file actions for stdio. A
  sibling `spawn_with_fds` there adds `adddup2` actions for extra descriptors (`&[(src, dst)]`),
  reusing its helpers rather than writing a second `posix_spawn` wrapper. Callers place fds 3 and
  4 with no `unsafe` of their own; the exact environment is passed, never inherited.
- Tests: getfd on a child's descriptor shares the lock (a second `flock` fails); a refused getfd
  returns its errno; a spawned child sees its fds at the asked numbers and nothing else above 2.
- **Ask-first:** two new public functions in platform crates.

### S2. `bbp slot-hold --cgroup PATH --dir D`

- The holder takes a slot itself through `slots.rs`, with the capacity recorded in the control
  directory (D108 item 2, already approved). It holds it on a descriptor it keeps and prints that
  descriptor's number as stdout's first line, which is also orch's "ready" signal before
  `pidfd_getfd`. In `--cgroup --dir` mode stdout carries that line, not the slot (plain
  `slot-hold` still holds the slot on stdout). At capacity it exits non-zero and takes nothing.
- The existing contract is unchanged: released only after stdin closes and the cgroup is empty;
  `cgroup.kill` from the deadline.
- Tests: slot taken and freed on an empty cgroup; capacity refusal; the slot outlives the holder's
  stdin while the cgroup is populated.
- Not ask-first: D108 items 1–2 approved the holder taking its own slot.

### S3. `orch-bbp::acp`: the ACP client and the `acp.log` renderer

- A sync, newline-delimited JSON-RPC 2.0 client over the parent end of the fd-4 socketpair, on
  `rusty_json`. Pure codec (requests out, messages in) plus one driver loop on a reader thread.
- One turn: `initialize` (no client fs or terminal capabilities, D101), `session/new` with the
  worktree as `cwd` and no MCP servers in v0, one `session/prompt` carrying the builder prompt, then
  `session/update` notifications until the prompt answers with its `stopReason`.
- `session/request_permission`: orch picks the offered `allow_once` option each time
  (`allow_always` only when there is no `allow_once`, which would stop later prompts and so later
  log lines) and writes the request and answer to `acp.log`. In v0 the jail confines and the
  agent's built-in tools stay on (D101); a prompt is not a boundary (spike #727). D187.
- Usage: tokens in and out from the prompt result's usage when the adapter reports it; otherwise
  0, and `acp.log` says so. Before #596 compares tokens, an unreported run must be told apart from
  0. D187.
- Renderer (pure): each `session/update` becomes text lines: agent message chunks, tool calls with
  title and status, plan entries, usage. A display aid, never evidence (design §6).
- Tests: codec round-trips; a scripted fake agent over a socketpair runs a whole turn; permission
  answers; a malformed line or a closed socket ends the turn as failed, never hangs; renderer
  golden lines.

### S4. `orch-bbp::mux`: the `own` adapter

This is *the* 1571 step-3 sender (session lookup and `own` child), not a builder-only one; the jail
path stays exactly D104's.

- Finds or creates the project's session and window with one-shot `rusty-mux-ctl` calls (#1571 §2),
  then starts **one fresh `rusty-mux-ctl own` child per builder**, spawned with S1's fd map so fd 3 is
  the slot and fd 4 the agent's ACP end.
- Requests: `launch` (program `lel-jail <jail.json>`, exact env `PATH`, `HOME`, `LANG`, `TERM`, no
  `RUSTY_MUX_*`, `fds: 2`, `hold: true`, `input: "owner-only"`, name
  `agent:<task>:<claim>:<n>`), `signal term`, `release`, `close`. A second, display-only block in
  the same window runs `tail -F acp.log`, outside the jail and held (D104 item 4, D187).
- Events: `program-exited`, `block-closed`, `lost`. A `lost` or a launch in flight when the
  connection drops is a mux loss.
- Tests against a fake `own` script: request lines exact; answers and events parsed; `lost` mid-launch
  reported as lost, not as failed.

### S5. `orch-bbp::builder` and `rusty_orch builder run`

The launch sequence of build plan §6, with the mux, the holder and ACP behind small ports so the
sequence is tested with fakes:

1. **Admit** with `orch_lens::builder::admit`, under the store's lock (load, admit, save).
2. **Prepare.** Worktree `wt/<claim>/<n>` from the claim's base (`rusty_worktree`). `bbp cgroup-open`.
   `jail.json` written with `JailFile`: write root the worktree; read roots the toolchain, the
   mirror root (empty for the toy claim; contents from the task's frozen inputs, design §4.3) and,
   per D166, Nano's `~/.claude` read-only; **no token** (the slot is optional, D107, and the minter
   is built last in #726, D180; D187); limits from the claim's lease; program `claude-code-acp`
   from the `claude-builder` agent file (D160–D162). The builder path forces `jail` isolation,
   above the agent file's `same_user` floor. Until 1571's `resolve` (step 2) lands, S5 hardcodes the
   `claude-builder` profile and says so; it switches to `resolve` when that exists.
3. **Hold.** `bbp slot-hold --cgroup PATH --slots DIR --deadline-ms <now + builder_wall_secs>`;
   `pidfd_getfd` its slot descriptor. A refusal fails the launch closed: no launch without fd 3.
4. **Open.** S4 launch, then the session row moves to `running`.
5. **Run.** S3 turn; `acp.log` under `<dir>/builders/<claim>/<n>/`.
6. **End.** On the stop reason, the wall clock, or giving up: `signal term`, 60 s grace, then
   **orch** writes `cgroup.kill` if anything is left, waits for empty, and sends `release` (D104 §2
   step 7). The holder's deadline kill is only the backstop for when orch is dead. The row ends
   `stored`, `failed` or `aborted`; how a candidate reaches BBP is the open point in §5.
   A mux loss or an orch restart ends it `aborted` (`orch_lens::builder::recover`), never debited by
   orch; admission reads the card's own candidate if BBP stored one first.

`rusty_orch builder run <config> --bbp B --dir D --task T --claim C --state S` runs one builder on
one claim and prints the session row as one JSON line. It is what the D167 toy run uses. A claims
loop that schedules builders for many claims is #1320's.

Tests with fakes: the end-path order (term, grace, orch's kill, release); a refused fd hand-off launches
nothing; a mux loss ends `aborted`; the wall clock ends `failed` with the cgroup killed; a stored
candidate ends `stored`; rows saved at each step.

**Ask-first:** the new `rusty_orch builder run` command.

### S6. The §11 tests in CI

In the existing "Orch BBP confined acceptance (Linux)" job: a real mux server, real `lel-jail` and
`bbp`, and a scripted fake ACP agent in place of `claude-code-acp` (no model call in CI):

- inside the jail `RUSTY_MUX_SOCKET` is unset, the socket path isn't reachable (Landlock; the mount
  namespace is #726), and `socket(AF_UNIX)` fails;
- a mux restart aborts the turn and debits nothing;
- the slot is released on an empty cgroup;
- the deadline holds with orch killed;
- a builder for a toy claim runs under `lel-jail` and its `acp.log` renders.

## 4. After the slices: the D167 toy run

One paid run is approved (D167): one claim, 2 builders, 45 minutes each, on BaileyAi, after Nano
adds the Claude `no_overage` assertion to `/etc/let-em-loose/assertions.json`. The toy run's config
sets `builders_per_claim` 2; the default stays 1 (D177). v0 is `same_user` (D161, D166): it runs on
Nano's own mux server as Nano's user, with `HOME` his home, and needs `claude-code-acp` installed
for his user. The `orch-runner` account isn't created in v0. I write a paste-ready prompt for it to `/mnt/project-files/let-em-loose/`
when S6 is green. #1320 stays unapproved.

## 5. Open points

**For Nano, with this design:**

| Point | Proposal |
|---|---|
| **How a candidate reaches BBP, and submit vs. gave up** (blocks S5; the jail has no socket but fd 4 and there is no gateway yet) | **a (recommended):** orch posts the diff and candidate under the builder's seat after the turn, only when the stop reason is `end_turn`, the agent wrote a submit marker (`.lel/submit` in the worktree) and the diff is non-empty; anything else stores and debits nothing. **b:** orch posts on any `end_turn` with a non-empty diff. **c:** orch offers BBP to the agent as a tool over ACP (an MCP server in `session/new`). a keeps "gives up and posts nothing" (600 design §4 step 7) from turning half-done work into a debited candidate, and orch already decides what reaches the board (D104); c adds a surface before #726. |
| New public surface | S1 `rusty_libc::pidfd_getfd` and rustils `spawn_with_fds`; S5 `rusty_orch builder run` |
| `claude-code-acp` for the toy run | A third-party npm package installed for Nano's user on BaileyAi, not a repo dependency |

**Settled as defaults (D187):** permission answers (`allow_once`, logged), usage 0 when unreported
(flagged before #596 compares), viewer `tail -F acp.log`, an empty mirror root for the toy claim
and a lock-file vendored mirror for #1320, and no token in v0 `jail.json`.

**Risk, not a decision:** the Claude CLI writes under `~/.claude`. If the read-only root breaks
it, a writable root would change D166 and goes back to Nano.
