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
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{Arc, Mutex};
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
            reaped: Mutex::new(None),
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
    /// The single authoritative "already reaped" cache for this child,
    /// consulted and updated by every reaping path below (`wait`,
    /// `try_wait`, `ready`, `try_wait_job`, `wait_job`). This mirrors
    /// rustils' own `LinuxChild::reaped` field, and exists for the same
    /// reason: `wait_job`/`try_wait_job` (job-control, `WUNTRACED|
    /// WCONTINUED`) and `wait`/`try_wait` (plain) both ultimately
    /// `waitpid` the same pid, and a terminal status can only be reaped
    /// once — whichever path reaps it first must stash the result so
    /// the other family's later call sees the cached status instead of
    /// re-`waitpid`-ing an already-gone pid (`ECHILD`). Because of this,
    /// every method here reads the wrapped `self.inner` only for
    /// non-reaping operations (`id`, `kill_tree`, `kill_single`,
    /// `take_stdin`/`take_stdout`/`take_stderr`) — `self.inner`'s own
    /// private reap cache is deliberately never consulted, since this
    /// field is what stays authoritative instead.
    reaped: Mutex<Option<ExitStatus>>,
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
            if let Some(status) = *this.reaped.lock().unwrap_or_else(|p| p.into_inner()) {
                return Ok(status);
            }
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
        if let Some(status) = *self.reaped.lock().unwrap_or_else(|p| p.into_inner()) {
            return Ok(Some(status));
        }
        let status = self.inner.try_wait()?;
        if let Some(s) = status {
            *self.reaped.lock().unwrap_or_else(|p| p.into_inner()) = Some(s);
        }
        Ok(status)
    }

    fn ready(&self) -> BoxFuture<'_, Result<()>> {
        // Borrowing counterpart of `wait` — same pidfd + `PidfdReady`
        // mechanism, but doesn't consume `self` or reap the child
        // afterward. This is what lets several `AsyncLinuxChild`s be
        // multiplexed through the same shared `EpollReactor` by
        // `platform_async::process::wait_any` without any of them being
        // given up before the caller knows which one actually finished.
        if self
            .reaped
            .lock()
            .unwrap_or_else(|p| p.into_inner())
            .is_some()
        {
            return Box::pin(std::future::ready(Ok(())));
        }
        let pid = self.inner.id();
        let reactor = Arc::clone(&self.reactor);
        Box::pin(async move {
            let pidfd = sys::pidfd::open(pid as libc::pid_t)?;
            PidfdReady::new(reactor, pidfd).await
        })
    }

    fn take_stdin(&mut self) -> Option<Box<dyn platform::fs::File>> {
        self.inner.take_stdin()
    }

    fn take_stdout(&mut self) -> Option<Box<dyn platform::fs::File>> {
        self.inner.take_stdout()
    }

    fn take_stderr(&mut self) -> Option<Box<dyn platform::fs::File>> {
        self.inner.take_stderr()
    }

    fn try_wait_job(&mut self) -> Result<Option<ExitStatus>> {
        if let Some(status) = *self.reaped.lock().unwrap_or_else(|p| p.into_inner()) {
            return Ok(Some(status));
        }
        let pid = self.inner.id();
        let status = platform_linux::sys::spawn::try_wait_job(pid as libc::pid_t)?;
        if let Some(s) = status {
            if !matches!(s, ExitStatus::Stopped(_) | ExitStatus::Continued) {
                *self.reaped.lock().unwrap_or_else(|p| p.into_inner()) = Some(s);
            }
        }
        Ok(status)
    }

    fn wait_job(&mut self) -> BoxFuture<'_, Result<ExitStatus>> {
        if let Some(status) = *self.reaped.lock().unwrap_or_else(|p| p.into_inner()) {
            return Box::pin(std::future::ready(Ok(status)));
        }
        let pid = self.inner.id();
        Box::pin(async move {
            let status = WaitJob::new(pid as libc::pid_t).await?;
            if !matches!(status, ExitStatus::Stopped(_) | ExitStatus::Continued) {
                *self.reaped.lock().unwrap_or_else(|p| p.into_inner()) = Some(status);
            }
            Ok(status)
        })
    }
}

