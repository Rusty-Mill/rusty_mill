//! Async process domain — mirrors `platform::process` (rustils RFC v2
//! §5.4) for the one domain in rustils that is already *Active* with a
//! real consumer (`coreutils`).
//!
//! What is here: an async counterpart to `Child::wait` only.
//!
//! Spawning a process is a single fast syscall, not something async
//! multiplexing helps with (`RM-DEV-ASYNC-0001`: "Async is used only
//! where the contract can exploit genuine I/O concurrency, waiting,
//! multiplexing, or cancellation. CPU-bound and trivially sequential
//! work remains synchronous"). [`AsyncSpawner::spawn`] therefore calls
//! straight through to a real, already-sound `platform::process::Spawner`
//! synchronously — this crate does not re-implement fork/exec, so it
//! does not reproduce the soundness risk rustils' own RFC v2 §6 spent
//! real effort closing (dangling `CString`s, post-fork allocation,
//! injection-by-construction quoting, double-wait).
//!
//! Waiting for termination *is* the genuine multiplexing point (many
//! children, one thread, no busy-poll), so that becomes a [`Future`].
//! `platform-async-linux` is what actually drives that future against a
//! real reactor; the mock backend (`platform-async-mock`) resolves it
//! immediately, since a scripted child has nothing to wait for.

use std::ffi::{OsStr, OsString};
use std::future::Future;
use std::pin::Pin;

use platform::error::Result;
use platform::process::{Command, ExitStatus, GroupHandle, Signal};

/// Boxed future — the hand-written equivalent of what an
/// `async-trait`-style macro would generate, chosen over that
/// dependency per rustils' own minimal-dependency discipline, now that
/// this trait needs to stay object-safe (`Box<dyn AsyncChild>`,
/// mirroring the sync `Box<dyn` [`platform::process::Child`]`>` it sits
/// beside).
///
/// Deliberately not bounded `+ Send`: `platform::process::Child` itself
/// carries no `Send` bound (unlike this crate's `net.rs`-style
/// counterparts, which do — see that module's own doc comment), so
/// requiring it here would claim a property the sync type this wraps
/// does not actually guarantee. A backend whose concrete `Child` type
/// happens to be `Send` can still add that bound at its own call site
/// (`Box<dyn AsyncChild + Send>`).
pub type BoxFuture<'a, T> = Pin<Box<dyn Future<Output = T> + 'a>>;

/// A spawned child with an async wait path. Object-safe; mirrors
/// [`platform::process::Child`] field-for-field except `wait`.
pub trait AsyncChild {
    /// Wait for termination without blocking a thread — the actual
    /// async value-add for this domain (see module docs). Consumes
    /// `self`, the same double-wait-is-unrepresentable contract as the
    /// sync `Child::wait`.
    fn wait(self: Box<Self>) -> BoxFuture<'static, Result<ExitStatus>>;

    /// OS process identifier, for display/diagnostics.
    fn id(&self) -> u32;

    /// Same contract as [`platform::process::Child::kill_tree`].
    fn kill_tree(&self, sig: Signal) -> Result<()>;

    /// Same contract as [`platform::process::Child::kill_single`].
    fn kill_single(&self, sig: Signal) -> Result<()>;

    /// Non-blocking poll — identical contract to the sync
    /// `Child::try_wait`. Already non-blocking, so it does not need an
    /// async counterpart (`RM-DEV-ASYNC-0001` again: work that does not
    /// wait stays sync).
    fn try_wait(&mut self) -> Result<Option<ExitStatus>>;
}

/// A backend capable of spawning processes with an async wait path.
/// Object-safe.
pub trait AsyncSpawner: Send + Sync {
    /// Spawn synchronously — see the module doc comment for why this is
    /// not itself async. This trait's job is to route the call and wrap
    /// the result, not to re-implement spawn internals: soundness for
    /// spawn itself stays owned by whichever sync `Spawner` a backend
    /// wraps (`RM-DEV-ASYNC-0003` forbids a sync API silently entering
    /// an async runtime; the same discipline in reverse argues against
    /// this crate duplicating sync's soundness-critical fork/exec path).
    fn spawn(&self, cmd: &Command) -> Result<Box<dyn AsyncChild>>;

    /// Same contract as [`platform::process::Spawner::resolve`].
    fn resolve(&self, program: &OsStr) -> Result<OsString>;

    /// Same contract as [`platform::process::Spawner::adopt`].
    fn adopt(&self, pid: u32) -> Result<Box<dyn GroupHandle>>;

    /// Same contract as [`platform::process::Spawner::is_alive`].
    fn is_alive(&self, pid: u32) -> Result<bool>;
}
