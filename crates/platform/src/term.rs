//! Terminal surface — the D9 cluster (extraction map, second wave;
//! architecture.md Layer 2). Five donors independently hand-rolled this
//! personality — rusty_libc's termios/tty, rusty_win32's console modes,
//! rush's isatty plumbing, rusty_lines' `term_sys` facade, and shh's
//! `tty/{unix,windows}` — the strongest consumer-gate evidence any
//! surface in this project has had (§3; named consumers: rusty_naner,
//! rush interactive, rusty_lines' host, shh, and `rterm` here as the
//! reference consumer).
//!
//! **Design oracle: `rusty_term`.** Its `backend::{Backend,
//! BackendHandle}` pair is a full terminal emulator's OS slice, already
//! factored as a portable trait over two per-OS unsafe files and
//! CI-proven on both OSes — richer evidence than any single donor above.
//! This trait is built fresh from its *semantics*, not linked to it: its
//! backend pulls in tokio and an edition-2024 MSRV, which would invert
//! the layer stack (Layer 2 depending on a Layer 3 tool's async
//! runtime) and blow past this workspace's 1.75 floor. `rusty_term`
//! converges later by swapping its backend internals for this trait.
//! Two lessons it teaches that the earlier three-donor sketch missed:
//! raw mode on Windows touches **two streams** (stdin input modes *and*
//! stdout VT-processing — see [`Terminal::enter_raw`]'s Windows arm),
//! and the save/restore lifecycle (not a bare toggle) is the actual
//! contract — both are reflected in this trait's shape.
//!
//! Slice 1 is the portable core the donors agree on: per-stream tty
//! detection, window size, and raw mode. Slice 2 (convergence roadmap
//! Phase 2, forced by rusty_lines' `term_sys` facade) adds the input
//! side a line editor needs: a live [`Terminal::is_raw`] probe,
//! [`Terminal::poll_readable`]/[`Terminal::read_chunk`] for batched,
//! non-per-byte input, and [`Terminal::set_echo`] for a password-prompt
//! style echo toggle independent of full raw mode.
//!
//! **Two facets rusty_lines has that deliberately get no new surface**
//! here, because they need none: bracketed paste is protocol bytes over
//! the stream the consumer already reads via [`read_chunk`](
//! Terminal::read_chunk) — no OS call is involved, so it stays
//! expressible by the consumer rather than owned here (the same
//! discipline this project applies to shell policy). Cooked↔raw
//! suspend/resume (the `$EDITOR`-handoff shape) is exactly
//! [`enter_raw`](Terminal::enter_raw)/[`leave_raw`](Terminal::leave_raw)
//! called a second time — their save/restore contract already covers
//! it; naming a separate pair would duplicate, not extend, the surface.
//!
//! Several other real, larger facets are deliberately deferred to later
//! slices, each entering only when a consumer forces it (§3): Unix
//! job-control handoff (`tcsetpgrp`, SIGTSTP/SIGCONT — no portable
//! twin); PTY hosting (D13 — a distinct Process×Terminal surface, not
//! folded into raw-mode/winsize); resize *notification* (a genuine
//! divergence `rusty_term` proves: a SIGWINCH stream on Unix vs no
//! Windows equivalent, so Windows consumers poll — `behavior/term.md`
//! records it); and byte→key decode (keymap policy, stays with the
//! consumer).
//!
//! **Console *acquisition*** for GUI-subsystem processes (attach/alloc/
//! redirect — from `rusty_naner`) is a separate facet from raw-mode on
//! an already-inherited console, built speculatively on the owner's
//! explicit call (the same posture PTY hosting was built under, D13):
//! see [`ConsoleState`]/[`ConsoleAcquisition`], below.
//!
//! Raw mode is **stateful and process-visible** (termios flags /
//! console modes): [`Terminal::enter_raw`] saves the previous state and
//! [`Terminal::leave_raw`] restores exactly it. Consumers own the
//! enter/leave pairing (RAII belongs to the consumer's guard, not this
//! object-safe trait — same division as the donors').

use std::time::Duration;

use crate::error::Result;

/// The three standard streams, named — raw fd/handle numbers never cross
/// this boundary (RFC v2 §5).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum TermStream {
    /// Standard input.
    Stdin,
    /// Standard output.
    Stdout,
    /// Standard error.
    Stderr,
}

