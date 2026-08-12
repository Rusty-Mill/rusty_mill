//! # platform-async-linux — the real Linux backend for `platform-async`
//!
//! Reuses rustils' own `platform-linux::LinuxSpawner` for spawning
//! (synchronously — see `platform_async::process`'s module doc comment
//! for why) and adds a genuinely async, non-blocking wait path: each
//! [`AsyncLinuxChild::wait`] opens a `pidfd` for the child and awaits
//! its readiness through an explicit, per-spawner [`sys::reactor::EpollReactor`]
//! (`RM-ASYNC-RUNTIME-0001`: no hidden global runtime) instead of
//! rustils' own blocking `pidfd + poll(2)` tick loop.
//!
//! No `unsafe` at this level — confined to `sys/`, one documented
//! invariant per block, same discipline as `platform-linux` itself.

#![cfg(target_os = "linux")]
#![deny(unsafe_code)] // opted back in, narrowly, inside sys/ modules only

pub mod sys;

use std::ffi::{OsStr, OsString};
use std::future::Future;
use std::os::fd::{AsRawFd, OwnedFd};
use std::pin::Pin;
use std::sync::Arc;
use std::task::{Context, Poll};

use platform::error::Result;
use platform::process::{Child, Command, ExitStatus, GroupHandle, Signal, Spawner};
use platform_async::process::{AsyncChild, AsyncSpawner, BoxFuture};
use platform_linux::LinuxSpawner;

use crate::sys::reactor::EpollReactor;

/// The Linux async process backend. Owns its own [`EpollReactor`] and
/// its background thread explicitly — constructing one is a real,
/// fallible, disclosed operation (spawning a thread can fail), not a
/// hidden side effect of first use.
pub struct AsyncLinuxSpawner {
    inner: LinuxSpawner,
    reactor: Arc<EpollReactor>,
}

impl AsyncLinuxSpawner {
    pub fn new() -> Result<Self> {
        Ok(Self {
            inner: LinuxSpawner,
            reactor: EpollReactor::new()?,
        })
    }
}

impl AsyncSpawner for AsyncLinuxSpawner {
    fn spawn(&self, cmd: &Command) -> Result<Box<dyn AsyncChild>> {
        let child = self.inner.spawn(cmd)?;
        Ok(Box::new(AsyncLinuxChild {
            inner: child,
            reactor: Arc::clone(&self.reactor),
        }))
    }

    fn resolve(&self, program: &OsStr) -> Result<OsString> {
        self.inner.resolve(program)
    }

    fn adopt(&self, pid: u32) -> Result<Box<dyn GroupHandle>> {
        self.inner.adopt(pid)
    }

    fn is_alive(&self, pid: u32) -> Result<bool> {
        self.inner.is_alive(pid)
    }
}

struct AsyncLinuxChild {
    inner: Box<dyn Child>,
    reactor: Arc<EpollReactor>,
}

impl AsyncChild for AsyncLinuxChild {
    fn wait(self: Box<Self>) -> BoxFuture<'static, Result<ExitStatus>> {
        // Move the boxed value out by value (`Box` is the one smart
        // pointer the compiler lets you do this with) so the async
        // block below owns plain fields rather than a `Box<Self>` — it
        // needs to partially move `inner` out at the end while only
        // borrowing `reactor` earlier.
        let this = *self;
        Box::pin(async move {
            let pid = this.inner.id();
            let pidfd = sys::pidfd::open(pid as libc::pid_t)?;
            PidfdReady::new(Arc::clone(&this.reactor), pidfd).await?;
            // The pidfd became readable: the child is reaped-ready.
            // This call is non-blocking in practice, not just in
            // signature — the OS has already done the waiting.
            this.inner.wait()
        })
    }

    fn id(&self) -> u32 {
        self.inner.id()
    }

    fn kill_tree(&self, sig: Signal) -> Result<()> {
        self.inner.kill_tree(sig)
    }

    fn kill_single(&self, sig: Signal) -> Result<()> {
        self.inner.kill_single(sig)
    }

    fn try_wait(&mut self) -> Result<Option<ExitStatus>> {
        self.inner.try_wait()
    }
}

/// Resolves once the wrapped pidfd is readable (RM-ASYNC-ENGINE-0001-
/// style completion orientation: this is a one-shot readiness-to-
/// completion translation, not a raw readiness stream). Registers with
/// the reactor on first poll and relies on the reactor calling
/// [`Waker::wake`] exactly once via `EPOLLONESHOT` — see
/// [`EpollReactor::register`].
struct PidfdReady {
    reactor: Arc<EpollReactor>,
    fd: OwnedFd,
    registered: bool,
}

impl PidfdReady {
    fn new(reactor: Arc<EpollReactor>, fd: OwnedFd) -> Self {
        Self {
            reactor,
            fd,
            registered: false,
        }
    }
}

impl Future for PidfdReady {
    type Output = Result<()>;

    fn poll(self: Pin<&mut Self>, cx: &mut Context<'_>) -> Poll<Result<()>> {
        // No self-referential state and no field needs pinning
        // (`OwnedFd`/`Arc`/`bool` are all `Unpin`) — `Self` is `Unpin`
        // automatically, so getting a plain `&mut Self` is sound.
        let this = self.get_mut();
        if !this.registered {
            let raw = this.fd.as_raw_fd();
            if let Err(e) = this.reactor.register(raw, cx.waker().clone()) {
                return Poll::Ready(Err(e));
            }
            this.registered = true;
            return Poll::Pending;
        }
        // Only reachable after the reactor observed readiness and woke
        // this future's waker (`EPOLLONESHOT` fires once, for exactly
        // this readiness edge) — ready by construction.
        Poll::Ready(Ok(()))
    }
}
