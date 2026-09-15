#![allow(unsafe_code)] // the one purpose of this module

//! An explicit, disclosed epoll reactor (`RM-ASYNC-RUNTIME-0001`: "MUST
//! NOT require one global executor or create a hidden runtime" —
//! satisfied here by making the reactor an ordinary, constructed,
//! `Drop`-cleaned-up value with its own background thread, not a
//! process-wide implicit singleton). One [`EpollReactor`] is owned by
//! one [`crate::AsyncLinuxSpawner`]; nothing here assumes a particular
//! async executor is running the futures that register with it.
//!
//! The background thread is the disclosed cost of not depending on an
//! external reactor crate (`mio`, `tokio`) or building this crate atop
//! a specific async runtime (`RM-DEV-ASYNC-0002`: "Blocking adapters
//! disclose their threads, queues, saturation, and shutdown behavior"
//! — this doc comment, [`EpollReactor::new`]'s signature, and
//! [`EpollReactor`]'s `Drop` impl are that disclosure). It wakes on a
//! bounded tick so it can observe shutdown promptly without needing a
//! self-pipe/eventfd wakeup mechanism — a reasonable simplification for
//! a first async increment, not a claim of sub-millisecond shutdown
//! latency.

use std::collections::HashMap;
use std::os::fd::{AsRawFd, FromRawFd, OwnedFd, RawFd};
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{Arc, Mutex, Weak};
use std::task::Waker;
use std::thread::JoinHandle;

use platform::error::{ErrorKind, OsCode, PlatformError, Result};
use reactor_core::ShutdownSignal;

/// How often the reactor thread's `epoll_wait` times out to check for
/// shutdown. Bounds shutdown latency; does not affect readiness
/// latency (a ready fd wakes `epoll_wait` immediately, regardless of
/// this value).
const REACTOR_TICK_MS: libc::c_int = 200;
const MAX_EVENTS: usize = 64;

/// An epoll instance plus the background thread driving it, and the
/// registry mapping a pending fd to the [`Waker`] to call once it
/// becomes readable.
pub struct EpollReactor {
    epoll_fd: OwnedFd,
    registry: Mutex<HashMap<RawFd, (Arc<AtomicBool>, Waker)>>,
    shutdown: ShutdownSignal,
    handle: Mutex<Option<JoinHandle<()>>>,
}

impl EpollReactor {
    /// Construct a new reactor and start its background thread. Callers
    /// own the returned `Arc` and clone it into every future that needs
    /// to register an fd; the reactor stops and its thread is joined
    /// when the last `Arc` is dropped (see the `Drop` impl).
    pub fn new() -> Result<Arc<Self>> {
        // SAFETY: `epoll_create1` takes one flags argument and returns
        // a fresh fd or -1; no pointer arguments.
        let raw = unsafe { libc::epoll_create1(libc::EPOLL_CLOEXEC) };
        if raw < 0 {
            let code = std::io::Error::last_os_error().raw_os_error().unwrap_or(0);
            return Err(PlatformError::new(
                ErrorKind::Other,
                OsCode::Errno(code),
                "epoll_create1",
            ));
        }
        // SAFETY: `raw` is a freshly returned, valid, otherwise-unowned
        // descriptor from the call above; wrapped exactly once.
        let epoll_fd = unsafe { OwnedFd::from_raw_fd(raw) };
        let epoll_fd_raw = epoll_fd.as_raw_fd();

        let reactor = Arc::new(Self {
            epoll_fd,
            registry: Mutex::new(HashMap::new()),
            shutdown: ShutdownSignal::new(),
            handle: Mutex::new(None),
        });

        // The background thread gets a `Weak`, never an owned `Arc`:
        // an owned `Arc` held for the thread's whole life (the old
        // bug here — monorepo review finding 27) makes
        // `Arc::strong_count` structurally unable to reach zero while
        // the thread runs, so `Drop` (the only thing that stops the
        // thread) could never fire. `shutdown` is cloned out
        // independently — it is the thread's actual, low-latency exit
        // signal, checked before every `epoll_wait`; `weak` is only
        // upgraded afterward, briefly, to reach the registry. `epoll_fd`
        // is likewise passed in as a raw descriptor rather than through
        // `self`, since it must stay usable for `epoll_wait` regardless
        // of the `Arc`'s strong count (see `run`'s doc comment).
        let shutdown = reactor.shutdown.clone();
        let worker = Arc::downgrade(&reactor);
        let handle = std::thread::Builder::new()
            .name("rustils-async-epoll-reactor".to_owned())
            .spawn(move || Self::run(worker, epoll_fd_raw, shutdown))
            .map_err(|e| {
                PlatformError::new(
                    ErrorKind::Other,
                    OsCode::Errno(e.raw_os_error().unwrap_or(0)),
                    "spawn epoll reactor thread",
                )
            })?;
        *reactor.handle.lock().unwrap_or_else(|p| p.into_inner()) = Some(handle);

        Ok(reactor)
    }

