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
use std::sync::{Arc, Mutex};
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

        let reactor = Arc::new(Self {
            epoll_fd,
            registry: Mutex::new(HashMap::new()),
            shutdown: ShutdownSignal::new(),
            handle: Mutex::new(None),
        });

        let worker = Arc::clone(&reactor);
        let handle = std::thread::Builder::new()
            .name("rustils-async-epoll-reactor".to_owned())
            .spawn(move || worker.run())
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

    fn run(self: Arc<Self>) {
        let mut events = [libc::epoll_event { events: 0, u64: 0 }; MAX_EVENTS];
        loop {
            if self.shutdown.is_triggered() {
                return;
            }
            // SAFETY: `self.epoll_fd` is valid; `events` is a live,
            // correctly sized buffer the kernel writes up to
            // `MAX_EVENTS` entries into, matching the length passed.
            let n = unsafe {
                libc::epoll_wait(
                    self.epoll_fd.as_raw_fd(),
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
            let mut registry = self.registry.lock().unwrap_or_else(|p| p.into_inner());
            for event in &events[..n as usize] {
                let fd = event.u64 as RawFd;
                if let Some((ready, waker)) = registry.remove(&fd) {
                    ready.store(true, Ordering::Release);
                    drop(registry);
                    waker.wake();
                    registry = self.registry.lock().unwrap_or_else(|p| p.into_inner());
                }
            }
        }
    }
}

impl Drop for EpollReactor {
    fn drop(&mut self) {
        self.shutdown.trigger();
        if let Some(handle) = self.handle.lock().unwrap_or_else(|p| p.into_inner()).take() {
            let _ = handle.join();
        }
    }
}
