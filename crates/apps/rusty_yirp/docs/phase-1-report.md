# Phase 1 report — walking skeleton and two spikes

Phase 1's stated scope (PLAN.md § Phased milestones):

> `SessionKind::PlainTerminal` only, Claude Code only, no worktree, no TUI.
> Proves the daemon/worker/detached-persistence loop
> (`supervisor_restart_recovery.rs` passing is the exit criterion) *and* runs
> the PTY-vs-piped-stdio spike and the Claude-Code-hooks-when-headless spike.

## Status

| Item | Outcome |
|---|---|
| Walking skeleton (daemon / worker / client) | **Done** |
| `supervisor_restart_recovery.rs` passing | **Done** — exit criterion met |
| Spike A: Claude Code hooks when headless | **Answered on Linux; Windows half open** |
| Spike B: PTY vs piped stdio | **Answered decisively — a real PTY is required** |
| Manual `taskkill` verification on Windows | **Not run** — no Windows machine available in this environment |

## Exit criterion

`supervisor_restart_recovery.rs` passes. It proves the product's central
promise against real OS processes rather than a simulation of them:

1. A session is created and confirmed running.
2. The daemon is killed **uncleanly** from outside (`SIGKILL`), with no
   graceful path — the way closing an app or crashing actually behaves.
3. The worker and the session's own process are asserted still alive.
4. A client transparently starts a replacement daemon.
5. The session still lists as `Running`, and **its worker pid is unchanged** —
   proving it was adopted, not silently respawned.

Two further tests in the same file cover reattachment (the recovery marker is
announced and the transcript replayed) and the graceful case (`daemon shutdown`
must also leave sessions running — an easy and very wrong "tidy-up" to add).

`worker_crash_recovery.rs` proves the other half: a worker that is genuinely
gone is reported `Crashed`, the recorded pid is unchanged and still dead, and
nothing is resurrected.

### Test totals

79 tests, all passing: 22 domain unit tests (`sessionmgr-core`), 26
adapter/composition unit tests, 8 process-adapter tests, 4 protocol tests, and
19 black-box subprocess tests driving the real compiled binary.

`cargo clippy --workspace --all-targets` is clean, and
`cargo check --workspace --all-targets --target x86_64-pc-windows-msvc`
compiles cleanly — including the `#[cfg(windows)]` arms of the process adapter
and the test harness.

## Spike A — do Claude Code's hooks fire when it is launched headless?

**The question** (PLAN.md risk 1, gating Phase 3): does Claude Code's hook
mechanism fire reliably when launched as a detached, non-console-attached child
from a Windows Rust process? Every public observation of this working was on
macOS with a normally-launched process.

**Method.** Deliberately run through this project's *own* machinery rather than
a synthetic reproduction, so the thing tested is the real configuration: a
project with `SessionStart` and `Stop` hooks in `.claude/settings.json`, run as
`sessionmgr new -- sh -c 'cd <proj> && claude -p "…"'`. That puts Claude Code
as the child of a detached worker process with piped stdio and no controlling
terminal — the exact shape Phase 3 depends on.

**Result: both hooks fired.**

```
/tmp/hs/sessionstart-hook-fired.txt
/tmp/hs/stop-hook-fired.txt
```

The session also exited 0 and was correctly recorded `Finished`, confirming
PLAN.md's **tier-2 signal** (process exit status) works end to end against a
real agent CLI, not just against `sh`.

**What this does and does not establish.** The question had two variables —
*headless/detached* and *Windows*. This answers the first: the hook mechanism
does not depend on a console, a TTY, or a normally-launched parent. The second
is untested, because this environment is Linux. That residual risk is
materially smaller than the original: the concern was that detachment might
suppress hooks, and detachment demonstrably does not.

**Unexpected finding, and it matters for Phase 3.** With stdin piped but never
written, Claude Code emits:

```
Warning: no stdin data received in 3s, proceeding without it.
```

So a piped-stdin adapter pays a silent 3-second penalty on every launch. The
Claude Code adapter should redirect stdin from the null device when using
`-p`, rather than leaving an idle pipe attached.

## Spike B — PTY or piped stdio?

