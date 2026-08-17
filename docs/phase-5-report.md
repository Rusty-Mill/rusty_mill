# Phase 5 report — dependent sessions

Phase 5's stated scope (PLAN.md § Phased milestones):

> dependent sessions (real, independent scope). Fork is *not* in this
> phase.

Two CAPABILITIES.md capabilities, neither previously designed:
**dependent sessions** (a child session tied to a parent's worktree,
optionally waiting for the parent to finish, with a "start now" override)
and **dependent terminal sessions** (a plain terminal alongside a running
agent, in the same workspace). Neither concept existed in the domain
model before this phase — `SessionKind` had three variants and nothing
let a session attach to another session's workspace.

## Status

| Item | Outcome |
|---|---|
| Design pass (parent reference, workspace sharing, wait scheduling) | **Done** |
| `SessionKind::Dependent` | **Done** |
| `SessionStatus::Waiting` | **Done** |
| `Workspace::dependent` | **Done** |
| `sessionmgr-core::dependency::parent_readiness` (pure, unit-tested) | **Done** |
| Daemon-side wait scheduling (poller, not worker-side blocking) | **Done** |
| `--parent`/`--start-now` CLI flags, `start-now <id>` subcommand | **Done** |
| Race between a user's close and the poller's promote | **Closed**, via `Supervisor::dependent_lock` |
| Daemon-restart-while-waiting recovery | **Done**, own black-box test |
| Live verification against real Claude Code/Codex sessions | **Not done this pass** — see below |

## The design

### How a session references a parent

**A new field, not a `Session` payload on the enum, and not a fourth
`Workspace` variant.** `Session` gained `parent_id: Option<SessionId>`
and `wait_for_parent: bool`, alongside a new `SessionKind::Dependent`.
Considered and rejected: putting `parent_id` inside `SessionKind` as an
enum payload (`Dependent(SessionId)`) — every existing `match kind` that
only cares about `PlainTerminal`/`SameDirectory`/`Worktree` would have
had to destructure a value it does not use, for no benefit, the same
reasoning `agent: Option<AgentKind>` already follows as a plain field
next to `kind`.

**The workspace is shared, not re-derived.** `Workspace::dependent(&parent)`
copies the parent's `repo`/`cwd` and sets `branch: None`. That last part
is the load-bearing choice: `Workspace::owns_worktree()` is
`branch.is_some() && cwd != repo`, so a dependent session's own close
already can't merge, discard, or otherwise touch the shared worktree —
`dispose_workspace` no-ops on it and `teardown_status` always resolves to
`Closed`, with **zero new code**, because both already implement exactly
this rule for `SameDirectory` sessions (which also own no branch). A
dependent session does not get its own `.sessionmgr-worktrees/<id>` entry
at all; it runs in the parent's.

The parent must itself be `Worktree` or `Dependent` (chaining is allowed:
a dependent session's own workspace already points at whatever the
original worktree-owning ancestor's `cwd` is, so a grandchild just
inherits it transitively with no special-casing). `SameDirectory` and
`PlainTerminal` parents are rejected at creation — a same-directory
parent's workspace *is* the user's own repository, and there is nothing
there for a "dependent" session to mean beyond "another same-directory
session", and a plain terminal has no workspace to share at all.

### Wait-for-parent scheduling: the daemon polls, the worker never blocks

The two options PLAN.md's own priority note posed were "does the daemon
poll the parent's status, or does the worker itself block before
spawning its child process?" **The daemon polls — no worker is ever
spawned for a session that has not yet started.**

Concretely: `session_new` writes the child's record and, if
`wait_for_parent` and the parent has not finished, transitions it
straight to `SessionStatus::Waiting` and returns — no worker, no pids
recorded at all. A background daemon task
(`poll_parent_then_start`, one per waiting session) re-reads the parent's
status every two seconds via
[`sessionmgr_core::parent_readiness`](../../crates/sessionmgr-core/src/dependency.rs),
a pure function (same shape as `recovery::decide_recovery`) mapping the
parent's `SessionStatus` to `NotYet` / `Ready` / `Unavailable`. `Ready`
spawns the worker through the exact same path an ordinary session start
already uses (factored into `Supervisor::spawn_and_await_running`, reused
by three call sites: an immediate session, a promoted `Waiting` one, and
`start-now`); `Unavailable` fails the session straight to `Errored` with
no worker ever existing.

This was the deciding trade-off, reasoned through rather than assumed:

- **Worker-blocks was rejected** because it would have required
  restructuring `worker::run`'s tightly-sequenced startup (bind socket,
  publish status, spawn backend, wire the accept loop) to support a
  "socket bound but nothing spawned yet" phase — a real, late-bound
  `Backend` the worker doesn't have today — purely so a process that is
  going to sit idle for however long the parent takes exists at all. That
  buys nothing: a `Waiting` session has no output, no PTY, nothing for a
  live-attach to stream.
