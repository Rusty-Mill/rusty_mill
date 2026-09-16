# Phase 4 report — the TUI (grid, panes, diff view)

Phase 4's stated scope (PLAN.md § Phased milestones):

> TUI (grid, panes, diff view, command palette) + session hooks/webhook
> extensibility, including the secret-scrubbing boundary and the
> `__hook-fire` no-op requirement.

## Status

| Item | Outcome |
|---|---|
| Grid layout, keyboard-driven resize | **Done** |
| Session panes (live terminal, real VT100 interpretation) | **Done** |
| Git diff pane | **Done** |
| Command palette | **Not done** — see below |
| Session hooks / webhook extensibility | **Not done** — see below |
| Live verification against a real daemon | **Done** |

This report covers the grid/panes/diff-view slice only. Command palette
and session hooks/webhooks are real, separately-scoped Phase 4 work not
attempted in this pass -- see "What is not done" rather than assumed
folded in.

## What was built

`sessionmgr-tui`, a new crate, wired into the `sessionmgr` binary as the
`tui` subcommand (`sessionmgr tui`, auto-starting a daemon like every
other client command). Depends on `sessionmgr-protocol` only, per
PLAN.md's ports-and-adapters boundary -- confirmed by the crate actually
building with no path to `sessionmgr-proc`/`sessionmgr-agents` in its
dependency graph.

- **`client.rs`**: the TUI's own line-delimited-JSON-over-`AF_UNIX`
  client. Not a reuse of `sessionmgr-daemon`'s `transport.rs`/`client.rs`
  -- that dependency direction is circular (the daemon binary depends on
  `sessionmgr-tui` to serve `tui`, so `sessionmgr-tui` cannot depend back
  on `sessionmgr-daemon`). The small duplication is the actual
  architectural boundary the plan calls for, not an oversight.
- **`panes/terminal_pane.rs`**: wraps a `vt100::Parser` and renders
  through `tui-term`'s `PseudoTerminal` widget. This is the one place in
  the TUI that touches raw terminal bytes at all -- everywhere else only
  ever sees the already-interpreted `vt100::Screen`, per ADR-0002.
- **`panes/session_pane.rs`**: a session's terminal plus a title bar
  (id, status, kind).
- **`panes/git_diff_pane.rs`**: changed-files list beside the selected
  file's unified diff. No syntax highlighting, matching PLAN.md's own
  stated v1 scope.
- **`grid.rs`**: pure layout math (`Rect` in, `Vec<Rect>` out, no
  `ratatui::Frame`), unit-tested the same way `sessionmgr-core`'s state
  machine is -- 10 tests covering pane-count-to-grid-shape, weight-sum
  invariants, grow/shrink symmetry, and the resize floor.
- **`app.rs`**: the event loop, keybindings, and session-list polling
  (every 2s, so sessions created/closed from outside the TUI appear and
  disappear without a restart).
- **Protocol additions**: `Request::GitStatus`/`GitDiff` and their
  `Response` variants, plus re-exporting `SessionId`/`SessionKind`/
  `SessionStatus`/`Disposition`/`ChangedFile` from `sessionmgr-protocol`
  so a protocol-only crate can actually name them.
- **Daemon**: `session_git_status`/`session_git_diff` handlers, using
  `SystemGit` under `spawn_blocking` -- the same reasoning as
  `prepare_workspace`/`dispose_workspace` (see `supervisor.rs`'s own
  comments): a git subprocess call is real, synchronous OS work and does
  not belong inline on the async executor.

## Keybindings: a prefix key, not per-key bindings

Every ordinary keystroke reaches the focused session as real terminal
input. That is the actual requirement for a multiplexer -- a user who
cannot send Ctrl-D or Ctrl-C to their own shell or agent CLI has lost
real functionality, not gained convenience. So, tmux-style, exactly one
combination is reserved: `Ctrl-B`. The key immediately following it is a
command (`n`/`p` focus, arrows resize, `g` diff, `q` quit), consumed
whether recognized or not. This was a deliberate choice over binding
individual `Ctrl-<letter>` combinations directly (the more obvious first
design) specifically because any such binding permanently steals that
control character from every session's shell.

