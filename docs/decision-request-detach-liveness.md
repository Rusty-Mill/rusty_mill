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
   `Unsupported` on Windows for `is_zombie` (divergence **016**).
2. **`Command::detach()`** — new spawn-time flag. Composes with
   `GroupSpec::NewGroup` on Linux (harmless-redundant `setsid`+
   `setpgid(0,0)`, both POSIX-specified); **refused** in that same
   combination on Windows, because a kill-on-close Job Object would
   silently defeat `detach`'s entire purpose the instant the spawning
   process exits for any reason, including a crash (divergence **015**).
   Refused with `JoinGroup` on **both** backends: `setsid` creating a new
   session is incompatible with joining an existing, different pgid — not
   an OS limitation, a self-contradictory request (`InvalidInput`).

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
   Windows (no zombie concept — divergence **016**, same honest-refusal
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
   (divergence 015) and with `GroupSpec::JoinGroup` on both backends
   (`InvalidInput` — self-contradictory, not an OS limitation).** Linux
   keeps `detach()` + `NewGroup` composable: `setsid` already gives the
   child its own pgid (`pid == pgid`, guaranteed by POSIX, no drop-side
   kill-on-close mechanism exists on Unix to fight it — divergence 002's
   own "Linux: process keeps running" line already establishes this). A
   caller that wants "detached, and later killable as a tree" on Windows
   uses the existing two-step path this repo already ships:
   `detach()`-only spawn, then `Spawner::adopt(pid)` when it actually
   wants to kill it — a conscious choice at kill time, not an accidental
   side effect of spawn-time flags. **Chosen.**
3. **Two independent booleans (`no_console` / `new_process_group`)
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
    /// before its first instruction runs, the same race-free
    /// before-first-instruction guarantee `GroupSpec::NewGroup` already
    /// gives group placement (`docs/design-discussion-pty.md`'s
    /// `posix_spawn`-substitute precedent, generalized from PTY-only to
    /// general use). Windows: `CREATE_NEW_PROCESS_GROUP | DETACHED_PROCESS`
    /// — no console, not a member of this process's Ctrl-C group.
    /// Refused (`InvalidInput`) with `GroupSpec::JoinGroup`: a fresh
    /// session cannot also join an existing, different pgid. Refused
    /// (`Unsupported`) with `GroupSpec::NewGroup` **on Windows only**
    /// (divergence 015): a kill-on-close Job Object would defeat
    /// `detach`'s "survives a crash" guarantee the instant every handle
    /// to it closes, which the OS does unconditionally when this process
    /// terminates. Combines cleanly with `NewGroup` on Linux — see
    /// `Child::kill_tree`'s doc comment.
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
    /// **016**.
    fn is_zombie(&self, pid: u32) -> Result<bool>;
}
```

## Divergence registry additions

- **015 — `detach()` + `GroupSpec::NewGroup` composability**: allowed on
  Linux (harmless-redundant `setsid`+`setpgid(0,0)`, no drop-side kill
  mechanism to fight it), refused `Unsupported` on Windows (kill-on-close
  Job Object would defeat `detach`'s guarantee). OS limitation, not
  convenience: Windows's only tree-kill primitive (Job Objects) is
  inherently handle-lifetime-coupled; Linux's (`kill(-pgid, sig)`) is not.
- **016 — no zombie concept on Windows**: `Spawner::is_zombie` is real on
  Linux (`/proc/<pid>/stat`), `Unsupported` on Windows — a process
  handle stays valid and queryable after exit with no distinct
  "unreaped" state to observe.

## Open questions for the owner

None outstanding — both primitives are scoped, the API shapes above are
final pending implementation review, and the two genuinely-new
divergences are pre-recorded rather than discovered after the fact.
Implementation follows in the same session; Linux changes cannot be
compiled or tested in this sandbox (Windows workstation, no working WSL
distro — `wsl --status` shows the FedoraLinux-43 disk failing to attach)
and are therefore **unverified beyond visual review against the existing
call sites they mirror**; Windows changes are compiled and unit-tested
natively. Flagging this explicitly per the brief's own instruction not to
silently skip a platform's test coverage — here it is Linux compilation/
test coverage that is unavailable in this environment, the mirror image
of the brief's own Windows-CI caveat.