- **Daemon-polls fits the architecture that already exists.** The daemon
  already gates a worker's existence on preconditions it resolves itself
  — `prepare_workspace` (create the worktree first), hook install (write
  the config first) — both run to completion *before* `worker::spawn_detached`
  is ever called. Waiting for a parent is the same shape: one more
  precondition, evaluated daemon-side, before a worker is spawned at all.
  `worker.rs` needed **zero changes** for this feature.
- **The "workers survive the daemon" guarantee is not weakened**, because
  it was never claimed for a session with nothing running yet. If the
  daemon dies while a session is `Waiting`, there is no process to lose —
  the in-memory poller task simply stops, and `Supervisor::run`'s startup
  `reconcile_all` now also returns every session still recorded `Waiting`
  (deliberately left untouched by the ordinary adopt/crash pass — see
  below) so `run` can restart an equivalent poller for each one. Verified
  by its own black-box test
  (`a_waiting_sessions_poller_survives_an_unclean_daemon_restart`): a
  session created, the daemon `SIGKILL`ed, a replacement daemon started
  transparently by an ordinary `list`, and the child still starts once
  its parent finishes — proving the restart path, not just the happy
  path.

### `SessionStatus::Waiting` and recovery

A `Waiting` session has no worker by design — nothing has been spawned
yet — so `SessionStatus::expects_live_worker()` deliberately **excludes**
it. Without that exclusion, `decide_recovery` (Phase 1's crash-detection
rule) would mark every session still waiting on its parent `Crashed` the
instant the daemon restarted, which is exactly backwards: no worker is
the *correct* state for `Waiting`, not evidence of a fault. This is the
one place Phase 5 touches Phase 1's central persistence policy, and it
touches it by carving out an explicit exception rather than weakening the
existing rule for every other status.

### The one real race, and how it's closed

