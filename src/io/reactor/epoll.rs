//! The Linux backend: `epoll_wait` plus an `eventfd` to wake it early
//! for registration/shutdown.

use super::{Interest, ScheduledIo};
use std::collections::HashMap;
use std::io;
use std::os::fd::RawFd;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{Arc, Mutex};

pub(crate) struct Reactor {
    epoll_fd: RawFd,
    wake_fd: RawFd,
    registry: Mutex<HashMap<RawFd, Arc<ScheduledIo>>>,
    shutdown: AtomicBool,
    thread: Mutex<Option<std::thread::JoinHandle<()>>>,
}

impl Reactor {
    pub(crate) fn new() -> io::Result<Reactor> {
        // SAFETY: no arguments reference memory.
        let epoll_fd = unsafe { libc::epoll_create1(libc::EPOLL_CLOEXEC) };
        if epoll_fd < 0 {
            return Err(io::Error::last_os_error());
        }
        // SAFETY: no arguments reference memory. Non-blocking so a
        // drain-read from the epoll thread never itself blocks.
        let wake_fd = unsafe { libc::eventfd(0, libc::EFD_NONBLOCK | libc::EFD_CLOEXEC) };
        if wake_fd < 0 {
            let err = io::Error::last_os_error();
            // SAFETY: `epoll_fd` is a valid fd we just created.
            unsafe { libc::close(epoll_fd) };
            return Err(err);
        }

        let mut wake_ev = libc::epoll_event {
            events: libc::EPOLLIN as u32,
            u64: wake_fd as u64,
        };
        // SAFETY: `epoll_fd`/`wake_fd` are both valid, freshly created
        // fds; `&mut wake_ev` outlives the call.
        let r = unsafe { libc::epoll_ctl(epoll_fd, libc::EPOLL_CTL_ADD, wake_fd, &mut wake_ev) };
        if r < 0 {
            let err = io::Error::last_os_error();
            // SAFETY: both fds are valid, owned by us, not yet shared.
            unsafe {
                libc::close(wake_fd);
                libc::close(epoll_fd);
            }
            return Err(err);
        }

        let reactor = Reactor {
            epoll_fd,
            wake_fd,
            registry: Mutex::new(HashMap::new()),
            shutdown: AtomicBool::new(false),
            thread: Mutex::new(None),
        };
        Ok(reactor)
    }

    /// Spawns the background epoll thread. Split from `new` because the
    /// thread closure needs an `Arc<Reactor>`, which doesn't exist until
    /// after construction.
    pub(crate) fn start(self: &Arc<Self>) {
        let reactor = self.clone();
        let handle = std::thread::Builder::new()
            .name("rusty_tokio-reactor".to_string())
            .spawn(move || reactor.event_loop())
            .expect("failed to spawn rusty_tokio reactor thread");
        *self.thread.lock().unwrap() = Some(handle);
    }

    fn event_loop(&self) {
        let mut events = vec![libc::epoll_event { events: 0, u64: 0 }; 256];
        loop {
            if self.shutdown.load(Ordering::Acquire) {
                return;
            }
            // SAFETY: `events` is a valid, exclusively-borrowed buffer
            // of at least `events.len()` `epoll_event`s; `epoll_fd` is
            // valid for the reactor's whole lifetime.
            let n = unsafe {
                libc::epoll_wait(self.epoll_fd, events.as_mut_ptr(), events.len() as i32, -1)
            };
            if n < 0 {
                let err = io::Error::last_os_error();
                if err.kind() == io::ErrorKind::Interrupted {
                    continue;
                }
                // Nothing sane to do with a fatal epoll_wait error;
                // exit the thread rather than spin.
                return;
            }
            for ev in &events[..n as usize] {
                let fd = ev.u64 as RawFd;
                if fd == self.wake_fd {
                    self.drain_wake_fd();
                    continue;
                }
                let io = self.registry.lock().unwrap().get(&fd).cloned();
                let Some(io) = io else { continue };
                let flags = ev.events;
                if flags
                    & (libc::EPOLLIN as u32
                        | libc::EPOLLHUP as u32
                        | libc::EPOLLERR as u32
                        | libc::EPOLLRDHUP as u32)
                    != 0
                {
                    io.mark_ready(Interest::Read);
                }
                if flags & (libc::EPOLLOUT as u32 | libc::EPOLLHUP as u32 | libc::EPOLLERR as u32)
                    != 0
                {
                    io.mark_ready(Interest::Write);
                }
            }
        }
    }

    fn drain_wake_fd(&self) {
        let mut buf = [0u8; 8];
        // SAFETY: `buf` is a valid 8-byte buffer; `wake_fd` is a valid,
        // non-blocking eventfd, so this never blocks even with nothing
        // to drain.
        unsafe {
            libc::read(self.wake_fd, buf.as_mut_ptr().cast(), buf.len());
        }
    }

    fn wake(&self) {
        let one: u64 = 1;
        // SAFETY: `&one` is a valid 8-byte buffer; `wake_fd` is a valid
        // eventfd.
        unsafe {
            libc::write(self.wake_fd, (&one as *const u64).cast(), 8);
        }
    }

