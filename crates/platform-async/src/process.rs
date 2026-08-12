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
//!
//! [`AsyncSpawner::wait_any`] (parity-gap #4) extends this to *several*
//! children at once — wait for whichever finishes first, without
//! dedicating a `Future` registration per child at the call site. It is
//! built entirely on [`AsyncChild::ready`], so a backend that already
//! implements `ready` gets a genuinely multiplexed `wait_any` for free
//! (see the default implementation below) rather than needing its own
//! native override.

use std::ffi::{OsStr, OsString};
use std::future::Future;
use std::pin::Pin;
use std::task::{Context, Poll};
use std::time::{Duration, Instant};

use platform::error::{ErrorKind, OsCode, PlatformError, Result};
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

    /// Resolves once this child has terminated — *borrowing*, not
    /// consuming. This is the building block [`wait_any`] needs to
    /// multiplex several children without permanently taking ownership
    /// of any of them before knowing which one actually finished first.
    ///
    /// A caller that only cares about one child should prefer
    /// [`AsyncChild::wait`] (which also retrieves the decoded
    /// `ExitStatus` in a single step); `ready` exists for the multi-child
    /// case, where retrieval happens afterward via `try_wait`/`wait` on
    /// whichever child answered first.
    fn ready(&self) -> BoxFuture<'_, Result<()>>;
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

    /// Async counterpart to [`platform::process::Spawner::wait_any`] /
    /// the free fn [`platform::process::wait_any`]: wait for *some*
    /// child in `children` to terminate, for up to `timeout` (`None` =
    /// forever). Resolves to `Some(index)` of a terminated child —
    /// retrieve its status via that child's [`AsyncChild::try_wait`]/
    /// [`AsyncChild::wait`] — or `None` on timeout. An empty slice is
    /// `InvalidInput`, mirroring the sync side's own refusal.
    ///
    /// The default implementation delegates to the free fn [`wait_any`],
    /// which is built entirely on [`AsyncChild::ready`] — a backend does
    /// not need its own override to get genuine multiplexing, only a
    /// correct `ready`. This mirrors the sync `Spawner::wait_any`'s own
    /// "portable default, backend may override" shape (RFC v2 §5.6),
    /// except here the "default" is already the real implementation for
    /// any backend whose `ready` registers with a shared reactor,
    /// `platform-async-linux` included.
    fn wait_any<'a>(
        &'a self,
        children: &'a mut [Box<dyn AsyncChild>],
        timeout: Option<Duration>,
    ) -> BoxFuture<'a, Result<Option<usize>>> {
        wait_any(children, timeout)
    }
}

/// Portable, backend-agnostic `wait_any` (see [`AsyncSpawner::wait_any`]'s
/// doc comment) — races every child's [`AsyncChild::ready`] future
/// alongside an optional [`Timeout`], hand-rolled rather than pulled from
/// an external `futures`-style crate (this workspace's minimal-dependency
/// discipline — see `platform-async-linux`'s own `EpollReactor` for the
/// same reasoning applied to the reactor itself).
pub fn wait_any<'a>(
    children: &'a mut [Box<dyn AsyncChild>],
    timeout: Option<Duration>,
) -> BoxFuture<'a, Result<Option<usize>>> {
    if children.is_empty() {
        let err = PlatformError::new(ErrorKind::InvalidInput, OsCode::None, "wait_any");
        return Box::pin(async move { Err(err) });
    }
    let readies: Vec<BoxFuture<'a, Result<()>>> = children.iter().map(|c| c.ready()).collect();
    Box::pin(WaitAny {
        readies,
        timeout: Timeout::new(timeout),
    })
}

/// Races a set of `ready()` futures against an optional [`Timeout`].
/// Re-polls every not-yet-ready child on each call — cheap, since a
/// `ready()` future that has already registered with its reactor just
/// checks its own `Poll::Pending`/`Poll::Ready` state rather than
/// re-registering.
struct WaitAny<'a> {
    readies: Vec<BoxFuture<'a, Result<()>>>,
    timeout: Timeout,
}

impl<'a> Future for WaitAny<'a> {
    type Output = Result<Option<usize>>;

    fn poll(self: Pin<&mut Self>, cx: &mut Context<'_>) -> Poll<Self::Output> {
        // No field needs structural pinning: `Vec<Pin<Box<dyn Future>>>`
        // and `Timeout` are both `Unpin`, so a plain `&mut Self` is sound
        // (`Pin<Box<dyn Future>>` is already pinned in its own right —
        // polling it through `&mut` doesn't move the pointee).
        let this = self.get_mut();
        for (i, ready) in this.readies.iter_mut().enumerate() {
            match ready.as_mut().poll(cx) {
                Poll::Ready(Ok(())) => return Poll::Ready(Ok(Some(i))),
                Poll::Ready(Err(e)) => return Poll::Ready(Err(e)),
                Poll::Pending => {}
            }
        }
        if Pin::new(&mut this.timeout).poll(cx).is_ready() {
            return Poll::Ready(Ok(None));
        }
        Poll::Pending
    }
}

/// A one-shot deadline future. `None` never fires (the caller relies on
/// the children themselves to end the wait). `Some(duration)` spawns
/// exactly one disclosed helper thread on first poll — the same
/// disclosed-thread-cost pattern `platform-async-linux`'s `EpollReactor`
/// already uses (`RM-DEV-ASYNC-0002`: blocking adapters disclose their
/// threads) — rather than pulling in a timer-wheel dependency for what
/// is, in this workspace, a single-use deadline.
struct Timeout {
    deadline: Option<Instant>,
    thread_armed: bool,
}

impl Timeout {
    fn new(duration: Option<Duration>) -> Self {
        Self {
            deadline: duration.map(|d| Instant::now() + d),
            thread_armed: false,
        }
    }
}

impl Future for Timeout {
    type Output = ();

    fn poll(self: Pin<&mut Self>, cx: &mut Context<'_>) -> Poll<()> {
        let this = self.get_mut();
        let Some(deadline) = this.deadline else {
            return Poll::Pending;
        };
        let now = Instant::now();
        if now >= deadline {
            return Poll::Ready(());
        }
        if !this.thread_armed {
            this.thread_armed = true;
            let waker = cx.waker().clone();
            let remaining = deadline.saturating_duration_since(now);
            std::thread::spawn(move || {
                std::thread::sleep(remaining);
                waker.wake();
            });
        }
        Poll::Pending
    }
}
