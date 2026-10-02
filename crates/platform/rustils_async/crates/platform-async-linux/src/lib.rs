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
        let signal_pidfd = match sys::pidfd::open(child.id() as libc::pid_t) {
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
    match child.kill_single(Signal::Kill) {
        Ok(()) => {}
        Err(error) if error.os == OsCode::Errno(libc::ESRCH) => {}
        Err(error) => return Err(error),
    }
    child.take_stdin();
    child.take_stdout();
    child.take_stderr();
    child.wait().map(|_| ())
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
        sys::pidfd::send_signal(self.signal_pidfd.as_fd(), sig)
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
            let result_slot = Arc::clone(&this.result);
            let reaped = Arc::clone(&this.reaped);
            let waker = this.waker.clone();
            let pid = this.pid;
            let spawned = std::thread::Builder::new()
                .name("rustils-async-waitjob".to_owned())
                .spawn(move || {
                    let outcome = platform_linux::sys::spawn::wait_job(pid);
                    #[cfg(test)]
                    let published = wait_job_test_hook::after_waitpid(pid);
                    if let Ok(status) = &outcome {
                        if !matches!(status, ExitStatus::Stopped(_) | ExitStatus::Continued) {
                            *reaped.lock().unwrap_or_else(|p| p.into_inner()) = Some(*status);
                        }
                    }
                    #[cfg(test)]
                    if let Some(published) = published {
                        published.wait();
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
    use std::sync::{Arc, Barrier, Mutex};

    struct Hook {
        pid: libc::pid_t,
        reaped: Arc<Barrier>,
        publish: Arc<Barrier>,
        published: Arc<Barrier>,
    }

    static HOOK: Mutex<Option<Hook>> = Mutex::new(None);

    pub(super) fn install(
        pid: libc::pid_t,
        reaped: Arc<Barrier>,
        publish: Arc<Barrier>,
        published: Arc<Barrier>,
    ) {
        *HOOK.lock().unwrap_or_else(|p| p.into_inner()) = Some(Hook {
            pid,
            reaped,
            publish,
            published,
        });
    }

    pub(super) fn after_waitpid(pid: libc::pid_t) -> Option<Arc<Barrier>> {
        let mut slot = HOOK.lock().unwrap_or_else(|p| p.into_inner());
        let hook = (slot.as_ref().is_some_and(|hook| hook.pid == pid))
            .then(|| slot.take().expect("matching hook exists"));
        drop(slot);
        if let Some(hook) = hook {
            hook.reaped.wait();
            hook.publish.wait();
            return Some(hook.published);
        }
        None
    }
}

#[cfg(test)]
mod wait_any_leak_tests {
    use super::*;
    use std::sync::atomic::AtomicUsize;
    use std::task::Wake;
    use std::time::Duration;

    struct CountingWaker(AtomicUsize);

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
        use std::sync::Barrier;

        let spawner = AsyncLinuxSpawner::new().expect("spawner");
        let mut child = spawner
            .spawn(&Command::new("/bin/true", "/"))
            .expect("spawn");
        let reaped = Arc::new(Barrier::new(2));
        let publish = Arc::new(Barrier::new(2));
        let published = Arc::new(Barrier::new(2));
        wait_job_test_hook::install(
            child.id() as libc::pid_t,
            Arc::clone(&reaped),
            Arc::clone(&publish),
            Arc::clone(&published),
        );
        {
            let waker = std::task::Waker::from(Arc::new(CountingWaker(AtomicUsize::new(0))));
            let mut cx = Context::from_waker(&waker);
            let mut abandoned = child.wait_job();
            assert!(abandoned.as_mut().poll(&mut cx).is_pending());
        }

        reaped.wait();
        let signal_result = child.kill_single(Signal::Kill);
        publish.wait();
        published.wait();
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
