# ADR-0002: A real PTY is required for agent-CLI sessions

- **Status**: Accepted — **implemented**
- **Date**: 2026-08-16
- **Phase**: 1 (spike outcome)
- **Supersedes**: PLAN.md's "open, unresolved-by-design tension" between a real
  PTY and plain piped stdio

## Context

PLAN.md left the PTY question deliberately open, to be settled by a Phase 1
spike rather than assumed either way:

> real PTY (`rustils`' `platform-windows::pty::WindowsPty`, ConPTY-backed --
> likely necessary since CLIs like Claude Code change output behavior when not
> attached to a real terminal) vs. plain piped stdio.

## Decision

**Sessions that host an interactive agent CLI must be PTY-backed.** Piped stdio
is retained only for non-interactive commands.

## Evidence

Measured, not inferred. Full method and output in
[`docs/phase-1-report.md`](../phase-1-report.md) § Spike B.

Interactive `claude` under piped stdio does not merely change its output
formatting — it **refuses to run**. It silently falls back to `--print` mode and
exits 1:

```
Error: Input must be provided either through stdin or as a prompt argument
       when using --print
```

The same command under a PTY renders the full interactive interface.

So the premise in PLAN.md ("changes output behavior") understated it: this is
not a fidelity trade-off, it is a hard functional requirement.

## Consequences

1. `rustils`' ConPTY-backed `WindowsPty` becomes a real dependency of the
   worker role, as PLAN.md anticipated — and not `portable-pty`, which would be
   a second, redundant PTY implementation.
2. **`SessionEvent::Output` must change from `String` to bytes.** PTY output
   carries ANSI and cursor-positioning sequences, and the current per-chunk
   `String::from_utf8_lossy` in `worker::pump` mangles multi-byte characters
   split across a read boundary. That limitation was documented in the code
   pending this spike; it now needs fixing.
3. The TUI's session panes must *interpret* terminal sequences, not print them
   — terminal emulation is in scope for Phase 4 in a way it previously was not.
4. The Claude Code adapter should redirect stdin from the null device when
   using `-p`, rather than attaching an idle pipe: an unwritten stdin pipe
   costs a silent 3-second startup delay and a warning on every launch.

## Implementation

Done. `sessionmgr-pty` wraps `rustils`' `platform::pty::Pty` capability —
ConPTY on Windows, `openpty` on Linux — rather than adding `portable-pty` as a
second PTY implementation to a dependency graph that already had one. Sessions
run on a terminal by default; `--no-pty` selects the piped backend.

All four consequences above are implemented:

1. `sessionmgr-pty` exists and is the default backend.
2. `SessionEvent::Output` and `SessionInput` are `Vec<u8>`, carried as base64
   because the framing is line-delimited JSON.
3. Terminal-sequence interpretation remains Phase 4's problem; the bytes now
   reach the client intact for it to interpret.
4. Not yet applied — the Claude Code adapter is Phase 3, and there is no
   adapter to put it in.

`a_session_runs_on_a_real_terminal_by_default` is the acceptance test: a
session runs `test -t 1` and its own output must say `IS_A_TTY`. Its
counterpart asserts `--no-pty` really does produce `NOT_A_TTY`.

### Why the piped backend was kept rather than deleted

Not hedging. The survives-the-manager-closing guarantee is **proven** for the
piped path on Windows by a green suite. It is **unproven** for ConPTY. Removing
the proven path to make room for the unproven one would trade a demonstrated
guarantee for an assumed one, in the one area where this project's whole value
rests.

## Resolved: ConPTY survives an unclean daemon kill

**Answered, by measurement.** This was PLAN.md risk 3 and the last unknown in
this ADR: `rustils`' PTY path was built for interactive foreground use, not for
detach-and-outlive, so whether a ConPTY-attached child survives its supervisor
being killed uncleanly was genuinely open.

The question was routed through the tests that already existed rather than a
bespoke spike. Sessions default to a PTY, so `supervisor_restart_recovery` —
create a session, `TerminateProcess` the daemon, assert the worker *and its
child* are still alive, assert the replacement daemon adopts them rather than
respawning — became the ConPTY-survival test automatically.

`test (windows-latest)` passed the full suite on
[run 31970983489](https://github.com/baileyrd/rusty_yirp/actions/runs/31970983489),
on `windows-latest` against real ConPTY.

**So the survives-the-manager-closing guarantee holds for the default PTY
backend**, and the default stays PTY. The `--no-pty` path is no longer
insurance against this specific unknown; it is kept for non-interactive
commands, where a terminal buys nothing.

Two honest caveats. This is one CI run on one Windows image, not a soak test —
but it now runs on every push, so a regression surfaces immediately rather than
during a manual pass. And GitHub's `windows-latest` is a server image; a
desktop Windows install with different console-host behaviour is not the same
environment, which is why the manual verification on a real dev machine stays
on the list.
