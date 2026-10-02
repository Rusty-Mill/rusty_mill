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
use std::os::fd::{AsFd, AsRawFd, OwnedFd};
use std::pin::Pin;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{Arc, Mutex};
use std::task::{Context, Poll};

use platform::error::{ErrorKind, OsCode, PlatformError, Result};
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
        let signal_pidfd = match pidfd_open(child.id() as libc::pid_t) {
            Ok(pidfd) => pidfd,
            Err(acquisition_error) => {
                cleanup_unexposed_child(child)?;
                return Err(acquisition_error);
            }
        };
        Ok(Box::new(AsyncLinuxChild {
            inner: child,
            signal_pidfd,
            reactor: Arc::clone(&self.reactor),
            reaped: Arc::new(Mutex::new(None)),
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

    fn is_zombie(&self, pid: u32) -> Result<bool> {
        self.inner.is_zombie(pid)
    }
}

/// Dispose of a child whose signaling pidfd could not be acquired.
///
/// The child has not been exposed and no helper can have reaped it, so its
/// numeric PID still belongs to this operation under the spawn ownership
/// assumptions documented in the crate README. `ESRCH` is harmless here: an
/// immediately exited child remains waitable. Other signal failures are
/// returned rather than risking an unbounded blocking wait.
fn cleanup_unexposed_child(mut child: Box<dyn Child>) -> Result<()> {
    match cleanup_kill(&*child) {
        Ok(()) => {}
        Err(error) if error.os == OsCode::Errno(libc::ESRCH) => {}
        Err(error) => return Err(error),
    }
    #[cfg(test)]
    syscall_test_hook::pipes_released((
        child.take_stdin().is_some(),
        child.take_stdout().is_some(),
        child.take_stderr().is_some(),
    ));
    #[cfg(not(test))]
    {
        child.take_stdin();
        child.take_stdout();
        child.take_stderr();
    }
    cleanup_wait(child).map(|_| ())
}

#[cfg(not(test))]
fn pidfd_open(pid: libc::pid_t) -> Result<OwnedFd> {
    sys::pidfd::open(pid)
}

#[cfg(not(test))]
fn cleanup_kill(child: &dyn Child) -> Result<()> {
    child.kill_single(Signal::Kill)
}

#[cfg(not(test))]
fn cleanup_wait(child: Box<dyn Child>) -> Result<ExitStatus> {
    child.wait()
}

#[cfg(not(test))]
fn pidfd_send_signal(pidfd: std::os::fd::BorrowedFd<'_>, signal: Signal) -> Result<()> {
    sys::pidfd::send_signal(pidfd, signal)
}

#[cfg(test)]
mod syscall_test_hook {
    use super::*;
    use std::cell::RefCell;
    use std::os::fd::BorrowedFd;

    #[derive(Clone, Copy)]
    pub(super) struct ErrorSpec {
        pub(super) kind: ErrorKind,
        pub(super) errno: i32,
        pub(super) op: &'static str,
    }

    impl ErrorSpec {
        fn make(self) -> PlatformError {
            PlatformError::new(self.kind, OsCode::Errno(self.errno), self.op)
        }
    }

    #[derive(Default)]
    pub(super) struct State {
        pub(super) open_error: Option<ErrorSpec>,
        pub(super) kill_error: Option<ErrorSpec>,
        pub(super) wait_error: Option<ErrorSpec>,
        pub(super) signal_error: Option<ErrorSpec>,
        pub(super) opened_pid: Option<libc::pid_t>,
        pub(super) open_calls: usize,
        pub(super) kill_calls: usize,
        pub(super) wait_calls: usize,
        pub(super) signal_fds: Vec<i32>,
        pub(super) numeric_signal_calls: usize,
        pub(super) released_pipes: Option<(bool, bool, bool)>,
        pub(super) wait_helper_starts: usize,
        pub(super) before_open: Option<fn(libc::pid_t)>,
    }

    thread_local! {
        static STATE: RefCell<Option<State>> = const { RefCell::new(None) };
    }

    pub(super) fn with<R>(state: State, f: impl FnOnce() -> R) -> (R, State) {
        STATE.with(|slot| {
            assert!(slot.borrow().is_none(), "nested syscall test hook");
            *slot.borrow_mut() = Some(state);
        });
        struct Clear;
        impl Drop for Clear {
            fn drop(&mut self) {
                STATE.with(|slot| {
                    slot.borrow_mut().take();
                });
            }
        }
        let clear = Clear;
        let result = f();
        let state = STATE.with(|slot| slot.borrow_mut().take().expect("hook installed"));
        std::mem::forget(clear);
        (result, state)
    }

    pub(super) fn open(pid: libc::pid_t) -> Result<OwnedFd> {
        let before = STATE.with(|slot| slot.borrow().as_ref().and_then(|state| state.before_open));
        if let Some(before) = before {
            // The callback is deliberately allowed to panic. Own the spawned
            // PID until it returns so test failures cannot leak a child or
            // zombie before `spawn` gets a chance to run normal cleanup.
            struct CallbackChild(libc::pid_t);
            impl Drop for CallbackChild {
                fn drop(&mut self) {
                    let _ = platform_linux::sys::spawn::kill_single(self.0, Signal::Kill);
                    let _ = platform_linux::sys::spawn::wait(self.0);
                }
            }
            let owned = CallbackChild(pid);
            before(pid);
            std::mem::forget(owned);
        }
        let injected = STATE.with(|slot| {
            let mut slot = slot.borrow_mut();
            slot.as_mut().and_then(|state| {
                state.open_calls += 1;
                state.opened_pid = Some(pid);
                state.open_error
            })
        });
        match injected {
            Some(error) => Err(error.make()),
            None => sys::pidfd::open(pid),
        }
    }

    pub(super) fn kill(child: &dyn Child) -> Result<()> {
        let injected = STATE.with(|slot| {
            slot.borrow_mut().as_mut().and_then(|state| {
                state.kill_calls += 1;
                state.numeric_signal_calls += 1;
                state.kill_error
            })
        });
        match injected {
            Some(error) => Err(error.make()),
            None => child.kill_single(Signal::Kill),
        }
    }

    pub(super) fn pipes_released(pipes: (bool, bool, bool)) {
        STATE.with(|slot| {
            if let Some(state) = slot.borrow_mut().as_mut() {
                state.released_pipes = Some(pipes);
            }
        });
    }

    pub(super) fn wait_helper_started() {
        STATE.with(|slot| {
            if let Some(state) = slot.borrow_mut().as_mut() {
                state.wait_helper_starts += 1;
            }
        });
    }

    pub(super) fn wait(child: Box<dyn Child>) -> Result<ExitStatus> {
        let injected = STATE.with(|slot| {
            slot.borrow_mut().as_mut().and_then(|state| {
                state.wait_calls += 1;
                state.wait_error
            })
        });
        match injected {
            Some(error) => Err(error.make()),
            None => child.wait(),
        }
    }

    pub(super) fn send_signal(pidfd: BorrowedFd<'_>, signal: Signal) -> Result<()> {
        let injected = STATE.with(|slot| {
            slot.borrow_mut().as_mut().and_then(|state| {
                state.signal_fds.push(pidfd.as_raw_fd());
                state.signal_error
            })
        });
        match injected {
            Some(error) => Err(error.make()),
            None => sys::pidfd::send_signal(pidfd, signal),
        }
    }
}

#[cfg(test)]
fn pidfd_open(pid: libc::pid_t) -> Result<OwnedFd> {
    syscall_test_hook::open(pid)
}

#[cfg(test)]
fn cleanup_kill(child: &dyn Child) -> Result<()> {
    syscall_test_hook::kill(child)
}

#[cfg(test)]
fn cleanup_wait(child: Box<dyn Child>) -> Result<ExitStatus> {
    syscall_test_hook::wait(child)
}

#[cfg(test)]
fn pidfd_send_signal(pidfd: std::os::fd::BorrowedFd<'_>, signal: Signal) -> Result<()> {
    syscall_test_hook::send_signal(pidfd, signal)
}

struct AsyncLinuxChild {
    inner: Box<dyn Child>,
    /// Dedicated identity handle for every `kill_single` call. Readiness uses
    /// independently opened pidfds so reactor deregistration keeps its
    /// existing open-file-description lifetime.
    signal_pidfd: OwnedFd,
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
    ///
    /// Shared with a [`WaitJob`] helper thread, which records a terminal
    /// status here itself: the thread reaps even if its future was
    /// dropped, and the status must not be lost with it.
    reaped: Arc<Mutex<Option<ExitStatus>>>,
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
        // A reaped pid may be recycled; see `LinuxChild::kill_single`.
        if self
            .reaped
            .lock()
            .unwrap_or_else(|p| p.into_inner())
            .is_some()
        {
            return Ok(());
        }
        pidfd_send_signal(self.signal_pidfd.as_fd(), sig)
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
            // The helper thread records a terminal status in `reaped`.
            WaitJob::new(pid as libc::pid_t, Arc::clone(&self.reaped)).await
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
        } else {
            // Later polls may come from another task or executor: point the
            // registration at the current waker, then re-check `ready` in
            // case the fire path ran (and woke the old waker) in between.
            this.reactor.update_waker(this.fd.as_raw_fd(), cx.waker());
            if this.ready.load(Ordering::Acquire) {
                return Poll::Ready(Ok(()));
            }
        }
        Poll::Pending
    }
}

