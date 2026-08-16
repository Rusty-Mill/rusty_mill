# ADR-0002: A real PTY is required for agent-CLI sessions

- **Status**: Accepted
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

## Still open

Whether a ConPTY-attached child survives an **unclean worker crash** — as
opposed to a graceful `ClosePseudoConsole` — is unverified and untestable off
Windows (PLAN.md risk 3). `rustils`' PTY path was built for interactive
foreground use, not for detach-and-outlive. This must be settled on a real
Windows machine before PTY-backed sessions ship, because a "no" would mean
PTY-backed sessions cannot deliver the survives-the-manager-closing guarantee
that the piped-stdio sessions built in Phase 1 already demonstrably do.

That is a genuinely load-bearing unknown, and it is the reason this ADR changes
the *decision* about PTYs without yet changing any Phase 1 code: the walking
skeleton's guarantee is proven, and it should not be traded away for a PTY
until the PTY is proven to keep it.
