# Decision needed: liveness/zombie probe + detached spawn (agent-harness brief)

Not a design document — a decision request, the same shape
`docs/decision-request-fork-execve.md` and
`docs/decision-request-msys-parity.md` use. Opened against an external
brief (2026-08-11) asking for three "platform primitives" for a
not-yet-started daemon-backed agent-harness consumer: (1) process-group-
or-single kill + liveness + zombie detection, (2) detached spawn, (3) a
`LocalListener` trait unifying Unix sockets and Windows named pipes. This
document is scoped to what that brief actually turned out to need once
checked against the current tree — most of it already exists.

## Outcome

**Decided 2026-08-11, owner override of RFC v2 §3's consumer gate** (the
harness is real per the brief but not yet started, not yet in this repo —
normally that gets a table row, not code; the owner explicitly chose to
implement now rather than wait for the gate). Scope, cut to the two
genuine gaps:

1. **`Spawner::is_alive`/`is_zombie`** — new `Spawner` methods, portable,
   `Unsupported` on Windows for `is_zombie` (divergence **015**).
2. **`Command::detach()`** — new spawn-time flag. Composes **only** with
   `GroupSpec::Inherit`. `NewGroup` is **refused on both backends**
   (`Unsupported`) — this section originally proposed Linux composing
   `detach()` with `NewGroup` (reasoning: `setsid`+`setpgid(0,0)` is
   POSIX-specified-harmless self-targeting); **that reasoning was wrong**,
   caught by CI, not by review: a real `posix_spawn` call with both
   `POSIX_SPAWN_SETSID` and `POSIX_SPAWN_SETPGROUP` set fails `EPERM`
   every time, because Linux's `setpgid(2)` forbids changing a session
   leader's process group ID at all, even to itself — `setsid` always
   makes the caller a session leader first, so the two flags can never
   coexist. Windows independently refuses the same combination for an
   unrelated reason (a kill-on-close Job Object would defeat `detach`'s
   entire purpose the instant the spawning process exits for any reason,
   including a crash). Both backends now agree, so this is documented in
   `docs/behavior/process.md`, not `docs/divergences.md` — that registry
   is for genuine cross-backend differences, and refusing uniformly
   isn't one. Refused with `JoinGroup` on **both** backends: `setsid`
   creating a new session is incompatible with joining an existing,
   different pgid — not an OS limitation, a self-contradictory request
   (`InvalidInput`).

**Also caught only by CI, unrelated to the design decision above:** a
`platform-linux`-only `clippy::too_many_arguments` finding and an
`E0282` type-inference gap in `sys::spawn::spawn`'s combined
`POSIX_SPAWN_SETPGROUP`/`_SETSID` flags word — both invisible in this
session's own local `cargo clippy`/`cargo check` runs, because
`platform-linux`'s crate root is `#![cfg(target_os = "linux")]` and
compiles to an empty crate on the Windows sandbox that authored this
change. And a genuine pid-reuse race in
`windows_is_alive_reports_running_then_exited`: the test originally
probed `is_alive` after the *consuming* `Child::wait`, which closes the
process handle the instant it returns — reopening the exact race
Windows's own "a pid is never reused while a handle to it stays open"
guarantee exists to prevent, made likely in practice by `cargo test`'s
parallel test threads spawning/killing many other real processes at
once. Fixed by probing while still holding the handle open via the
non-consuming `try_wait` instead of `wait`.

**Not built** — already shipped, would duplicate existing surface:

- Process-group-or-single kill: `Child::kill_tree`/`kill_single`,
  `GroupHandle` (`crates/platform/src/process.rs:237,321`). Windows
  already uses the "real Job Object" branch the brief poses as an open
  question (`crates/platform-windows/src/sys/proc.rs:287-297,419-553` —
  suspended-spawn → kill-on-close-Job-assign → resume).
- `LocalListener`/named pipes: `platform::net::Net::unix_connect`/
  `unix_listen`, `UnixStream`/`UnixListener`, shipped **Stable** on Linux
  *and* Windows (native Winsock `AF_UNIX`, not a named-pipe emulation —
  `crates/platform-windows/src/sys/net.rs:683-812`,
  `docs/behavior/net.md:3-6,52-113`). No named-pipe backend exists or is
  needed; the brief's own stated worry (`if win32` branching pain) is
  already avoided by using one address family on both OSes.

## Where this stands today (checked against the tree, not the brief's own framing)