    pub(crate) fn register(&self, fd: RawFd) -> io::Result<Arc<ScheduledIo>> {
        let io = Arc::new(ScheduledIo::new());
        // `EPOLLET`: edge-triggered, matching `kqueue.rs`'s own
        // `EV_CLEAR` (see that backend's registration for the identical
        // reasoning) and the level this crate's own retry-until-
        // `WouldBlock` design (`poll_io`'s loop, `ScheduledIo::clear`)
        // already assumes. Without it, `epoll_wait` is level-triggered
        // by default: a socket that's idle-but-writable (true for
        // almost any connected socket almost all the time -- nothing
        // about "no one is currently writing" makes the send buffer
        // stop being reported ready) keeps reporting `EPOLLOUT` on
        // *every* call, so the reactor thread's `epoll_wait(..., -1)`
        // returns immediately forever instead of actually blocking --
        // pegging a full CPU core in a tight spin for as long as any
        // fd sits registered, not just this fd, since it shares one
        // `epoll_wait` call with everything else registered on this
        // reactor. Caught via `strace -c`: ~864k `epoll_wait` calls in
        // a 12s wait for one slow HTTP response that should have needed
        // exactly one (blocking) call.
        let mut ev = libc::epoll_event {
            events: (libc::EPOLLIN | libc::EPOLLOUT | libc::EPOLLRDHUP | libc::EPOLLET) as u32,
            u64: fd as u64,
        };
        // SAFETY: `epoll_fd` is valid for the reactor's lifetime; `fd`
        // is a valid, open fd owned by the caller; `&mut ev` outlives
        // the call.
        let r = unsafe { libc::epoll_ctl(self.epoll_fd, libc::EPOLL_CTL_ADD, fd, &mut ev) };
        if r < 0 {
            return Err(io::Error::last_os_error());
        }
        self.registry.lock().unwrap().insert(fd, io.clone());
        Ok(io)
    }

    pub(crate) fn deregister(&self, fd: RawFd) {
        self.registry.lock().unwrap().remove(&fd);
        // SAFETY: `epoll_fd` is valid; `fd` was previously registered
        // (or this is a harmless no-op if it wasn't). The kernel ignores
        // the ignored `event` pointer for `EPOLL_CTL_DEL`, but older
        // kernels (pre-2.6.9) require a non-null pointer, so we pass one
        // anyway for portability.
        let mut dummy = libc::epoll_event { events: 0, u64: 0 };
        unsafe {
            libc::epoll_ctl(self.epoll_fd, libc::EPOLL_CTL_DEL, fd, &mut dummy);
        }
    }

    pub(crate) fn shutdown(&self) {
        self.shutdown.store(true, Ordering::Release);
        self.wake();
        if let Some(handle) = self.thread.lock().unwrap().take() {
            let _ = handle.join();
        }
    }
}

impl Drop for Reactor {
    fn drop(&mut self) {
        // SAFETY: both fds are owned exclusively by this `Reactor` and
        // still open at this point.
        unsafe {
            libc::close(self.wake_fd);
            libc::close(self.epoll_fd);
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::time::{Duration, Instant};

    /// Regression test for the busy-spin `register`'s own doc comment
    /// describes: without `EPOLLET`, a registered fd that's idle but
    /// writable (true of almost any connected socket almost all the
    /// time, since "no one is currently writing" doesn't make the send
    /// buffer stop being reported ready) makes `epoll_wait` return
    /// immediately on *every* call instead of actually blocking --
    /// found via `strace -c` showing ~864k `epoll_wait` calls for a 12s
    /// wait that should have needed exactly one.
    ///
    /// Drives `epoll_wait` directly against the reactor's own
    /// `epoll_fd` (bypassing the background thread, so this test
    /// controls its own timing) for a bounded window, counting calls
    /// that return with events pending rather than timing out. A
    /// correctly edge-triggered registration reports the fd's initial
    /// writability once (maybe twice, allowing for scheduling jitter)
    /// and then blocks for the rest of the window; the level-triggered
    /// regression reports it on essentially every call, since nothing
    /// about the socket ever changes.
    #[test]
    fn idle_writable_socket_does_not_busy_spin_epoll_wait() {
        let reactor = Reactor::new().unwrap();

        // A connected pair needs no networking/ports, and both ends are
        // immediately writable and otherwise idle -- exactly the
        // "idle-but-writable" condition that triggered the spin.
        let mut fds = [0i32; 2];
        // SAFETY: `fds` is a valid 2-element buffer for `socketpair` to
        // write both new fds into.
        let r = unsafe { libc::socketpair(libc::AF_UNIX, libc::SOCK_STREAM, 0, fds.as_mut_ptr()) };
        assert_eq!(r, 0, "socketpair failed: {}", io::Error::last_os_error());
        let [a, b] = fds;

        reactor.register(a).unwrap();

        let mut events = vec![libc::epoll_event { events: 0, u64: 0 }; 16];
        let deadline = Instant::now() + Duration::from_millis(500);
        let mut ready_calls = 0;
        loop {
            let remaining = deadline.saturating_duration_since(Instant::now());
            if remaining.is_zero() {
                break;
            }
            // SAFETY: `events` is a valid, exclusively-borrowed buffer
            // of at least `events.len()` `epoll_event`s; `epoll_fd` is
            // valid for `reactor`'s whole lifetime, which outlives this
            // call.
            let n = unsafe {
                libc::epoll_wait(
                    reactor.epoll_fd,
                    events.as_mut_ptr(),
                    events.len() as i32,
                    remaining.as_millis().min(i32::MAX as u128) as i32,
                )
            };
            assert!(n >= 0, "epoll_wait failed: {}", io::Error::last_os_error());
            if n > 0 {
                ready_calls += 1;
            }
        }

        // SAFETY: both fds are owned by this test and still open.
        unsafe {
            libc::close(a);
            libc::close(b);
        }

        assert!(
            ready_calls <= 5,
            "epoll_wait returned with events {ready_calls} times in 500ms for an idle, \
             nothing-changed fd -- expected edge-triggered semantics to report readiness \
             once and then block, not busy-spin (this is the regression this test guards \
             against)"
        );
    }
}