/// Deregisters this future's own fd from the reactor if it was ever
/// registered and the fire path hasn't already claimed it. Without
/// this, every `wait_any` sibling that loses the race — or every
/// child abandoned when a timeout fires first — would leak its
/// registry entry forever, since [`EpollReactor::run`]'s fire path is
/// otherwise the *only* place an entry is ever removed.
///
/// Checking `ready` rather than unconditionally calling
/// [`EpollReactor::deregister`] matters once fd numbers can be
/// reused: `self.fd` is still open at this point (its own `Drop` runs
/// after this one, in field-declaration order), so no other
/// registration could have reused this exact fd number yet — but if
/// the fire path already removed and fired this entry (`ready` is
/// `true`), skipping the call avoids ever touching the registry for a
/// no-longer-ours fd number on the (unrelated) off chance one showed
/// up between the fire and this drop.
impl Drop for PidfdReady {
    fn drop(&mut self) {
        if self.registered && !self.ready.load(Ordering::Acquire) {
            self.reactor.deregister(self.fd.as_raw_fd());
        }
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
///
/// Cancellation: dropping this future does not stop the thread, which may
/// still reap the child. So the thread itself records a terminal status
/// in the child's `reaped` cache, before publishing its result; a later
/// `kill_single`/`try_wait` then sees the child as reaped instead of
/// acting on a pid that may be recycled.
struct WaitJob {
    pid: libc::pid_t,
    reaped: Arc<Mutex<Option<ExitStatus>>>,
    result: Arc<Mutex<Option<Result<ExitStatus>>>>,
    spawned: bool,
    /// The latest poller's waker (design review 4): the thread used to
    /// capture the first poll's waker only.
    waker: platform_async::waker_slot::WakerSlot,
}

impl WaitJob {
    fn new(pid: libc::pid_t, reaped: Arc<Mutex<Option<ExitStatus>>>) -> Self {
        Self {
            pid,
            reaped,
            result: Arc::new(Mutex::new(None)),
            spawned: false,
            waker: platform_async::waker_slot::WakerSlot::new(),
        }
    }
}

impl Future for WaitJob {
    type Output = Result<ExitStatus>;

    fn poll(self: Pin<&mut Self>, cx: &mut Context<'_>) -> Poll<Result<ExitStatus>> {
        let this = self.get_mut();
        // Update before checking the result: see `WakerSlot`'s ordering note.
        this.waker.update(cx.waker());
        if let Some(result) = this.result.lock().unwrap_or_else(|p| p.into_inner()).take() {
            return Poll::Ready(result);
        }
        if !this.spawned {
            this.spawned = true;
            #[cfg(test)]
            syscall_test_hook::wait_helper_started();
            let result_slot = Arc::clone(&this.result);
            let reaped = Arc::clone(&this.reaped);
            let waker = this.waker.clone();
            let pid = this.pid;
            let spawned = std::thread::Builder::new()
                .name("rustils-async-waitjob".to_owned())
                .spawn(move || {
                    let outcome = platform_linux::sys::spawn::wait_job(pid);
                    #[cfg(test)]
                    let publication = wait_job_test_hook::after_waitpid(pid);
                    #[cfg(test)]
                    let may_publish = publication.as_ref().is_none_or(|(allowed, _)| *allowed);
                    #[cfg(not(test))]
                    let may_publish = true;
                    if may_publish {
                        if let Ok(status) = &outcome {
                            if !matches!(status, ExitStatus::Stopped(_) | ExitStatus::Continued) {
                                *reaped.lock().unwrap_or_else(|p| p.into_inner()) = Some(*status);
                            }
                        }
                    }
                    #[cfg(test)]
                    if let Some((allowed, published)) = publication {
                        let _ = published.send(allowed);
                    }
                    *result_slot.lock().unwrap_or_else(|p| p.into_inner()) = Some(outcome);
                    waker.wake();
                });
            if let Err(e) = spawned {
                return Poll::Ready(Err(PlatformError::new(
                    ErrorKind::Other,
                    OsCode::Errno(e.raw_os_error().unwrap_or(0)),
                    "spawn waitjob thread",
                )));
            }
        }
        Poll::Pending
    }
}

#[cfg(test)]
mod wait_job_test_hook {
    use std::sync::{mpsc, Mutex};

    struct Hook {
        pid: libc::pid_t,
        reaped: mpsc::SyncSender<()>,
        publish: mpsc::Receiver<()>,
        published: mpsc::SyncSender<bool>,
    }

    static HOOK: Mutex<Option<Hook>> = Mutex::new(None);

    pub(super) struct Guard {
        pid: libc::pid_t,
        release: mpsc::SyncSender<()>,
    }

    impl Guard {
        pub(super) fn release(&self) {
            let _ = self.release.try_send(());
        }
    }

    impl Drop for Guard {
        fn drop(&mut self) {
            let _ = self.release.try_send(());
            let mut slot = HOOK.lock().unwrap_or_else(|p| p.into_inner());
            if slot.as_ref().is_some_and(|hook| hook.pid == self.pid) {
                slot.take();
            }
        }
    }

    pub(super) fn install(
        pid: libc::pid_t,
        reaped: mpsc::SyncSender<()>,
        release: mpsc::SyncSender<()>,
        publish: mpsc::Receiver<()>,
        published: mpsc::SyncSender<bool>,
    ) -> Guard {
        *HOOK.lock().unwrap_or_else(|p| p.into_inner()) = Some(Hook {
            pid,
            reaped,
            publish,
            published,
        });
        Guard { pid, release }
    }

    pub(super) fn after_waitpid(pid: libc::pid_t) -> Option<(bool, mpsc::SyncSender<bool>)> {
        let mut slot = HOOK.lock().unwrap_or_else(|p| p.into_inner());
        let hook = (slot.as_ref().is_some_and(|hook| hook.pid == pid))
            .then(|| slot.take().expect("matching hook exists"));
        drop(slot);
        if let Some(hook) = hook {
            let _ = hook.reaped.send(());
            let allowed = hook
                .publish
                .recv_timeout(std::time::Duration::from_secs(5))
                .is_ok();
            return Some((allowed, hook.published));
        }
        None
    }
}

#[cfg(test)]
mod wait_any_leak_tests {
    use super::*;
    use platform::process::Stdio;
    use std::sync::atomic::AtomicUsize;
    use std::task::Wake;
    use std::time::Duration;

    struct CountingWaker(AtomicUsize);

    struct OwnedPid(libc::pid_t);

    /// Test-only decorator at the wrapped [`Child::kill_single`] boundary.
    /// Unlike the acquisition-cleanup hook, this observes any accidental
    /// numeric-PID signal made through an exposed `AsyncLinuxChild`'s actual
    /// inner child.
    struct ObservedChild {
        inner: Box<dyn Child>,
        kill_single_calls: Arc<AtomicUsize>,
    }

    impl Child for ObservedChild {
        fn wait(self: Box<Self>) -> Result<ExitStatus> {
            self.inner.wait()
        }

        fn id(&self) -> u32 {
            self.inner.id()
        }

        fn kill_tree(&self, sig: Signal) -> Result<()> {
            self.inner.kill_tree(sig)
        }

        fn kill_single(&self, sig: Signal) -> Result<()> {
            self.kill_single_calls.fetch_add(1, Ordering::SeqCst);
            self.inner.kill_single(sig)
        }

        fn try_wait(&mut self) -> Result<Option<ExitStatus>> {
            self.inner.try_wait()
        }

        fn wait_job(&mut self) -> Result<ExitStatus> {
            self.inner.wait_job()
        }

        fn try_wait_job(&mut self) -> Result<Option<ExitStatus>> {
            self.inner.try_wait_job()
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

        fn as_any_mut(&mut self) -> &mut dyn std::any::Any {
            self
        }
    }

    fn observed_child(
        reactor: Arc<EpollReactor>,
        command: &Command,
    ) -> (Box<AsyncLinuxChild>, Arc<AtomicUsize>) {
        let inner = LinuxSpawner.spawn(command).expect("spawn observed child");
        let signal_pidfd = sys::pidfd::open(inner.id() as libc::pid_t).expect("open signal pidfd");
        let calls = Arc::new(AtomicUsize::new(0));
        let inner = Box::new(ObservedChild {
            inner,
            kill_single_calls: Arc::clone(&calls),
        });
        (
            Box::new(AsyncLinuxChild {
                inner,
                signal_pidfd,
                reactor,
                reaped: Arc::new(Mutex::new(None)),
            }),
            calls,
        )
    }

    impl Drop for OwnedPid {
        fn drop(&mut self) {
            let _ = platform_linux::sys::spawn::kill_single(self.0, Signal::Kill);
            let _ = platform_linux::sys::spawn::wait(self.0);
        }
    }

    fn injected(errno: i32, op: &'static str) -> syscall_test_hook::ErrorSpec {
        syscall_test_hook::ErrorSpec {
            kind: ErrorKind::Other,
            errno,
            op,
        }
    }

    fn isolated(marker: &str, test: &str) -> bool {
        if std::env::var_os(marker).is_some() {
            return true;
        }
        let status = std::process::Command::new(std::env::current_exe().expect("test binary"))
            .arg("--exact")
            .arg(test)
            .arg("--nocapture")
            .env(marker, "1")
            .status()
            .expect("run isolated descriptor test");
        assert!(status.success(), "isolated descriptor test failed");
        false
    }

    fn wait_until_zombie(pid: libc::pid_t) {
        let deadline = std::time::Instant::now() + Duration::from_secs(5);
        loop {
            let status = std::fs::read_to_string(format!("/proc/{pid}/status"))
                .expect("owned child remains present until it is reaped");
            if status.lines().any(|line| line.starts_with("State:\tZ")) {
                return;
            }
            assert!(std::time::Instant::now() < deadline, "child did not exit");
            std::thread::sleep(Duration::from_millis(5));
        }
    }

    #[test]
    fn spawn_acquisition_errors_clean_running_and_exited_children() {
        for (program, args, before_open) in [
            ("/bin/sleep", vec!["30"], None),
            (
                "/bin/true",
                Vec::new(),
                Some(wait_until_zombie as fn(libc::pid_t)),
            ),
        ] {
            for errno in [libc::ENOSYS, libc::EMFILE] {
                let spawner = AsyncLinuxSpawner::new().expect("spawner");
                let mut cmd = Command::new(program, "/");
                for arg in &args {
                    cmd = cmd.arg(arg);
                }
                cmd.stdin = Stdio::Pipe;
                cmd.stdout = Stdio::Pipe;
                cmd.stderr = Stdio::Pipe;
                let (result, state) = syscall_test_hook::with(
                    syscall_test_hook::State {
                        open_error: Some(syscall_test_hook::ErrorSpec {
                            kind: if errno == libc::ENOSYS {
                                ErrorKind::Unsupported
                            } else {
                                ErrorKind::Other
                            },
                            errno,
                            op: "injected pidfd_open",
                        }),
                        before_open,
                        ..Default::default()
                    },
                    || spawner.spawn(&cmd),
                );
                let error = result.err().expect("acquisition must fail");
                assert_eq!(error.os, OsCode::Errno(errno));
                assert_eq!(error.op, "injected pidfd_open");
                assert_eq!(
                    (state.open_calls, state.kill_calls, state.wait_calls),
                    (1, 1, 1)
                );
                assert_eq!(state.numeric_signal_calls, 1, "numeric-signal spy control");
                assert_eq!(state.released_pipes, Some((true, true, true)));
                assert_eq!(state.wait_helper_starts, 0);
                let pid = state.opened_pid.expect("spawn happened before acquisition");
                assert_eq!(
                    platform_linux::sys::spawn::try_wait(pid).unwrap_err().os,
                    OsCode::Errno(libc::ECHILD),
                    "cleanup must reap the owned child"
                );
            }
        }
    }

    #[test]
    fn cleanup_failure_is_contextual_and_never_waits_after_failed_termination() {
        let spawner = AsyncLinuxSpawner::new().expect("spawner");
        let (result, state) = syscall_test_hook::with(
            syscall_test_hook::State {
                open_error: Some(injected(libc::EMFILE, "injected pidfd_open")),
                kill_error: Some(injected(libc::EPERM, "cleanup SIGKILL")),
                ..Default::default()
            },
            || spawner.spawn(&Command::new("/bin/sleep", "/").arg("30")),
        );
        let owned = OwnedPid(state.opened_pid.expect("child was created"));
        let error = result.err().expect("cleanup termination must fail");
        assert_eq!(
            (error.op, error.os),
            ("cleanup SIGKILL", OsCode::Errno(libc::EPERM))
        );
        assert_eq!((state.kill_calls, state.wait_calls), (1, 0));
        drop(owned);
    }

    #[test]
    fn cleanup_wait_failure_is_returned_and_owned_child_can_still_be_reaped() {
        let spawner = AsyncLinuxSpawner::new().expect("spawner");
        let (result, state) = syscall_test_hook::with(
            syscall_test_hook::State {
                open_error: Some(injected(libc::EMFILE, "injected pidfd_open")),
                wait_error: Some(injected(libc::EIO, "cleanup wait")),
                ..Default::default()
            },
            || spawner.spawn(&Command::new("/bin/sleep", "/").arg("30")),
        );
        let owned = OwnedPid(state.opened_pid.expect("child was created"));
        let error = result.err().expect("cleanup wait must fail");
        assert_eq!(
            (error.op, error.os),
            ("cleanup wait", OsCode::Errno(libc::EIO))
        );
        assert_eq!((state.kill_calls, state.wait_calls), (1, 1));
        drop(owned);
    }

    #[test]
    fn creation_failure_never_attempts_pidfd_acquisition() {
        let spawner = AsyncLinuxSpawner::new().expect("spawner");
        let (result, state) = syscall_test_hook::with(Default::default(), || {
            spawner.spawn(&Command::new("/definitely/not/a/program", "/"))
        });
        assert!(result.is_err());
        assert_eq!(state.open_calls, 0);
        assert!(state.opened_pid.is_none());
    }

    #[test]
    fn exposed_child_routes_signal_to_retained_fd_and_preserves_error() {
        let spawner = AsyncLinuxSpawner::new().expect("spawner");
        let (mut child, numeric_calls) = observed_child(
            Arc::clone(&spawner.reactor),
            &Command::new("/bin/sleep", "/").arg("30"),
        );
        // Positive control: the decorator observes a call made directly at
        // the wrapped-child boundary that the exposed path must not use.
        child
            .inner
            .kill_single(Signal::Cont)
            .expect("direct inner signal control");
        assert_eq!(numeric_calls.swap(0, Ordering::SeqCst), 1);
        let (result, state) = syscall_test_hook::with(
            syscall_test_hook::State {
                signal_error: Some(injected(libc::EIO, "injected pidfd_send_signal")),
                ..Default::default()
            },
            || child.kill_single(Signal::Term),
        );
        let error = result.expect_err("non-ESRCH pidfd failure must be preserved");
        assert_eq!(
            (error.op, error.os),
            ("injected pidfd_send_signal", OsCode::Errno(libc::EIO))
        );
        assert_eq!(
            state.signal_fds.len(),
            1,
            "the pidfd boundary is used exactly once"
        );
        assert_eq!(state.numeric_signal_calls, 0);
        assert_eq!(numeric_calls.load(Ordering::SeqCst), 0);
        assert_eq!(
            child.try_wait().expect("status check"),
            None,
            "no status is fabricated"
        );
        child
            .kill_single(Signal::Kill)
            .expect("real pidfd cleanup signal");
        block_on(child.wait()).expect("reap owned child");
    }

    #[test]
    fn retained_pidfd_is_cloexec_and_closes_when_child_is_dropped() {
        if !isolated(
            "RUSTY_MILL_PIDFD_CLOEXEC_CHILD",
            "wait_any_leak_tests::retained_pidfd_is_cloexec_and_closes_when_child_is_dropped",
        ) {
            return;
        }
        let spawner = AsyncLinuxSpawner::new().expect("spawner");
        let child = spawner
            .spawn(&Command::new("/bin/sleep", "/").arg("30"))
            .expect("spawn");
        let pid = child.id() as libc::pid_t;
        let (_, state) = syscall_test_hook::with(
            syscall_test_hook::State {
                signal_error: Some(injected(libc::EIO, "inspect retained pidfd")),
                ..Default::default()
            },
            || child.kill_single(Signal::Term),
        );
        let fd = state.signal_fds[0];
        let flags = sys::pidfd::descriptor_flags(fd);
        assert_ne!(flags, -1, "retained pidfd must be valid before inspection");
        assert_ne!(flags & libc::FD_CLOEXEC, 0);
        drop(child);
        assert_eq!(sys::pidfd::descriptor_flags(fd), -1);
        drop(OwnedPid(pid));
    }

    fn retained_fd(child: &dyn AsyncChild) -> i32 {
        let (_, state) = syscall_test_hook::with(
            syscall_test_hook::State {
                signal_error: Some(injected(libc::EIO, "inspect retained pidfd")),
                ..Default::default()
            },
            || child.kill_single(Signal::Term),
        );
        state.signal_fds[0]
    }

    #[test]
    fn consuming_wait_and_cancelled_wait_release_the_retained_pidfd() {
        if !isolated(
            "RUSTY_MILL_PIDFD_WAIT_LIFETIME_CHILD",
            "wait_any_leak_tests::consuming_wait_and_cancelled_wait_release_the_retained_pidfd",
        ) {
            return;
        }
        let spawner = AsyncLinuxSpawner::new().expect("spawner");
        let child = spawner
            .spawn(&Command::new("/bin/true", "/"))
            .expect("spawn exiting child");
        let fd = retained_fd(&*child);
        block_on(child.wait()).expect("consume wait");
        assert_eq!(sys::pidfd::descriptor_flags(fd), -1);

        let child = spawner
            .spawn(&Command::new("/bin/sleep", "/").arg("30"))
            .expect("spawn running child");
        let pid = child.id() as libc::pid_t;
        let fd = retained_fd(&*child);
        let mut wait = child.wait();
        let waker = std::task::Waker::from(Arc::new(CountingWaker(AtomicUsize::new(0))));
        assert!(wait
            .as_mut()
            .poll(&mut Context::from_waker(&waker))
            .is_pending());
        drop(wait);
        assert_eq!(sys::pidfd::descriptor_flags(fd), -1);
        drop(OwnedPid(pid));
    }

    #[test]
    fn retained_pidfd_can_be_descriptor_zero_in_an_isolated_process() {
        const MARKER: &str = "RUSTY_MILL_PIDFD_ZERO_CHILD";
        if !isolated(
            MARKER,
            "wait_any_leak_tests::retained_pidfd_can_be_descriptor_zero_in_an_isolated_process",
        ) {
            return;
        }
        let spawner = AsyncLinuxSpawner::new().expect("spawner");
        sys::pidfd::close_descriptor(0);
        let child = spawner
            .spawn(&Command::new("/bin/sleep", "/").arg("30"))
            .expect("spawn");
        let pid = child.id() as libc::pid_t;
        assert_eq!(retained_fd(&*child), 0, "fd zero is valid, not a sentinel");
        drop(child);
        drop(OwnedPid(pid));
    }

    #[test]
    fn stale_owner_paths_do_not_close_a_reused_descriptor_number() {
        if !isolated(
            "RUSTY_MILL_PIDFD_REUSE_CHILD",
            "wait_any_leak_tests::stale_owner_paths_do_not_close_a_reused_descriptor_number",
        ) {
            return;
        }
        use std::os::fd::{AsFd, AsRawFd};

        let spawner = AsyncLinuxSpawner::new().expect("spawner");
        let child = spawner
            .spawn(&Command::new("/bin/true", "/"))
            .expect("spawn");
        let pid = child.id() as libc::pid_t;
        let reservation = std::fs::File::open("/dev/null").expect("reserve descriptor number");
        let stale_number = reservation.as_raw_fd();
        sys::pidfd::close_descriptor(stale_number);
        std::mem::forget(reservation);
        let mut original_activity = child.ready();
        let waker = std::task::Waker::from(Arc::new(CountingWaker(AtomicUsize::new(0))));
        let mut cx = Context::from_waker(&waker);
        assert!(original_activity.as_mut().poll(&mut cx).is_pending());
        assert_ne!(
            sys::pidfd::descriptor_flags(stale_number),
            -1,
            "original readiness activity must own the reserved descriptor"
        );
        let readiness_deadline = std::time::Instant::now() + Duration::from_secs(5);
        loop {
            match original_activity.as_mut().poll(&mut cx) {
                Poll::Ready(result) => {
                    result.expect("original child readiness");
                    break;
                }
                Poll::Pending => {
                    assert!(
                        std::time::Instant::now() < readiness_deadline,
                        "original child readiness did not complete before the deadline"
                    );
                    std::thread::sleep(Duration::from_millis(10));
                }
            }
        }
        assert_eq!(sys::pidfd::descriptor_flags(stale_number), -1);

        let replacement = std::fs::File::open("/dev/null").expect("replacement descriptor");
        let replacement_number = replacement.as_raw_fd();
        assert_eq!(
            sys::pidfd::replace_descriptor(replacement.as_fd(), stale_number),
            stale_number
        );
        assert_ne!(sys::pidfd::descriptor_flags(stale_number), -1);

        // The completed future still belongs to the original child. Dropping
        // it after its readiness pidfd legitimately closed must not perform a
        // stale close on a descriptor that now belongs to the replacement.
        drop(original_activity);
        assert_ne!(sys::pidfd::descriptor_flags(stale_number), -1);

        // Exercise both signaling and owner-drop paths on another child. A
        // stale owner of `stale_number` would incorrectly affect replacement.
        let other = spawner
            .spawn(&Command::new("/bin/sleep", "/").arg("30"))
            .expect("spawn control child");
        let other_pid = other.id() as libc::pid_t;
        other.kill_single(Signal::Kill).expect("pidfd signal path");
        drop(other);
        assert_ne!(sys::pidfd::descriptor_flags(stale_number), -1);
        if replacement_number != stale_number {
            sys::pidfd::close_descriptor(stale_number);
        }
        drop(replacement);
        drop(child);
        drop(OwnedPid(pid));
        drop(OwnedPid(other_pid));
    }

    impl Wake for CountingWaker {
        fn wake(self: Arc<Self>) {
            self.0.fetch_add(1, Ordering::SeqCst);
        }

        fn wake_by_ref(self: &Arc<Self>) {
            self.0.fetch_add(1, Ordering::SeqCst);
        }
    }

    /// Same tiny blocking-with-timeout executor `tests/spawn_and_wait.rs`
    /// uses — duplicated here because unit tests (this module, compiled
    /// with `--cfg test` as part of the library itself, which is what
    /// lets it reach the `#[cfg(test)]` `EpollReactor::registered_len`
    /// introspection below) and that separate integration-test binary
    /// cannot share code.
    fn block_on<F: Future>(fut: F) -> F::Output {
        let woken = Arc::new(CountingWaker(AtomicUsize::new(0)));
        let waker = std::task::Waker::from(Arc::clone(&woken));
        let mut cx = Context::from_waker(&waker);
        let mut fut = Box::pin(fut);
        loop {
            match fut.as_mut().poll(&mut cx) {
                Poll::Ready(v) => return v,
                Poll::Pending => {
                    let deadline = std::time::Instant::now() + Duration::from_secs(5);
                    while woken.0.load(Ordering::SeqCst) == 0 {
                        assert!(
                            std::time::Instant::now() < deadline,
                            "reactor never woke the waiting future"
                        );
                        std::thread::sleep(Duration::from_millis(5));
                    }
                    woken.0.store(0, Ordering::SeqCst);
                }
            }
        }
    }

    /// Regression test for the `EpollReactor` registry leak (monorepo
    /// review finding 49): `wait_any`'s losing sibling — here the slow
    /// child, still pending when the fast one wins — used to leave its
    /// pidfd registered in the reactor forever, since nothing ever
    /// deregistered it once its `PidfdReady` future was dropped instead
    /// of polled to `Ready`. Races a fast and a slow real child through
    /// the real `wait_any`/`EpollReactor` path; `block_on` drops the
    /// `WaitAny` future (and therefore the slow child's still-pending
    /// `ready()`/`PidfdReady`) before returning, exactly like any real
    /// caller does once it has its answer.
    #[test]
    fn losing_wait_any_sibling_is_deregistered_on_drop() {
        let spawner = AsyncLinuxSpawner::new().expect("construct reactor");
        let fast = spawner
            .spawn(&Command::new("/bin/true", "/"))
            .expect("spawn fast child");
        let slow = spawner
            .spawn(&Command::new("/bin/sleep", "/").arg("30"))
            .expect("spawn slow child");
        let mut children: Vec<Box<dyn AsyncChild>> = vec![fast, slow];

        let index = block_on(spawner.wait_any(&mut children, Some(Duration::from_secs(5))))
            .expect("wait_any")
            .expect("Some(index), not a timeout");
        assert_eq!(index, 0, "the fast child should win the race");

        assert_eq!(
            spawner.reactor.registered_len(),
            0,
            "losing wait_any sibling's registry entry leaked once WaitAny was dropped"
        );

        let slow_child = children.remove(1);
        let _ = slow_child.kill_single(Signal::Kill);
        let _ = block_on(slow_child.wait());
    }

    /// Review 1.6 follow-up: a `wait_job` future dropped after its first
    /// poll leaves its helper thread blocked in `waitpid`. When the child
    /// exits, the thread reaps it; the status must still reach the child's
    /// cache, so `try_wait` answers from it (not `ECHILD`) and
    /// `kill_single` sends nothing to the possibly recycled pid.
    #[test]
    fn an_abandoned_wait_job_still_records_the_reap() {
        let spawner = AsyncLinuxSpawner::new().expect("spawner");
        let mut child = spawner
            .spawn(
                &Command::new("/bin/sh", "/")
                    .arg("-c")
                    .arg("sleep 0.2; exit 3"),
            )
            .expect("spawn");
        {
            let waker = std::task::Waker::from(Arc::new(CountingWaker(AtomicUsize::new(0))));
            let mut cx = Context::from_waker(&waker);
            let mut abandoned = child.wait_job();
            assert!(abandoned.as_mut().poll(&mut cx).is_pending());
        }
        // Gone from /proc: the helper thread has reaped it, zombie and all.
        let proc_entry = format!("/proc/{}", child.id());
        let deadline = std::time::Instant::now() + Duration::from_secs(5);
        while std::path::Path::new(&proc_entry).exists() {
            assert!(std::time::Instant::now() < deadline, "child never reaped");
            std::thread::sleep(Duration::from_millis(10));
        }
        assert_eq!(
            child.try_wait().expect("cached, not ECHILD"),
            Some(ExitStatus::Code(3))
        );
        child
            .kill_single(Signal::Kill)
            .expect("no signal to a reaped pid");
    }

    /// The helper has successfully reaped the child but has not published the
    /// shared cache yet. Numeric-PID signaling returned `ESRCH` in this exact
    /// interval (and could target a recycled PID); the retained pidfd instead
    /// identifies only the original process and treats its termination as
    /// success.
    #[test]
    fn kill_single_is_safe_between_helper_reap_and_cache_publication() {
        use std::sync::mpsc;

        let spawner = AsyncLinuxSpawner::new().expect("spawner");
        let (mut child, numeric_calls) = observed_child(
            Arc::clone(&spawner.reactor),
            &Command::new("/bin/true", "/"),
        );
        let (reaped_tx, reaped) = mpsc::sync_channel(1);
        let (publish, publish_rx) = mpsc::sync_channel(1);
        let (published_tx, published) = mpsc::sync_channel(1);
        let hook_guard = wait_job_test_hook::install(
            child.id() as libc::pid_t,
            reaped_tx,
            publish.clone(),
            publish_rx,
            published_tx,
        );
        {
            let waker = std::task::Waker::from(Arc::new(CountingWaker(AtomicUsize::new(0))));
            let mut cx = Context::from_waker(&waker);
            let mut abandoned = child.wait_job();
            assert!(abandoned.as_mut().poll(&mut cx).is_pending());
        }

        reaped
            .recv_timeout(Duration::from_secs(5))
            .expect("helper did not reap child");
        let (signal_result, signal_state) =
            syscall_test_hook::with(Default::default(), || child.kill_single(Signal::Kill));
        assert_eq!(signal_state.signal_fds.len(), 1);
        assert_eq!(
            signal_state.numeric_signal_calls, 0,
            "exposed child must never fall back to numeric signaling"
        );
        assert_eq!(
            numeric_calls.load(Ordering::SeqCst),
            0,
            "the actual wrapped-child boundary must remain unused"
        );
        hook_guard.release();
        assert!(
            published
                .recv_timeout(Duration::from_secs(5))
                .expect("helper did not report publication"),
            "helper timed out before explicit publication release"
        );
        signal_result.expect("terminated process behind stable pidfd is success");
        assert_eq!(
            child.try_wait().expect("helper published cached status"),
            Some(ExitStatus::Code(0))
        );
    }

    /// Waits until `counter` is non-zero, up to 5 s.
    fn await_wake(counter: &CountingWaker, what: &str) {
        let deadline = std::time::Instant::now() + Duration::from_secs(5);
        while counter.0.load(Ordering::SeqCst) == 0 {
            assert!(std::time::Instant::now() < deadline, "{what}");
            std::thread::sleep(Duration::from_millis(5));
        }
    }

    /// Design review 4: `WaitJob` and `PidfdReady` kept the first poll's
    /// waker. After a second poll with another waker, that one is woken.
    #[test]
    fn wait_job_and_pidfd_ready_wake_the_latest_poller() {
        for use_pidfd in [false, true] {
            let child = std::process::Command::new("/bin/sleep")
                .arg("0.2")
                .spawn()
                .expect("spawn child");
            let pid = child.id() as libc::pid_t;
            let a = Arc::new(CountingWaker(AtomicUsize::new(0)));
            let b = Arc::new(CountingWaker(AtomicUsize::new(0)));
            let wa = std::task::Waker::from(Arc::clone(&a));
            let wb = std::task::Waker::from(Arc::clone(&b));
            let reactor = EpollReactor::new().expect("reactor");
            let mut fut: Pin<Box<dyn Future<Output = Result<()>>>> = if use_pidfd {
                let fd = sys::pidfd::open(pid).expect("pidfd");
                Box::pin(PidfdReady::new(Arc::clone(&reactor), fd))
            } else {
                let reaped = Arc::new(Mutex::new(None));
                let job = WaitJob::new(pid, reaped);
                Box::pin(async move { job.await.map(|_| ()) })
            };
            assert!(fut
                .as_mut()
                .poll(&mut Context::from_waker(&wa))
                .is_pending());
            assert!(fut
                .as_mut()
                .poll(&mut Context::from_waker(&wb))
                .is_pending());
            await_wake(&b, "the latest poller was never woken");
            assert_eq!(a.0.load(Ordering::SeqCst), 0, "pidfd={use_pidfd}");
            assert!(fut.as_mut().poll(&mut Context::from_waker(&wb)).is_ready());
            drop(child); // reaped by WaitJob, or zombie until process exit
        }
    }
}