/// A terminal's visible size, in character cells.
///
/// On Windows this is the *viewport* (the window), not the scrollback
/// buffer — the donor (rusty_win32) learned that distinction the hard
/// way; `behavior/term.md` pins it.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct WinSize {
    /// Rows (height).
    pub rows: u16,
    /// Columns (width).
    pub cols: u16,
}

/// The portable terminal surface. Object-safe.
pub trait Terminal {
    /// Whether `stream` is attached to a terminal (isatty /
    /// console-mode probe). Never errors: a stream that cannot be
    /// probed is not a tty.
    fn is_tty(&self, stream: TermStream) -> bool;

    /// The controlling terminal's size. Errors when no stream is a
    /// terminal (CI pipes, redirected batch runs) — callers that want a
    /// fallback width own that policy.
    fn window_size(&self) -> Result<WinSize>;

    /// Switch the terminal to raw mode (no echo, no line buffering, no
    /// signal-generating keys), saving the previous state. Errors when
    /// stdin is not a terminal. Idempotent: a second call is a no-op.
    fn enter_raw(&mut self) -> Result<()>;

    /// Restore the state saved by [`enter_raw`](Terminal::enter_raw).
    /// Idempotent: without a prior `enter_raw` it is a no-op.
    fn leave_raw(&mut self) -> Result<()>;

    /// A **live** probe of whether stdin's current attributes look raw
    /// (line buffering and echo both off) — re-queried from the OS each
    /// call, not cached from the last `enter_raw`. This is what lets a
    /// consumer notice drift: something outside this handle (a `stty`
    /// invocation, a suspended-then-foregrounded shell) can change the
    /// terminal's mode without going through `enter_raw`/`leave_raw`
    /// at all, and rusty_lines' self-healing idle tick exists precisely
    /// to catch and correct that.
    fn is_raw(&self) -> bool;

    /// Block on stdin for up to `timeout` (`None` = forever), returning
    /// whether it became readable. Works whether or not stdin is a
    /// terminal (a redirected pipe is just as pollable) — this is a
    /// readiness check, not a tty operation.
    fn poll_readable(&self, timeout: Option<Duration>) -> Result<bool>;

    /// Read up to `buf.len()` bytes from stdin in one call — batched,
    /// not per-byte (the VMIN=1/VTIME=0 shape in raw mode: blocks for at
    /// least one byte, then returns whatever else is already buffered
    /// without an extra round trip). `Ok(0)` is EOF, matching
    /// `std::io::Read`'s convention.
    fn read_chunk(&self, buf: &mut [u8]) -> Result<usize>;

    /// Toggle local echo on stdin, independent of full raw mode (the
    /// password-prompt shape: line editing/canonical mode stays on,
    /// only the terminal's echo of typed characters is suppressed).
    /// Returns the *previous* echo state, so a caller restores exactly
    /// by calling `set_echo(previous)` — the same save/restore
    /// discipline as raw mode, scaled down to one bool because that is
    /// all this operation touches.
    fn set_echo(&mut self, on: bool) -> Result<bool>;
}

/// Job-control terminal handoff (`tcsetpgrp`) — D1/D9, Unix-only with no
/// Windows twin (this module's doc comment; divergence-registry entry
/// 008 covers the process-side half of the same gap). Deliberately a
/// separate trait from [`Terminal`], not a method on it: every backend
/// including Windows implements `Terminal`, but there is no
/// `tcsetpgrp`-equivalent for a Windows backend to implement here — a
/// job-control consumer (a shell) opts into this trait specifically,
/// rather than every `Terminal` implementor being forced to answer for a
/// capability that does not exist on one of them.
pub trait JobControlTerminal {
    /// Hand the controlling terminal's foreground process group to
    /// `pgid` (`tcsetpgrp(STDIN_FILENO, pgid)`) — called both to give a
    /// foreground job the terminal and, once it stops or exits, to
    /// reclaim it for the shell's own group. Sound only once `SIGTTOU`
    /// is ignored in the calling process (D1's precondition): a
    /// background process that calls `tcsetpgrp` is stopped by
    /// `SIGTTOU` by default otherwise. Implementations encode this
    /// precondition themselves — ignoring `SIGTTOU` as part of every
    /// call — rather than leaving it to the caller to remember.
    fn give_terminal(&self, pgid: u32) -> Result<()>;
}