**The question** (PLAN.md § Process supervision, "Open, unresolved-by-design
tension"): real ConPTY-backed PTY versus plain piped stdio, since CLIs like
Claude Code are suspected to change output behaviour when not attached to a
real terminal.

**Result: not a preference — a hard requirement. Piped stdio cannot host an
interactive agent session at all.**

Interactive `claude` (no `-p`) under piped stdio does not degrade gracefully;
it refuses:

```
Warning: no stdin data received in 3s, proceeding without it.
Error: Input must be provided either through stdin or as a prompt argument
       when using --print
exit code 1
```

It silently fell back to `--print` mode and died. The session was correctly
recorded `Errored`, but no interactive session existed at any point.

The identical command under a PTY renders the full interactive interface:

```
Welcome to Claude Code v2.1.233
Let's get started.
Choose the text style that looks best with your terminal
> 2. Dark mode ✓
```

**Consequences, which are real scope for Phase 2 and Phase 4:**

1. **A PTY is mandatory** for any session that hosts an interactive agent CLI.
   Phase 1's piped stdio is adequate for `PlainTerminal` running a
   non-interactive command, and is not adequate for the actual product.
   `rustils`' ConPTY-backed `WindowsPty` is the intended implementation, per
   PLAN.md, rather than adding `portable-pty` as a second PTY dependency.
2. **`SessionEvent::Output` must become bytes.** The PTY output above is dense
   with ANSI and cursor-positioning sequences. The current wire type is
   `String` with a per-chunk `from_utf8_lossy`, which mangles any multi-byte
   character split across a read boundary. That was documented as a known
   limitation in `worker::pump` pending this spike's answer; the answer is in,
   and it needs fixing before PTY sessions ship.
3. **Terminal emulation is now in scope for the TUI.** Rendering those
   sequences in a `ratatui` pane means interpreting them, not printing them.

**Still open, and untestable here**: whether a ConPTY-attached child survives an
unclean *worker* crash — not a graceful `ClosePseudoConsole` (PLAN.md risk 3).
`rustils`' PTY path was built for interactive foreground use. This needs a real
Windows machine and is a genuine blocker for PTY-backed sessions, though not
for anything Phase 1 shipped.

## Gate status for Phase 3

PLAN.md gates Phase 3's adapter work on Spike A's outcome. Spike A came back
**positive** on the mechanism itself, so the hook-based tier-1 design is not
invalidated and Phase 3 need not pre-commit to the degraded pattern-matching
path for Claude Code. The Windows-specific confirmation should be folded into
the Phase 2 Windows verification pass rather than blocking further work.

PLAN.md's other Phase 3 precondition — whether Codex and Gemini CLI have any
hook equivalent at all — remains entirely unresearched, exactly as its risk
list states. Neither CLI is installed in this environment.

## Deviations from PLAN.md, and why

1. **Integration tests live in `crates/sessionmgr-daemon/tests/`**, not at the
   workspace root as PLAN.md's tree shows. Purely mechanical: Cargo only sets
   `CARGO_BIN_EXE_sessionmgr` for tests in the package defining that binary,
   and a virtual workspace root has no package to hang tests off. Same tests,
   same harness.
2. **`sessionmgr-agents` and `sessionmgr-git` do not exist yet.** They are
   Phase 2/3 crates; creating them empty now would be building ahead of the
   phase.
3. **`SessionKind` has one variant.** `SameDirectory` and `Worktree` arrive in
   Phase 2 with the worktree lifecycle that gives them meaning. `--kind`
   already exists and rejects the other two with an explanatory message, so the
   interface does not change shape later.
4. **Readiness is the session record, not the worker socket.** The obvious
   design — probe the worker's socket after spawning — is wrong, and the test
   suite caught it: a session whose command exits immediately has a worker that
   already recorded the outcome and exited before the probe arrives, so `new`
   reported a connection failure for a session that ran perfectly. The worker
   now binds its socket *before* publishing `Running`, so a record past
   `Created` guarantees both "started" and "attachable".

## Beyond PLAN.md, adopted from the real reference sources

PLAN.md describes `rusty_prime_agent`'s `procutil.rs` as `prepare_detached` /
`is_alive` / `kill`. Reading the actual source showed it has since grown two
things the plan does not mention, both of which are ported here because they
fix real, already-observed bugs rather than hypothetical ones:

- **`is_same_process` with a process start-time fingerprint.** A bare pid check
  answers "does *a* process hold this number", which after pid reuse is a
  different question with the same answer. Without it, a supervisor declines to
  mark a genuinely dead worker as crashed, and the session wedges with nothing
  running and nothing noticing.
- **A zombie check on Unix.** A zombie answers `kill(pid, 0)` successfully.
  Without this, a just-exited worker reads as healthy for the whole window
  before it is reaped — precisely the window in which crash detection runs.
  This is not theoretical: it appeared during this phase's own manual
  verification, where a killed daemon still answered a shell `kill -0` probe
  while this project's own code correctly determined it was not running.

## Manual verification still owed on Windows

PLAN.md § Verification asks for one manual run on a real Windows box. It has
not been done — this environment is Linux. The Linux equivalent was run by hand
and passed (session created, daemon `SIGKILL`ed, worker confirmed alive, daemon
restarted, session still `Running` and reattachable). On Windows the same
sequence should be run with `taskkill /IM sessionmgr.exe`, along with
`cargo build --workspace` and `cargo test --workspace`, and the results appended
to ADR-0001 and this document.