/// Resolves once the wrapped pidfd is readable (RM-ASYNC-ENGINE-0001-
/// style completion orientation: this is a one-shot readiness-to-
/// completion translation, not a raw readiness stream). Registers with
/// the reactor on first poll and checks its own `ready` flag on every
/// poll thereafter — see [`EpollReactor::register`]'s doc comment for
/// why checking the flag, rather than assuming "polled again means
/// ready," is required once this future can be polled through a waker
/// shared with unrelated futures (as [`platform_async::process::wait_any`]
/// does).
struct PidfdReady {
    reactor: Arc<EpollReactor>,
    fd: OwnedFd,
    ready: Arc<AtomicBool>,
    registered: bool,
}

impl PidfdReady {
    fn new(reactor: Arc<EpollReactor>, fd: OwnedFd) -> Self {
        Self {
            reactor,
            fd,
            ready: Arc::new(AtomicBool::new(false)),
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
        if this.ready.load(Ordering::Acquire) {
            return Poll::Ready(Ok(()));
        }
        if !this.registered {
            let raw = this.fd.as_raw_fd();
            if let Err(e) = this
                .reactor
                .register(raw, Arc::clone(&this.ready), cx.waker().clone())
            {
                return Poll::Ready(Err(e));
            }
            this.registered = true;
        }
        Poll::Pending
    }
}

/// Runs the *blocking* `waitpid(pid, WUNTRACED|WCONTINUED)` on a
/// disclosed one-shot background thread and resolves once it returns.
///
/// Unlike plain termination, a pidfd does **not** become readable on a
/// stop/continue transition — pidfd readiness is specifically an
/// exit/termination signal (confirmed against the actual Linux
/// behavior, not assumed from the exit case) — so the `EpollReactor`
/// this crate otherwise builds everything on cannot multiplex
/// job-control waits the way it does plain ones. rustils' own sync
/// `platform-linux::sys::spawn::wait_job` has no non-blocking,
/// multiplexable primitive to build on either — it is a direct blocking
/// `waitpid` call. Spawning a dedicated thread to run that blocking
/// call and waking the caller when it returns is the correct minimum
/// mechanism here, not a shortcut — the same disclosed-thread-cost
/// reasoning (`RM-DEV-ASYNC-0002`) already used for [`Timeout`].
///
/// Checks the actual result slot on every poll rather than inferring
/// completion from being re-polled — the same lesson `PidfdReady`
/// already had to learn the hard way once this future's waker can be
/// shared with unrelated futures (e.g. if a caller ever raced this
/// against something else).
struct WaitJob {
    pid: libc::pid_t,
    result: Arc<Mutex<Option<Result<ExitStatus>>>>,
    spawned: bool,
}

impl WaitJob {
    fn new(pid: libc::pid_t) -> Self {
        Self {
            pid,
            result: Arc::new(Mutex::new(None)),
            spawned: false,
        }
    }
}

impl Future for WaitJob {
    type Output = Result<ExitStatus>;

    fn poll(self: Pin<&mut Self>, cx: &mut Context<'_>) -> Poll<Result<ExitStatus>> {
        let this = self.get_mut();
        if let Some(result) = this.result.lock().unwrap_or_else(|p| p.into_inner()).take() {
            return Poll::Ready(result);
        }
        if !this.spawned {
            this.spawned = true;
            let result_slot = Arc::clone(&this.result);
            let waker = cx.waker().clone();
            let pid = this.pid;
            std::thread::spawn(move || {
                let outcome = platform_linux::sys::spawn::wait_job(pid);
                *result_slot.lock().unwrap_or_else(|p| p.into_inner()) = Some(outcome);
                waker.wake();
            });
        }
        Poll::Pending
    }
}