A `Waiting` session has two things that can act on it concurrently: the
background poller promoting it to `Running`, and a user closing it. Both
read-then-write `state.json` outside of a worker's exclusive-ownership
window (see `catalog.rs`'s ownership table), which the existing daemon
architecture never had to think about — every prior daemon-side write
happened either before a worker existed at all or after one was
conclusively dead. `Supervisor::dependent_lock` (one
`rusty_tokio::sync::Mutex<()>`, coarse-grained rather than per-session,
since promoting-out-of-`Waiting` is a rare, one-time-per-session event)
serializes "is this session still `Waiting`?" and "act on it" for both
sides, closing what would otherwise be a real orphan-process bug: the
poller spawning a real worker while a stale `session_close` overwrites
`state.json` with `Closed` and no recorded pids.

### CLI surface

```
sessionmgr new --parent <id> [--start-now] [--agent AGENT] [--no-pty] [-- <command>...]
sessionmgr start-now <id>
```

`--parent` implies `--kind dependent` (there is no way to type `--kind
dependent` directly — only `--parent` sets it, so the two can never
disagree) and rejects `--kind`/`--repo`/`--hooks` passed alongside it
explicitly, rather than silently ignoring them. `--hooks` in particular:
a dependent session shares its parent's directory, so any hook config the
parent installed with its own `--hooks` already applies to the child's
agent CLI — installing a second, `--hooks`-requested copy on top would be
a conflicting settings file in the same directory, not a helpful
addition.

Default `wait_for_parent: true`, matching CAPABILITIES.md's own framing
("the child can be configured to wait... with a start now override to
skip the wait"). `--start-now` sets it false at creation; `start-now <id>`
does the same thing later, to a session already `Waiting` — a small,
cheap addition once the lock-guarded promote path existed for the poller
to share, and it directly matches CAPABILITIES.md's own description of
"start now" as an action available on a listed, already-created session,
not only a creation-time flag.

"Dependent sessions" vs. "dependent terminal sessions" are **not two
mechanisms** — CAPABILITIES.md describes them separately, but they are
the same `SessionKind::Dependent` distinguished only by whether `--agent`
is given, an axis the type already has. This was the one meaningful
simplification found during the design pass: building a second kind or a
second flag for "terminal" would have duplicated the sharing/waiting
mechanism for no real difference in behavior.

## What was built

- `sessionmgr-core`: `SessionKind::Dependent`, `SessionStatus::Waiting`,
  `Session::parent_id`/`wait_for_parent`, `Workspace::dependent`, the new
  `dependency` module (`parent_readiness`, unit-tested against every
  `SessionStatus` variant).
- `sessionmgr-protocol`: `Request::SessionNew` gained `parent`/
  `wait_for_parent`; new `Request::SessionStartNow`; `SessionSummary`
  gained `parent` (so `list`/a future TUI grouping can show it).
- `sessionmgr-daemon`: `Supervisor::resolve_dependent_workspace` (the
  parent/workspace validation), `spawn_and_await_running` (factored out
  of `session_new`'s original tail, now shared by three start paths),
  `try_advance_waiting_session`/`poll_parent_then_start`/
  `fail_waiting_session` (the wait mechanism), `dependent_lock`, the
  `session_close` fast path for a still-`Waiting` session, `--parent`/
  `--start-now` CLI parsing, `start-now` subcommand, `sessionmgr list`'s
  new `PARENT` column.

## Live verification

Run against real repositories and a real daemon on this machine (Linux;
see "What is not done" for the Windows-specific caveat every prior phase
in this position has also carried):

1. `sessionmgr new --kind worktree` (parent, long-running) then
   `sessionmgr new --parent <id> --start-now -- <commit a file>`: the
   file landed in the **parent's** `.sessionmgr-worktrees/<parent-id>`
   directory, not a new one of the child's own; `sessionmgr list` shows
   `Dependent` and the parent's id in a new `PARENT` column.
2. `sessionmgr new --parent <id>` (no `--start-now`) against a still-
   running parent: the child is immediately `Waiting`, with no worker
   recorded (`"worker": null` in `state.json`). Closing the parent with a
   bare `close` (no disposition — the worktree stays) is followed, within
   the 2-second poll interval, by the child transitioning `Waiting` →
   `Running` on its own.
3. `start-now <id>` against a `Waiting` child promotes it immediately,
   regardless of the parent's status, and does not touch the parent.
4. Closing a still-`Waiting` session ends it `Closed` with no worker ever
   spawned and the parent's worktree untouched.
5. Closing the parent with `--discard` while the child is `Waiting`:
   within one poll interval the child is `Errored`, never `Running` —
   the parent's workspace is gone, so there was nowhere for it to start.
6. Creating a dependent session against an already-`Merged`/`Discarded`
   parent is rejected immediately, before any record is written, with an
   error naming the parent and what happened to it.
7. `--kind`/`--repo` alongside `--parent`, and `--start-now` without
   `--parent`, are all rejected with a usage error rather than silently
   picking one.
8. **The daemon-restart case**: a session created `Waiting`, the daemon
   killed uncleanly, a replacement daemon started transparently by an
   ordinary `list`, and the child still starts once its parent finishes —
   proving `reconcile_all`'s poller-restart path, not just that the happy
   path works when the daemon never goes away.

All eight scenarios above are `tests/dependent_sessions.rs` black-box
tests (9 total, including a same-directory-parent rejection test), driven
against the real compiled binary the same way every other phase's
black-box suite is.

## Tests

15 new unit tests (`sessionmgr-core`: 5 `session.rs` state-machine tests
for `Waiting`, 1 `workspace.rs` test for `Workspace::dependent`, 6
`dependency.rs` tests for `parent_readiness` against every
`SessionStatus`), 9 new black-box tests
(`crates/sessionmgr-daemon/tests/dependent_sessions.rs`). Workspace-wide
`cargo test` green (excluding the pre-existing, environment-gated
`agent_needs_input_claude`/`codex`/`gemini` tests, whose Claude Code
instance in this sandbox does not reach its trust prompt within the
existing 60-second window — confirmed unrelated to this phase by
reproducing the identical failure against unmodified `main`, not
something this phase's changes caused or should paper over).

`cargo build`/`clippy --workspace --all-targets -- -D warnings`/`fmt --all
--check`/`test --workspace` all green on this machine. `cargo check
--workspace --all-targets --target x86_64-pc-windows-msvc` and `cargo
+1.88 check --workspace --all-targets` both green as well.

## What is not done

**Manual verification on a real Windows box.** This environment is Linux,
same limitation every phase report through Phase 4b has carried for its
own new work; the cross-target `cargo check` above is not a substitute
for `cargo test --workspace` actually running on Windows. Nothing in this
phase touches PTY/ConPTY/process-detachment code at all — the whole
mechanism is daemon-side polling and `state.json` bookkeeping, the same
kind of surface Phase 2's worktree lifecycle work already proved portable
— but it has not been run on Windows and should not be reported as if it
had.

**Live verification against real Claude Code/Codex sessions specifically**
(as opposed to the `long_running`/`commit_a_file`/`echo` shell commands
the black-box tests use). Dependent sessions do not touch the agent
adapters at all — `--agent`/hooks/tier-1/tier-3 detection are unchanged
by this phase, since a dependent session is exactly as agent-aware as any
other `--agent`-flagged session once it starts — so this is lower-risk
than it would be for a genuinely new adapter surface, but it has not been
run.

**TUI grouping.** CAPABILITIES.md describes dependent sessions as
"grouped together in the sidebar." `SessionSummary.parent` is surfaced
over the wire and in `sessionmgr list`'s new `PARENT` column, which is
enough for a future TUI pass to build a grouped view on top of, but no
TUI-side grouping/rendering was built in this pass — matching Phase 4's
own precedent of scoping command-palette/hooks work separately from the
grid/panes slice that shipped first.

**Fork.** Explicitly not in this phase, per PLAN.md, and untouched here.