## A real, load-bearing discovery about `rusty_tokio::select!`

Not in PLAN.md or either ADR, and worth recording next to the code (it is,
in `app.rs`'s own comments) rather than only here: `rusty_tokio::select!`
evaluates each branch's body **inside** the `poll_fn` closure driving the
race -- a plain synchronous closure, unlike real tokio's `select!`, which
runs the winning branch's body outside the polling machinery. A branch
body containing `.await` does not compile.

The fix is structural, not a workaround: branches only ever produce a
plain value (this crate's `Woken` enum), and anything requiring `.await`
happens in a `match` after `select!` has already resolved. Worth
generalizing if a second `select!` call site appears anywhere else in
this codebase -- it currently does not.

## Live verification

Not just "it compiles" -- run against a real daemon on this machine, PTY
by PTY, via `hub`'s process control (`daemon start`, two real sessions --
one `worktree`, one `terminal` -- then `sessionmgr tui` launched under its
own PTY and driven with real keystrokes):

1. **Rendering**: two bordered panes side by side, each with a title bar
   (`id [status] kind`), live PowerShell banner/prompt output in both.
2. **ADR-0002's actual requirement**: one session's prompt was a custom
   PowerShell function emitting a raw `\x1b[32m` (green) escape sequence.
   It never appeared as literal escape-code text in the rendered pane --
   direct evidence the bytes went through `vt100`'s state machine rather
   than being printed.
3. **Diff pane**: `Ctrl-B g` on the worktree session replaced its cell
   with "Changed files" / "Diff" sub-panels, populated by a real
   `Request::GitStatus` round trip through the daemon's new handler.
4. **Resize**: `Ctrl-B` + Right visibly grew the focused pane's column
   and reflowed the neighboring pane's wrapped text at the new, narrower
   width.
5. **Keystroke forwarding**: ordinary typed characters appeared verbatim
   inside the focused session's own prompt line, echoed by PowerShell
   itself -- proof input reaches the right session's PTY, not just that
   the TUI accepts keypresses.
6. **Quit leaves sessions running**: `Ctrl-B q` exited the TUI process
   (`exit=0`); `sessionmgr list` immediately after showed both sessions
   still `Running`. The whole point of the architecture, exercised
   through the newest client role rather than assumed to still hold.

## What is not done

**Command palette** (PLAN.md's Cmd+K-equivalent). Real, separately-scoped
work -- a fuzzy action launcher over a command registry -- not attempted
here. The prefix-key scheme above covers the keybindings this pass
needed; a palette is additive on top of it, not a prerequisite for
anything shipped in this report.

**Session hooks / outbound webhooks** (PLAN.md's other Phase 4 half,
`docs/plan/PLAN.md` § "Session hooks / extensibility"). Entirely separate
surface area -- installing Claude Code hook config, `__hook-fire`,
outbound webhook dispatch, the secret-scrubbing boundary -- none of it
touched in this pass. Phase 3 (the adapters that would actually install
and fire these hooks) is also not done yet, and hooks without an adapter
to install them for is work with no consumer.

**Mouse-driven resize, embedded browser, in-app file editor**: all
explicitly deferred by PLAN.md itself (keyboard-driven resize instead of
mouse-drag, a "detected: localhost:8000" signal instead of an embedded
browser, "open in $EDITOR" instead of an in-app editor) -- not revisited
here, and not this report's scope to revisit.

## Tests

13 new unit tests (`sessionmgr-tui`): 10 in `grid.rs` (pane-count-to-shape,
weight invariants, grow/shrink, resize floor, out-of-range lookups), 3 in
`git_diff_pane.rs` (selection wraparound, empty-list no-op navigation,
selection clamping when the file list shrinks). No black-box subprocess
tests for the TUI itself -- driving a real terminal UI through
`tests/common`'s pattern would mean scripting `crossterm` input against a
`TestBackend`, real scope not attempted here; the live-verification pass
above is this phase's acceptance evidence instead, the same role
`supervisor_restart_recovery.rs` plays for the daemon.

`cargo build`/`clippy --all-targets -- -D warnings`/`fmt --all --check`/
`test --workspace` all green on real Windows.