/// Which of the console-acquisition personalities the calling process is
/// currently in — the D9 facet from the `rusty_naner` donor
/// (`naner-core/console.rs`), landed as its own opt-in trait
/// ([`ConsoleAcquisition`], below) rather than folded into [`Terminal`].
///
/// This state (and the trait that reports/changes it) exists only
/// because Windows draws a distinction Unix does not: a Windows process
/// can be built for the GUI subsystem (`/SUBSYSTEM:WINDOWS`), which gets
/// **no console at all** unless it explicitly acquires one — a state
/// with no Unix analog (every Unix process either inherits a controlling
/// terminal at exec time or doesn't; there is no separate "go acquire
/// one now" step to model). Mirrors [`JobControlTerminal`]'s own
/// asymmetry in the other direction: that trait is Unix-only with no
/// Windows implementor, this one is (in practice) Windows-only with no
/// Unix implementor.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ConsoleState {
    /// No console is attached to this process at all — the GUI-subsystem
    /// default, or the state after [`ConsoleAcquisition::free_console`].
    None,
    /// A console was inherited at process-creation time — the ordinary
    /// case for a console-subsystem process launched from a shell; every
    /// other [`Terminal`] capability in this module already operates
    /// against it with no acquisition step required.
    Inherited,
    /// A brand-new console was acquired via
    /// [`ConsoleAcquisition::alloc_console`].
    Allocated,
    /// Another process's console (typically the parent's) was acquired
    /// via [`ConsoleAcquisition::attach_console`].
    Attached,
}

/// Console *acquisition* for GUI-subsystem processes — `AttachConsole`/
/// `AllocConsole`/`FreeConsole`, plus the `CONOUT$`/`CONIN$` reopen and
/// VT re-enable a caller needs afterward to actually use the newly
/// acquired console through this same module's [`Terminal`] surface.
/// D9's `rusty_naner` facet, deliberately a separate opt-in trait from
/// `Terminal` for the same reason [`JobControlTerminal`] is: no portable
/// twin exists (Unix has no GUI-subsystem/console-subsystem split to
/// acquire a console *into*), so a consumer that needs this opts in
/// explicitly rather than every `Terminal` implementor answering for a
/// capability only one of them has.
///
/// **Ordering contract**: after a successful [`alloc_console`](
/// ConsoleAcquisition::alloc_console) or [`attach_console`](
/// ConsoleAcquisition::attach_console), the process's std handles are
/// already repointed at the new console (`CONOUT$`/`CONIN$`/`CONERR$`
/// reopened and installed via `SetStdHandle`) and VT output processing
/// is re-enabled on the new stdout — every other [`Terminal`] method
/// this trait's implementor also implements observes the new console
/// immediately, with no separate fixup step for the caller to remember.
pub trait ConsoleAcquisition {
    /// The console personality this process is currently in — a cheap,
    /// non-probing accessor reporting what this handle's own
    /// `alloc_console`/`attach_console`/`free_console` calls last left
    /// it as (seeded from a real inherited-console probe at
    /// construction), not a fresh OS query on every call.
    fn console_state(&self) -> ConsoleState;

    /// Allocate a brand-new console for this process (`AllocConsole`)
    /// and repoint its std handles onto it. Fails with
    /// `ErrorKind::PermissionDenied` — Windows' own `AllocConsole`
    /// failure code for the case, surfaced as-is rather than remapped —
    /// if this process already has a console; free it first with
    /// [`free_console`](Self::free_console).
    fn alloc_console(&mut self) -> Result<()>;

    /// Attach to another process's console (`AttachConsole`) and
    /// repoint this process's std handles onto it. `pid = None` attaches
    /// to whatever console (if any) launched this process — the usual
    /// case for a GUI-subsystem process reusing its parent's console
    /// rather than allocating its own. Fails if this process already has
    /// a console, or if the target has none.
    fn attach_console(&mut self, pid: Option<u32>) -> Result<()>;

    /// Detach this process from its current console (`FreeConsole`) —
    /// e.g. before [`alloc_console`](Self::alloc_console) a second time
    /// to create a fresh one. A no-op, not an error, when no console is
    /// currently attached.
    fn free_console(&mut self) -> Result<()>;
}
