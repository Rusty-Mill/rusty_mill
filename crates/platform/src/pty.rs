//! PTY surface (RFC v2 R5+, decision D13, convergence roadmap Phase 7) — a
//! Process×Terminal capability, built without a confirmed live consumer on
//! the owner's explicit call (the same posture `security::CredentialStore`
//! and `security::Sandbox`'s confinement half were built under). See
//! `docs/design-discussion-pty.md` for the full donor-shape reconciliation
//! this trait's shape is drawn from.
//!
//! **One atomic [`Pty::spawn`], not a separate open/attach pair.** Windows's
//! ConPTY structurally cannot attach an already-created pseudo console to
//! an already-running, separately-spawned process — the pseudo console is
//! only ever wired to a child at `CreateProcessW` time. Offering a Unix-only
//! "open a pty, attach a process to it later" pair would leave the attach
//! step permanently `Unsupported` on Windows, so this trait doesn't offer
//! it: opening the pty and spawning the child are one call on every
//! backend. [`Command`]'s existing `argv`/`cwd`/`env` fields apply
//! unchanged; `stdin`/`stdout`/`stderr` are moot (the pty slave is all
//! three, so backends ignore whatever those fields hold) and
//! [`GroupSpec::JoinGroup`](crate::process::GroupSpec::JoinGroup) is
//! rejected outright — see [`Pty::spawn`]'s own doc for why.
//!
//! **`Ok(0)` at EOF**, matching [`crate::fs::File`]/
//! [`crate::term::Terminal::read_chunk`]'s existing convention — not a raw
//! errno, not an OS-specific sentinel. Unix's `EIO`-on-slave-close and
//! Windows's broken-pipe-on-child-exit both collapse to this one signal.
//!
//! **Resize** reuses [`crate::term::WinSize`] rather than inventing a
//! parallel size type.
//!
//! Deliberately out of scope: macOS (no donor evidence — D13 only surveys
//! shh/rusty_term, both Linux+Windows), job-control terminal handoff
//! (already job control in D9 — a consumer composes the two),
//! and resize *notification* (an already-deferred `term` facet, D9).

use crate::error::Result;
use crate::process::{Child, Command};
use crate::term::WinSize;

/// The master side of a spawned pty pair. Object-safe.
pub trait PtyMaster {
    /// Read output the pty-hosted child (or the pty layer itself) wrote to
    /// the slave. Blocking. `Ok(0)` is EOF — the slave side closed because
    /// the child exited — matching [`crate::fs::File::read`]'s convention,
    /// not a raw `EIO`/broken-pipe error.
    fn read(&self, buf: &mut [u8]) -> Result<usize>;

    /// Write input for the pty-hosted child to read from the slave.
    /// Blocking.
    fn write(&self, buf: &[u8]) -> Result<usize>;

    /// Update the pty's window size, visible to the child the next time it
    /// queries its terminal size (`TIOCGWINSZ`/console-buffer-info
    /// equivalent) — the same size a real terminal resize would report.
    fn resize(&self, size: WinSize) -> Result<()>;
}

/// A backend capable of hosting a process on a fresh pty. Object-safe.
pub trait Pty {
    /// Open a fresh pty pair and spawn `cmd` attached to its slave side —
    /// one atomic operation (see this module's doc comment for why it
    /// isn't split into separate open/attach steps). `size` is the pty's
    /// initial window size.
    ///
    /// `cmd.stdin`/`cmd.stdout`/`cmd.stderr` are ignored — the pty slave
    /// is all three, unconditionally, exactly like a real interactive
    /// terminal session. `cmd.group` accepts
    /// [`GroupSpec::Inherit`](crate::process::GroupSpec::Inherit) and
    /// [`GroupSpec::NewGroup`](crate::process::GroupSpec::NewGroup)
    /// (both are effectively the same request here — see below) but
    /// [`GroupSpec::JoinGroup`](crate::process::GroupSpec::JoinGroup)
    /// fails with `InvalidInput`: a pty-hosted child unconditionally
    /// becomes a new *session* leader (the mechanism that makes it own
    /// the pty as its controlling terminal in the first place), which
    /// makes it a fresh process-group leader too, by definition — there
    /// is no way to host a child on a fresh pty and also place it into an
    /// existing, different process group.
    fn spawn(&self, cmd: &Command, size: WinSize) -> Result<(Box<dyn PtyMaster>, Box<dyn Child>)>;
}
