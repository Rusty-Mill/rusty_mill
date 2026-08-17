# Phase 8 report — a real (graphical/desktop) front end

Not part of PLAN.md's original phased milestones -- this phase exists
because of a real, explicitly identified gap: `sessionmgr-tui` (Phase
4/4b) is the only UI this tool has ever had, and it is a terminal UI.
Issue #23 ("A real (graphical/desktop) front end for sessionmgr") was
filed to track that gap after a direct user question ("So do we have a
front end for this?") during Phase 6/7's live-verification work made the
honest answer -- no, only a terminal one -- worth recording rather than
leaving implicit. This report covers closing it: `sessionmgr-desktop`, a
Tauri-based graphical client with full interactive parity with the TUI
(the user's own explicit scope choice over a smaller read-only viewer),
built and live-verified.

## Status

| Item | Outcome |
|---|---|
| New crate `sessionmgr-desktop` (Tauri v2, protocol-only dependency) | **Done** |
| Live session grid: real xterm.js panes, real keyboard passthrough | **Done, live-verified** |
| Command palette (New/Close/Rename/Fork/SwitchAgent/Toggle diff/Focus) | **Done, live-verified** |
| Git diff pane | **Done, live-verified** |
| Daemon auto-start (no `sessionmgr tui`-style CLI wrapper to do it) | **Done, live-verified** |
| Drag-resizable grid (row/col weights, ported from `sessionmgr-tui::grid`) | **Done** -- not live-verified (see "What is not done") |
| Live verification | **Done** -- real window, real daemon, real keystrokes, under Xvfb in this environment |

## The design

### Architecture: a fourth client role, not a rewrite

`sessionmgr-desktop` is structured exactly the way `docs/phase-4-report.md`
frames `sessionmgr-tui`: a client that depends on `sessionmgr-protocol`
only, speaking the same line-delimited-JSON-over-`AF_UNIX` protocol every
other client role already speaks, with **zero daemon-side changes**. The
architecture predicted this would be tractable (`sessionmgr-tui`'s own
`client.rs` module docs call out the acyclic-dependency reasoning
explicitly, precisely so a future client could be added the same way),
and this phase is the confirmation: no new `Request`/`Response` variant,
no new daemon handler, nothing in `sessionmgr-core`/`sessionmgr-agents`
touched at all. The entire diff is a new crate.

### Why Tauri, and why real `tokio` inside it is a deliberate exception

The user chose Tauri (native webview + Rust backend) over egui/iced --
richer UI on the same terminal-content problem this app fundamentally
has (rendering a VT100 stream), and `xterm.js` is a mature, standard
answer to exactly that.

That choice has one real architectural consequence worth recording
plainly rather than glossing over: **Tauri's own async runtime is real
`tokio`**, not `rusty_tokio` (`docs/decisions/0001-async-runtime-rusty-tokio.md`'s
project-wide choice) -- baked into the `tauri` crate itself, not
something this crate opted into. Fighting that (trying to drive
`rusty_tokio` tasks from inside a real-tokio-hosted command handler) is a
dead end: two different async runtimes' wakers do not interoperate.
Rather than force the issue, `sessionmgr-desktop`'s own socket client
(`client.rs`, `attach.rs`) is deliberately synchronous `std` I/O on
dedicated blocking threads -- the same "blocking read, forward into a
channel" shape `sessionmgr-tui::app::spawn_input_thread` already uses
for keyboard input, applied here to the daemon socket instead. This
crate is consequently the one place in the workspace with no
`rusty_tokio` dependency at all, and that is a deliberate, load-bearing
decision, not an oversight -- Tauri's runtime requirement is external to
this project, and threading around it is simpler and more honest than
pretending it isn't there.

### Protocol-only boundary, and what that meant for starting a daemon

`sessionmgr-tui` never has to start a daemon itself: it is reached
through `sessionmgr tui`, and the *daemon binary* calls
`client::ensure_daemon` before handing off to it. `sessionmgr-desktop`
has no such wrapper -- it is its own executable, launched directly (by a
user double-clicking it, eventually) -- so it needs the same
auto-start sugar every other client role gets, but cannot get it by
depending on `sessionmgr-daemon` (that dependency would break the same
boundary `sessionmgr-tui`'s own module docs describe: "a UI that cannot
name a process type cannot accidentally spawn one").

The resolution (`daemon.rs`) keeps that boundary intact by shelling out
to the one thing this app is allowed to name: the `sessionmgr` binary's
own `daemon start` subcommand, already idempotent and already blocking
until the daemon answers. `sessionmgr-desktop` locates it as a sibling
of its own executable first (how a packaged install ships both
binaries together -- confirmed in this environment: `cargo build
--workspace` puts both in the same `target/debug/`), falling back to
`PATH`. This is the same "cannot name a process type" boundary
`paths.rs`'s own doc comment describes, just applied to starting the
daemon instead of a session -- the only process command line this crate
ever constructs is "run this one, already-named external program, with
these two words."

`paths.rs` itself duplicates `sessionmgr-daemon::paths::state_root`/
`daemon_socket` rather than depending on that crate, matching the same
reasoning `sessionmgr-tui::client.rs` already established for protocol
framing.

### The frontend: vanilla JS, no bundler, `xterm.js` vendored

No build step (no npm install at build time, no webpack/vite) --
`ui/vendor/xterm/{xterm.js,xterm.css,addon-fit.js}` are vendored
directly (fetched once via `npm pack`, unpacked, copied in), loaded as
plain `<script>` tags exposing `window.Terminal`/`window.FitAddon`
globals. `main.js` is an ES module importing one sibling module,
`grid.js`. This keeps the toolchain surface identical to the rest of
the workspace's own "boring over bespoke" bias -- no second package
manager's lockfile to keep in sync, no bundler config to maintain.

- **`grid.js`**: pure layout math, ported weight-for-weight from
  `sessionmgr-tui::grid::Grid` -- the same "as close to square as
  `count` allows, row-major, shared weights per row/column, summing to
  100" model. The one real difference: `grid.rs`'s own resize is a fixed
  keyboard step per keypress; this grid is resized by dragging a
  divider, so `dragColBoundary`/`dragRowBoundary` take an absolute
  target percentage (computed from the pointer's live position against
  the two immediate neighbors) rather than redistributing evenly across
  every row/column at once -- correct for a drag gesture, which only
  ever moves the one boundary the pointer is on.
- **`main.js`**: session-list polling (2s, matching
  `sessionmgr-tui::app::POLL_INTERVAL`), pane lifecycle (attach on
  appearance, detach and dispose on disappearance -- an `xterm.Terminal`
  per pane, fed raw bytes over a `"session-event"` Tauri event,
  `onData` forwarding real terminal byte sequences back through
  `send_input`), the command palette (`Ctrl/Cmd+K`, the same
  case-insensitive-subsequence fuzzy match `sessionmgr-tui::app::fuzzy_match`
  uses), a prompt overlay for the two actions that need one line of
  text first (new session's repo path, rename, switch-agent's target
  name), and the git diff pane.
- **Command palette actions**: `New session...`, and -- when a session
  is focused -- `Close`, `Rename...`, `Fork`, `Switch agent...`, `Toggle
  git diff`, plus one `Focus: <label>` entry per other open pane. The
  same six actions are also reachable directly from a small button
  toolbar in each pane's own titlebar (`diff`/`rename`/`fork`/`switch`/`×`)
  -- a GUI affordance the TUI has no equivalent surface for, added
  because a mouse-driven app that made every action palette-only would
  be strictly less discoverable than the terminal original it is meant
  to complement, not a replacement for the palette (both paths call the
  same functions).
- **Capabilities**: Tauri v2 gates its own core APIs (not this app's own
  `#[tauri::command]`s, which are unrestricted by default) behind an
  explicit ACL. `src-tauri/capabilities/default.json` grants
  `core:event:default` -- the one core API the frontend actually calls,
  `event.listen`, for the `"session-event"` stream.

## Live verification

Real, not simulated -- run against a real compiled binary, a real
daemon, a real worktree session's real `/bin/bash` PTY, and real
synthetic X11 input, in this environment. This session has no attached
display, so verification used `Xvfb` (a real, if virtual, X server;
`libwebkit2gtk-4.1-dev`/`libgtk-3-dev`/etc. installed for a real Tauri
build) plus `xdotool` for genuine X11 key/mouse events (the same XTest
mechanism a real desktop input device drives) and `import`
(ImageMagick) for screenshots -- the same spirit as this project's
`hub`-driven TUI verification, adapted to a GUI target rather than a
PTY.

1. **Cold boot, no daemon running**: killed every `sessionmgr` process
   first. Launching `sessionmgr-desktop` directly produced a real
   `sessionmgr daemon run --state-root ...` child process (confirmed via
   `ps`) before the window ever rendered -- `daemon.rs`'s auto-start,
   proven against a genuinely daemon-less start, not just against an
   already-running one.
2. **Session list / sidebar**: `sessionmgr new` from a second, separate
   shell created a real worktree session; the app's own 2s poll picked
   it up with no interaction, both in the sidebar and as a new live pane
   -- confirmed by screenshot, not inferred from logs.
3. **Real terminal rendering**: the pane showed the session's actual
   `/bin/bash` prompt and a real "reattached to a session that survived
   a manager restart" marker (this session had already produced output
   before this particular attach connection), rendered through
   `xterm.js`'s own VT100 interpreter from raw bytes streamed over
   `session-event` -- the arguably even more standard implementation of
   ADR-0002's "interpret, don't print" requirement than `sessionmgr-tui`'s
   own `vt100` crate, since `xterm.js` is a reference terminal
   implementation in its own right.
4. **Real keyboard input, round-tripped**: `xdotool type`/`key` sent
   genuine X11 key events into the focused pane's terminal. `echo
   DESKTOP_INPUT_WORKS` and its echoed output both appeared in the
   rendered pane -- direct proof bytes travel `xterm.onData` ->
   `send_input` -> the daemon -> the real PTY -> back out through
   `session-event`, not just that keystrokes are accepted somewhere.
5. **Command palette**: `Ctrl+K` (via `xdotool key`) opened the palette
   with exactly the expected action list for a focused session (`New
   session...`, `Close`, `Rename...`, `Fork`, `Switch agent...`,
   `Toggle git diff`, plus `Focus:` entries for other panes).
6. **Multi-pane grid, live**: triggering `New session...` through the
   palette's own prompt (typed a real repo path, pressed Enter) created
   a **second** real session through the app's own `session_new`
   command -- the grid correctly reflowed to two columns, and the first
   pane's terminal visibly re-wrapped its existing text at the new,
   narrower width, direct evidence a real `send_resize` fired on the
   layout change (not just a CSS resize) and the real PTY's own line
   wrapping responded to it.
7. **Git diff pane**: created a real untracked file inside the session's
   worktree from outside the app, then clicked the pane's own `diff`
   button -- the diff pane opened showing a real `git status`
   (`?? testfile.txt`) and, selecting it, a real (empty, correctly --
   `git diff` shows nothing for an untracked file) diff view.
8. **Rename**: the pane's own `rename` button opened the prompt
   pre-filled correctly, and submitting a new name updated both the
   sidebar and, after a fix (below), the pane's own titlebar --
   confirmed to persist correctly even across the daemon restart from
   step 1 by the time it was tested.
9. **Error surfacing**: clicking `fork` on a plain (no-agent) session
   produced the daemon's real error message, verbatim, in the status
   bar -- `fork failed: session <id> has no agent CLI conversation to
   fork` -- proof the whole error path (`Response::Error` ->
   `client::expect` -> the JS `catch` -> `setStatus`) works end to end,
   not just the happy path.

### Five real bugs found and fixed by this pass

None of these were caught by `cargo build`/`clippy`/`fmt` -- all five
only surfaced by actually running the app and looking at it, which is
the entire reason this section exists rather than a bare "it works":

1. **Empty-state never appeared on first boot.** `layout()` (the only
   thing that toggles the `no sessions...` message) was only called when
   a pane was added or removed; with zero sessions from a cold start,
   neither ever happened, so it never ran at all. Fixed with an
   `everLaidOut` flag forcing one initial layout pass regardless.
2. **`event.listen` denied by Tauri's own ACL.** Tauri v2 gates its core
   APIs by capability even though this app's own `#[tauri::command]`s
   are unrestricted; nothing declared `core:event:listen` was allowed,
   so the frontend's session-event subscription failed on every launch
   with an unhandled promise rejection (caught live by a small
   diagnostic script added specifically to surface this class of
   failure visibly in a screenshot). Fixed by adding
   `src-tauri/capabilities/default.json`.
3. **`Ctrl/Cmd+K`/`Ctrl/Cmd+N` silently swallowed whenever a terminal
   pane had focus.** `xterm.js`'s own keydown handler calls
   `stopPropagation()` on most key combinations so a terminal's own
   Ctrl bindings don't leak into page-level shortcuts -- which also
   meant a bubble-phase `document` listener never saw them. Fixed by
   registering the global keybinding listener in the **capture** phase,
   which runs before the event ever reaches the focused pane's own
   textarea.
4. **The status bar rendered entirely off-screen.** A structural HTML
   bug, not a CSS one: `#status-bar` was a sibling of `#app` (which
   itself fills `100vh`) rather than nested inside `#workspace`'s own
   flex column, so it was pushed below the full-height `#app` in normal
   document flow -- invisible on any window, at any size. Fixed by
   moving it inside `#workspace`, after `#grid`.
5. **A pane's own titlebar never updated after creation.** The session
   list refresh loop called `updatePaneHeader` only in the "this pane is
   new" branch; an already-open pane's name/status labels were set once,
   at creation, and never again -- so renaming a session updated the
   sidebar (which re-reads `sessions` fresh every render) but not the
   pane's own title. Fixed by calling `updatePaneHeader` for every
   already-open pane on every poll too, matching what
   `sessionmgr-tui::app::refresh_sessions` already does for its own
   panes.

## Tests

5 new unit tests: `paths.rs` (the daemon-socket path shape, and the
same "empty env var is rejected, not silently cwd" rule
`sessionmgr-daemon::paths` itself tests, mirrored exactly including its
own save-and-restore-around-the-mutation care since `std::env::set_var`
is process-global and shared with every other test in the binary),
`commands.rs` (`parse_agent_name`'s three-name/error-shape duplication,
`parse_id`'s malformed-id rejection). No Rust-side tests for `grid.js`
or the rest of `main.js` -- there is no JS test runner in this
workspace's toolchain, and adding one for a single small pure module
would be a bigger toolchain addition than the module itself; the live
verification above is this phase's acceptance evidence for the
frontend, the same role live verification already plays for
`sessionmgr-tui`'s own command palette (`docs/phase-4b-report.md`).

`cargo build --workspace`/`clippy --workspace --all-targets -- -D
warnings`/`fmt --all --check` all clean. `cargo test --workspace`
green aside from the same pre-existing, undiffed
`agent_needs_input_claude` interactive-PTY flake every phase report
since 5 has documented (confirmed unrelated: reproduces identically on
unmodified `main`, and this phase's diff never touches
`sessionmgr-daemon` at all).

## What is not done

**Drag-resize, not live-verified.** The grid's column/row divider drag
handlers (`wireDrag` in `main.js`) are implemented and code-reviewed
against the same weight math `grid.rs` already has unit tests for, but
this pass's `xdotool`-driven verification did not include a synthetic
pointer-drag gesture (`xdotool`'s own drag primitives are considerably
less reliable under a virtual, WM-less X server than click/key
injection was) -- unlike everything else in the "Live verification"
section, this one piece is asserted from code review, not observed
behavior. Worth a real pass on an actual desktop machine with a mouse
before leaning on it.

**Packaging/distribution.** This phase built and live-verified a
working `cargo build`-produced binary; it did not run `tauri build`'s
full bundler (installers, code signing, auto-update) or produce a
distributable artifact. `tauri.conf.json`'s `bundle.icon` currently
points at three placeholder PNGs generated programmatically in this
pass (a solid dark-teal square) -- real branding, if wanted, is a
separate, later decision.

**Feature parity beyond the palette's six actions.** `sessionmgr new`'s
full flag surface (kind other than `Worktree`, hooks, dependent
sessions/`--parent`) has no palette or UI equivalent yet, matching
`sessionmgr-tui`'s own "fast shortcut for the common case" scope for its
identical `New session...` action -- not a regression from the TUI, but
not an expansion past it either.

**Windows.** This environment is Linux; Tauri's Windows target uses
WebView2 (ships with Windows 10/11, unlike the `webkit2gtk` this pass
installed) rather than anything verified here, and this project is
otherwise Windows-native per every prior phase report's own closing
checks. The architecture and dependency choices here are
platform-agnostic (Tauri itself is the cross-platform layer), but a
real Windows build/run has not been attempted in this pass -- flagged
honestly rather than assumed to "just work" the way this project's own
conventions require for anything not actually run.