    /// Register `fd` for one edge of readiness (`EPOLLONESHOT`). `ready`
    /// is set to `true` and `waker` is called exactly once, the next
    /// time `fd` becomes readable; the reactor forgets about the entry
    /// afterward — matching this crate's one-shot use (a pidfd is read
    /// exactly once, for exactly one termination event, and closed
    /// immediately after).
    ///
    /// The `ready` flag exists because the `Waker` alone is not a
    /// trustworthy signal on its own: a combinator like
    /// [`platform_async::process::wait_any`](../../platform_async/process/fn.wait_any.html)
    /// polls several registered futures through one shared waker, so a
    /// wake caused by *sibling* `A` becoming ready would otherwise look,
    /// from `B`'s perspective, indistinguishable from `B` itself having
    /// been reported ready by this reactor. Checking `ready` on every
    /// poll — not "was I polled again" — is what keeps each future
    /// honest about its own fd.
    pub fn register(&self, fd: RawFd, ready: Arc<AtomicBool>, waker: Waker) -> Result<()> {
        self.registry
            .lock()
            .unwrap_or_else(|p| p.into_inner())
            .insert(fd, (ready, waker));

        let mut event = libc::epoll_event {
            events: (libc::EPOLLIN | libc::EPOLLONESHOT) as u32,
            u64: fd as u64,
        };
        // SAFETY: `self.epoll_fd` is a valid, owned epoll descriptor;
        // `fd` is a valid descriptor supplied by the caller; `event` is
        // a live, correctly initialized local the kernel only reads
        // from for `EPOLL_CTL_ADD`.
        let rc = unsafe {
            libc::epoll_ctl(
                self.epoll_fd.as_raw_fd(),
                libc::EPOLL_CTL_ADD,
                fd,
                &mut event,
            )
        };
        if rc < 0 {
            let code = std::io::Error::last_os_error().raw_os_error().unwrap_or(0);
            self.registry
                .lock()
                .unwrap_or_else(|p| p.into_inner())
                .remove(&fd);
            return Err(PlatformError::new(
                ErrorKind::Other,
                OsCode::Errno(code),
                "epoll_ctl(EPOLL_CTL_ADD)",
            ));
        }
        Ok(())
    }

    /// Removes `fd`'s pending registration without waiting for the
    /// fire path in [`Self::run`] to observe it — used by a future
    /// that is being dropped before ever seeing readiness (e.g. the
    /// losing side of [`platform_async::process::wait_any`], or a
    /// timeout that elapses before every sibling finishes). Mirrors
    /// that fire path's own cleanup exactly: only the registry entry
    /// is removed, with no separate `EPOLL_CTL_DEL` — this reactor's
    /// `fd` owners close their descriptor immediately once done with
    /// it (see [`Self::register`]'s doc comment), and closing a
    /// descriptor already drops every epoll registration for it.
    ///
    /// Returns `true` if an entry was actually removed. `false` means
    /// the fire path already claimed this fd first — safe to ignore;
    /// the caller has nothing left to clean up.
    pub fn deregister(&self, fd: RawFd) -> bool {
        self.registry
            .lock()
            .unwrap_or_else(|p| p.into_inner())
            .remove(&fd)
            .is_some()
    }

    /// Test-only introspection: how many fds currently have a live
    /// registration. Lets regression tests assert the registry does
    /// not grow unboundedly when a multiplexed wait is abandoned.
    #[cfg(test)]
    pub(crate) fn registered_len(&self) -> usize {
        self.registry
            .lock()
            .unwrap_or_else(|p| p.into_inner())
            .len()
    }