- Repo is **not** greenfield: `rustils` v0.26.0, governed by
  `docs/rfc-v2.md`, consumer-gated (§3), with `process`/`net` both
  **Active**/**Stable**.
- `rusty_libc` is not an in-repo roadmap doc — it is the external sibling
  `baileyrd/rusty_libc`, pinned as a git dependency behind the `track-p`
  feature (`crates/platform-linux/Cargo.toml:15-25`), consumed for
  `kill`/`killpg`/`pidfd_open`/etc. (RFC §7.3 O-2/D-12). There is nothing
  there to check plan conflicts against; the relevant governance is RFC
  §3's table plus the surfaces actually shipped.
- `Child::try_wait` (`platform-linux/src/sys/spawn.rs:601-638`, `WNOHANG`)
  is the closest existing thing to a liveness probe, but it requires
  owning a `Child` (`&mut self`) and conflates "still running" with
  "reap". No standalone probe over a bare `pid: u32` exists — flagged as
  a real gap in `docs/design-discussion-msys-pgid-table.md`'s own text,
  never built (no consumer).
- No `setsid`-general/`CREATE_NEW_PROCESS_GROUP`/`DETACHED_PROCESS`
  exists anywhere in the tree (repo-wide grep, zero hits outside PTY's
  narrow, internal `POSIX_SPAWN_SETSID` use for session-leader wiring —
  `crates/platform-linux/src/sys/pty.rs:15-19,176`).
- The repo is deliberately sync/std-only (MSRV 1.75); pulling an async
  runtime for this is explicitly rejected precedent elsewhere
  (`crates/platform/src/term.rs:14-18`) — moot for these two primitives,
  neither needs one.

## Options

### Liveness / zombie

1. **A free function keyed by raw `pid: u32`, outside any trait.**
   Cheapest to write, but this repo's whole discipline is object-safe
   backend traits + `platform-mock` as the injectable double (RFC §5.1,
   §9.4: "the mock backend carries the bulk of consumer-logic coverage").
   A free function can't be scripted through the mock. **Not chosen.**
2. **`Spawner::is_alive`/`is_zombie(&self, pid: u32) -> Result<bool>`.**
   Same object-safe shape as the existing pid-keyed `Spawner::adopt`
   (`process.rs:413`) sitting right next to it — mock-testable,
   consistent placement, no new trait. `is_zombie` is `Unsupported` on
   Windows (no zombie concept — divergence **015**, same honest-refusal
   pattern `wait_job`/`try_wait_job` already use for Windows's missing
   stop/continue analog). **Chosen.**
3. **Fold zombie detection into `is_alive` as a three-state enum**
   (`Dead`/`Zombie`/`Running`). Rejected: `is_alive`'s Unix `kill(pid, 0)`
   semantics and `is_zombie`'s Linux-only `/proc/<pid>/stat` read are
   different OS mechanisms with different failure/availability shapes
   (`kill(pid,0)` exists everywhere this crate targets; `/proc` parsing
   doesn't need to exist on Windows at all except as an honest refusal).
   Two narrow methods over one broad enum matches this crate's existing
   granularity (`wait` vs `wait_job`, `kill_tree` vs `kill_single`).
   **Not chosen.**

### Detached spawn

1. **`Command::detach()` boolean, unconditionally composable with every
   `GroupSpec`.** Rejected after checking divergence 002's own kill-on-
   close semantics: Windows's `NewGroup` Job Object is torn down (killing
   every member) the instant every handle to it closes, which happens
   unconditionally when the OS reaps a terminated/crashed process's
   handles — `detach()` promising "survives even a parent crash" while
   silently still being tied to a Job Object would be a footgun, not a
   feature. **Not chosen.**
2. **`Command::detach()`, refused with `GroupSpec::NewGroup` on Windows
   only, composable with it on Linux.** The version originally chosen
   here, on the reasoning that `setsid`+`setpgid(0,0)` is
   POSIX-specified-harmless self-targeting. **Retracted**: a real
   `posix_spawn` call proved that reasoning wrong — Linux's
   `setpgid(2)` forbids changing a session leader's process group ID at
   all (not just to a *different* group; `setpgid(0, 0)` targeting its
   own current group still fails), and `setsid` always makes the caller
   a session leader first, so `POSIX_SPAWN_SETSID` and
   `POSIX_SPAWN_SETPGROUP` can never coexist in one `posix_spawn` call
   regardless of the target pgid. **Not chosen** (superseded by option 3
   below, discovered only after this option shipped once and failed CI).
3. **`Command::detach()`, refused with `GroupSpec::NewGroup` on **both**
   backends, and with `GroupSpec::JoinGroup` on both backends**
   (`InvalidInput` for `JoinGroup` — self-contradictory, not an OS
   limitation; `Unsupported` for `NewGroup` — a real OS limitation on
   each backend, just a different one). A caller that wants "detached,
   and later killable as a tree" uses the existing two-step path this
   repo already ships on either OS: `detach()`-only spawn — which, on
   Linux, already gives the child its own pgid via `setsid`, so
   `kill_tree` needs no `NewGroup` at all — then `Spawner::adopt(pid)`
   when it actually wants to kill it, a conscious choice at kill time,
   not an accidental side effect of spawn-time flags. **Chosen.**
4. **Two independent booleans (`no_console` / `new_process_group`)
   instead of one `detach()`.** Matches Win32's own two flags
   (`DETACHED_PROCESS`/`CREATE_NEW_PROCESS_GROUP`) more literally, but no
   consumer (real or the brief's own stated one) has asked for
   console-visibility and Ctrl-C-group-membership to vary independently
   — the brief's whole ask is "survive parent exit and terminal close",
   which both flags serve identically. Splitting them now is exactly the
   speculative-surface-without-a-forcing-need §3 exists to prevent.
   **Not chosen** — revisit only if a real caller needs one without the
   other.

## API shapes decided

```rust
// crates/platform/src/process.rs

impl Command {
    /// Detach the child from this process's session/console: it survives
    /// this process exiting (including crashing) and its terminal
    /// closing. Unix: `POSIX_SPAWN_SETSID` — the child becomes a new
    /// session **and** process-group leader (`pid == sid == pgid`)
    /// before its first instruction runs. Windows:
    /// `CREATE_NEW_PROCESS_GROUP | DETACHED_PROCESS` — no console, not a
    /// member of this process's Ctrl-C group.
    ///
    /// Composes **only** with `GroupSpec::Inherit`. Refused
    /// (`InvalidInput`) with `GroupSpec::JoinGroup` on every backend: a
    /// fresh session cannot also join an existing, different pgid.
    /// Refused (`Unsupported`) with `GroupSpec::NewGroup` on **every**
    /// backend too, for two unrelated real reasons: Linux — `setsid`
    /// always makes the child a session leader, and `setpgid(2)` forbids
    /// changing a session leader's process group ID, even a
    /// self-targeting `setpgid(0, 0)` no-op, so `posix_spawn` fails
    /// `EPERM` outright (confirmed against a real kernel); Windows — a
    /// kill-on-close Job Object would defeat `detach`'s "survives a
    /// crash" guarantee the instant every handle to it closes, which the
    /// OS does unconditionally when this process terminates.
    #[must_use]
    pub fn detach(mut self) -> Self { self.detached = true; self }
}

pub trait Spawner {
    // ...existing methods...

    /// Is `pid` currently alive? Not tied to any `Child` this backend
    /// spawned — works for any pid, including ones this process has no
    /// wait/kill relationship with (a detached worker's pid recorded
    /// earlier, or a third-party pid). Unix: `kill(pid, 0)` — `Ok(true)`
    /// on success *or* `EPERM` (exists, just not signalable by us),
    /// `Ok(false)` on `ESRCH`. Windows: `OpenProcess` +
    /// `GetExitCodeProcess`; `Ok(true)` if `STILL_ACTIVE` or the open
    /// itself failed with anything other than "no such process";
    /// `Ok(false)` otherwise. A zombie (Linux, unreaped) reports `true`
    /// here — `kill(pid, 0)` cannot tell "running" from "exited but not
    /// yet reaped" apart; see `is_zombie` for that distinction.
    fn is_alive(&self, pid: u32) -> Result<bool>;

    /// Is `pid` a zombie (exited, not yet reaped by its real parent)?
    /// Linux: `/proc/<pid>/stat`'s state field == `Z`; a missing `/proc`
    /// entry means the pid is gone entirely, reported as `Ok(false)`
    /// (not a zombie — it doesn't exist), not an error. Windows:
    /// `Unsupported` — there is no zombie concept; an exited process's
    /// handle stays valid and its exit code re-readable indefinitely
    /// (see `Child::try_wait`'s own doc comment), so the question this
    /// method asks has no Windows answer to give, honest per divergence
    /// **015**.
    fn is_zombie(&self, pid: u32) -> Result<bool>;
}
```

## Divergence registry additions

- **015 — no zombie concept on Windows**: `Spawner::is_zombie` is real on
  Linux (`/proc/<pid>/stat`), `Unsupported` on Windows — a process
  handle stays valid and queryable after exit with no distinct
  "unreaped" state to observe.

`Command::detach` + `GroupSpec::NewGroup` was drafted as a second new
divergence in this same slice — allowed on Linux, refused on Windows —
then retracted before this branch merged: CI proved the Linux half
wrong (real kernel `EPERM`), so both backends refuse the combination
now, and uniform refused behavior across backends isn't divergence
material (the registry is for where backends genuinely differ). See
`docs/behavior/process.md`'s `Command::detach()` entry instead.

## Open questions for the owner

None outstanding — both primitives are scoped and implemented. Two
things worth recording as the actual close-out, not the
optimistic pre-implementation state this section originally described:

- **The Linux `detach()` + `NewGroup` design was wrong on first pass,
  corrected by CI, not by review.** This sandbox (Windows workstation,
  no working WSL distro) could not compile or test `platform-linux` at
  all — its crate root is `#![cfg(target_os = "linux")]`, so it no-ops
  under `cargo check`/`clippy` on Windows — so the "harmless
  self-targeting `setpgid(0, 0)`" reasoning went unverified until GitHub
  Actions' `ubuntu-latest` legs ran a real `posix_spawn` call and it
  failed `EPERM`. The design was corrected in the same PR before merge;
  flagging this because it's exactly the failure mode this document's
  own original "unverified beyond visual review" caveat warned about,
  now with a concrete example rather than a hypothetical.
- Windows changes are compiled, clippy-clean (including the `track-w`
  leg), and live-tested end to end via GitHub Actions CI, not just this
  sandbox's local runs — CI also caught a pid-reuse race in one Windows
  test (`windows_is_alive_reports_running_then_exited`, fixed by probing
  via the non-consuming `try_wait` instead of the handle-closing
  `wait`) that this sandbox's own local runs never reproduced.