    /// The background thread's loop body. Takes a [`Weak`] — never an
    /// owned `Arc` — for the reasons documented in [`Self::new`]:
    /// holding a strong reference for the thread's entire life would
    /// make it structurally impossible for `Arc::strong_count` to ever
    /// reach zero, so `Drop` (which triggers `shutdown` and joins this
    /// thread) could never run. `epoll_fd` and `shutdown` are passed
    /// in independently of `self` so the blocking `epoll_wait` call
    /// below never needs a successful upgrade: `epoll_fd` stays open
    /// until `Drop` has joined this thread (the field is only dropped
    /// after `Drop::drop` returns), and `shutdown` is this thread's
    /// actual, immediately observable stop signal. `weak` is upgraded
    /// only afterward, briefly, to reach the shared registry when
    /// there are events to deliver — and dropped again before the
    /// next `epoll_wait` call, so it is never held across a block.
    fn run(weak: Weak<Self>, epoll_fd: RawFd, shutdown: ShutdownSignal) {
        let mut events = [libc::epoll_event { events: 0, u64: 0 }; MAX_EVENTS];
        loop {
            if shutdown.is_triggered() {
                return;
            }
            // SAFETY: `epoll_fd` is valid for the reason documented
            // above; `events` is a live, correctly sized buffer the
            // kernel writes up to `MAX_EVENTS` entries into, matching
            // the length passed.
            let n = unsafe {
                libc::epoll_wait(
                    epoll_fd,
                    events.as_mut_ptr(),
                    MAX_EVENTS as libc::c_int,
                    REACTOR_TICK_MS,
                )
            };
            if n < 0 {
                let err = std::io::Error::last_os_error();
                if err.kind() == std::io::ErrorKind::Interrupted {
                    continue;
                }
                // The epoll fd itself is unusable (should not happen in
                // practice — this reactor owns it exclusively); nothing
                // further this thread can do but stop.
                return;
            }
            if n == 0 {
                // Plain tick, nothing ready: skip the upgrade entirely
                // so this thread holds no strong reference at all for
                // the common case, leaving `Arc::strong_count` free to
                // reach zero the moment every external `Arc` is gone.
                continue;
            }
            let Some(this) = weak.upgrade() else {
                // Every external `Arc` is already gone and `Drop` is
                // running (or has run) on some other thread; nothing
                // left to deliver these events to.
                return;
            };
            let mut registry = this.registry.lock().unwrap_or_else(|p| p.into_inner());
            for event in &events[..n as usize] {
                let fd = event.u64 as RawFd;
                if let Some((ready, waker)) = registry.remove(&fd) {
                    ready.store(true, Ordering::Release);
                    drop(registry);
                    waker.wake();
                    registry = this.registry.lock().unwrap_or_else(|p| p.into_inner());
                }
            }
        }
    }
}

impl Drop for EpollReactor {
    fn drop(&mut self) {
        self.shutdown.trigger();
        if let Some(handle) = self.handle.lock().unwrap_or_else(|p| p.into_inner()).take() {
            if handle.thread().id() == std::thread::current().id() {
                // `drop` is running on the reactor's own background
                // thread: the strong reference that just hit zero was
                // the transient one `run` upgrades to reach the
                // registry (see `run`'s doc comment) — this thread is
                // already unwinding out of `run` and about to return.
                // Joining here would be a self-join deadlock (a thread
                // cannot wait on its own completion), so just detach
                // the handle instead of blocking on it.
                drop(handle);
            } else {
                let _ = handle.join();
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// Regression test for monorepo review finding 27: `run` used to
    /// take `self: Arc<Self>` by value and hold that strong reference
    /// for its entire life. That made `Arc::strong_count` structurally
    /// unable to reach zero while the thread ran, so `Drop::drop` (the
    /// only thing that calls `shutdown.trigger()` and joins the
    /// thread) could never execute — every `EpollReactor` and its
    /// background thread leaked for the life of the process no matter
    /// how many external `Arc`s were dropped.
    #[test]
    fn dropping_every_external_arc_actually_tears_down_the_reactor() {
        for _ in 0..4 {
            let reactor = EpollReactor::new().expect("construct reactor");
            assert_eq!(
                Arc::strong_count(&reactor),
                1,
                "the background thread must not hold its own strong Arc \
                 reference to the reactor it belongs to"
            );

            let weak = Arc::downgrade(&reactor);
            drop(reactor);
            assert!(
                weak.upgrade().is_none(),
                "EpollReactor was not actually dropped once every external \
                 Arc went away -- its background thread must still be \
                 holding a strong reference, leaking both the reactor and \
                 its thread"
            );
        }
    }
}
