//! The macOS/BSD backend: `kevent` plus a software `EVFILT_USER` event
//! (rather than a separate wake fd like Linux's `eventfd` -- kqueue has
//! a built-in way to do this without one) to wake it early for
//! registration/shutdown. `kqueue`/`kevent`/`EVFILT_*` don't differ
//! across macOS, FreeBSD, OpenBSD, NetBSD, or DragonFly -- this file has
//! no OS-specific branches at all, gated in as a block by
//! `reactor/mod.rs` for exactly that reason (see #116).
//!
//! Untested on real hardware as of this writing -- this sandbox is
//! Linux-only. Verification varies by target, mirroring `platform-bsd`'s
//! own per-target honesty (see that crate's docs): `cargo check --target
//! x86_64-apple-darwin`/`x86_64-unknown-freebsd`/`x86_64-unknown-netbsd`
//! (real target-specific `libc` bindings, real type-checking) all pass,
//! but none of the five has ever actually linked or run this file --
//! OpenBSD and DragonFly can't even be cross-compile-checked from here
//! (no prebuilt `std` for either target). Treat every target as
//! reviewed-but-unverified until someone runs the test suite on real
//! hardware -- unlike `socket/mod.rs`'s macOS/BSD half, which now builds
//! on rustils' `platform-bsd` and inherits that crate's own real
//! `macos-latest`/FreeBSD-VM/OpenBSD-VM CI (see rustils#48/#52/#53/#86);
//! this reactor is this crate's own code with no such upstream coverage.

use super::{InitialReadiness, Interest, ScheduledIo};
use std::collections::HashMap;
use std::io;
use std::mem;
use std::os::fd::RawFd;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{Arc, Mutex};

/// Arbitrary, fixed identity for the one `EVFILT_USER` event this
/// reactor registers for waking itself -- never collides with a real fd
/// (`ident` for `EVFILT_READ`/`EVFILT_WRITE` is always a fd, and this
/// reactor only ever registers one `EVFILT_USER` event, so there's
/// nothing else it could be confused with).
const WAKE_IDENT: usize = 0;

fn empty_kevent() -> libc::kevent {
    // SAFETY: an all-zero `kevent` is a valid (if inert) value for this
    // plain-old-data type; every field actually used is set explicitly
    // below before the struct is passed to the kernel.
    unsafe { mem::zeroed() }
}

/// `libc::kevent`'s `filter`/`flags` fields are `i16`/`u16` on
/// macOS/FreeBSD/OpenBSD/DragonFly but `u32`/`u32` on NetBSD -- a real
/// struct-layout divergence `cargo check --target x86_64-unknown-netbsd`
/// caught (see this file's own docs), not something inferred from the
/// other four. `filter`/`flags` are taken here as `i64` (losslessly wide
/// enough for every one of those representations) and cast with `as _`
/// into whichever the live target's field actually is, rather than
/// hard-coding one platform's widths.
fn change(ident: usize, filter: i64, flags: i64, fflags: u32) -> libc::kevent {
    let mut ev = empty_kevent();
    ev.ident = ident;
    ev.filter = filter as _;
    ev.flags = flags as _;
    ev.fflags = fflags as _;
    ev
}

pub(crate) struct Reactor {
    kq_fd: RawFd,
    registry: Mutex<HashMap<RawFd, Arc<ScheduledIo>>>,
    shutdown: AtomicBool,
    thread: Mutex<Option<std::thread::JoinHandle<()>>>,
}

impl Reactor {
    pub(crate) fn new() -> io::Result<Reactor> {
        // SAFETY: no arguments reference memory.
        let kq_fd = unsafe { libc::kqueue() };
        if kq_fd < 0 {
            return Err(io::Error::last_os_error());
        }

        // `EV_CLEAR`: the event auto-resets after being reported, so it
        // behaves like a one-shot pulse per `wake()` call rather than
        // staying "ready" forever after the first trigger.
        let wake_ev = change(
            WAKE_IDENT,
            libc::EVFILT_USER as i64,
            (libc::EV_ADD | libc::EV_CLEAR) as i64,
            0,
        );
        // SAFETY: `kq_fd` is valid and freshly created; `&wake_ev` is
        // a valid single-element changelist outliving the call; no
        // output eventlist is requested (`nevents: 0`).
        let r = unsafe {
            libc::kevent(
                kq_fd,
                &wake_ev,
                1,
                std::ptr::null_mut(),
                0,
                std::ptr::null(),
            )
        };
        if r < 0 {
            let err = io::Error::last_os_error();
            // SAFETY: `kq_fd` is a valid fd we just created.
            unsafe { libc::close(kq_fd) };
            return Err(err);
        }

        Ok(Reactor {
            kq_fd,
            registry: Mutex::new(HashMap::new()),
            shutdown: AtomicBool::new(false),
            thread: Mutex::new(None),
        })
    }

    /// Spawns the background kqueue thread. Split from `new` because the
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
        let mut events = vec![empty_kevent(); 256];
        loop {
            if self.shutdown.load(Ordering::Acquire) {
                return;
            }
            // SAFETY: `events` is a valid, exclusively-borrowed buffer
            // of at least `events.len()` `kevent`s; `kq_fd` is valid for
            // the reactor's whole lifetime; a null `timeout` blocks
            // indefinitely, the same as epoll's `-1`.
            let n = unsafe {
                libc::kevent(
                    self.kq_fd,
                    std::ptr::null(),
                    0,
                    events.as_mut_ptr(),
                    events.len() as _,
                    std::ptr::null(),
                )
            };
            if n < 0 {
                let err = io::Error::last_os_error();
                if err.kind() == io::ErrorKind::Interrupted {
                    continue;
                }
                // Nothing sane to do with a fatal kevent error; exit the
                // thread rather than spin.
                return;
            }
            for ev in &events[..n as usize] {
                if ev.filter == libc::EVFILT_USER && ev.ident == WAKE_IDENT {
                    // Just a nudge to re-check `shutdown` above; nothing
                    // to drain (`EV_CLEAR` already reset it).
                    continue;
                }
                let fd = ev.ident as RawFd;
                let io = self.registry.lock().unwrap().get(&fd).cloned();
                let Some(io) = io else { continue };
                // EOF/error can arrive on either filter but means both
                // directions should be woken -- the same reasoning
                // epoll's EPOLLHUP/EPOLLERR handling uses.
                let eof_or_err = ev.flags & (libc::EV_EOF | libc::EV_ERROR) != 0;
                if ev.filter == libc::EVFILT_READ || eof_or_err {
                    io.mark_ready(Interest::Read);
                }
                if ev.filter == libc::EVFILT_WRITE || eof_or_err {
                    io.mark_ready(Interest::Write);
                }
            }
        }
    }

    fn wake(&self) {
        let ev = change(
            WAKE_IDENT,
            libc::EVFILT_USER as i64,
            libc::EV_ADD as i64,
            libc::NOTE_TRIGGER,
        );
        // SAFETY: `kq_fd` is valid; `&ev` is a valid single-element
        // changelist outliving the call; no output eventlist requested.
        unsafe {
            libc::kevent(
                self.kq_fd,
                &ev,
                1,
                std::ptr::null_mut(),
                0,
                std::ptr::null(),
            );
        }
    }

    pub(crate) fn register(&self, fd: RawFd) -> io::Result<Arc<ScheduledIo>> {
        self.register_with(fd, InitialReadiness::Optimistic)
    }

    /// [`register`](Self::register) with an explicit starting
    /// readiness -- see [`InitialReadiness`] for the one case (a
    /// connect still in flight) where the optimistic default is wrong.
    pub(crate) fn register_with(
        &self,
        fd: RawFd,
        initial: InitialReadiness,
    ) -> io::Result<Arc<ScheduledIo>> {
        let io = Arc::new(ScheduledIo::with_initial(initial));
        let changes = [
            change(
                fd as usize,
                libc::EVFILT_READ as i64,
                libc::EV_ADD as i64,
                0,
            ),
            change(
                fd as usize,
                libc::EVFILT_WRITE as i64,
                libc::EV_ADD as i64,
                0,
            ),
        ];
        // SAFETY: `kq_fd` is valid; `changes` is a valid 2-element
        // changelist outliving the call; `fd` is a valid, open fd owned
        // by the caller. `nevents: 0` (no output eventlist) is
        // deliberate, not an oversight -- see this struct's own doc
        // comment on why a real eventlist here is a real bug, not just
        // unnecessary: with a non-null timeout of `null` (block
        // indefinitely) *and* a non-zero `nevents`, `kevent()`'s
        // documented behavior is to apply the changelist and then wait
        // for up to `nevents` events to report -- across the *entire*
        // kqueue, not just this fd's own change -- which on a freshly
        // registered fd with nothing else immediately ready blocks this
        // call forever. `Reactor::new`'s own `EVFILT_USER` registration
        // already gets this right (`nevents: 0`); this call and
        // `deregister`'s used to differ from it for no reason that ever
        // needed the output (neither read it).
        // Registry entry first, kernel registration second -- same
        // reasoning as `epoll.rs`'s `register_with`: `EV_ADD` reports the
        // fd's current state right away and `EV_CLEAR` reports each
        // edge once, so an event dequeued before the entry exists is
        // dropped by `event_loop`'s lookup and never re-reported. For a
        // `WritePending` registration (an in-flight connect) that lost
        // first edge would be the one that completes `connect`.
        self.registry.lock().unwrap().insert(fd, io.clone());
        let r = unsafe {
            libc::kevent(
                self.kq_fd,
                changes.as_ptr(),
                changes.len() as _,
                std::ptr::null_mut(),
                0,
                std::ptr::null(),
            )
        };
        if r < 0 {
            self.registry.lock().unwrap().remove(&fd);
            return Err(io::Error::last_os_error());
        }
        Ok(io)
    }

    pub(crate) fn deregister(&self, fd: RawFd) {
        self.registry.lock().unwrap().remove(&fd);
        let changes = [
            change(
                fd as usize,
                libc::EVFILT_READ as i64,
                libc::EV_DELETE as i64,
                0,
            ),
            change(
                fd as usize,
                libc::EVFILT_WRITE as i64,
                libc::EV_DELETE as i64,
                0,
            ),
        ];
        // SAFETY: see `register` -- same call shape, same reasoning for
        // `nevents: 0`. A per-change error here (e.g. the kernel already
        // dropped this filter because the fd itself was closed) was
        // never read even when this call did request an eventlist, and
        // deregistering an already-gone filter is meant to be a
        // harmless no-op regardless, the same as epoll's
        // `EPOLL_CTL_DEL` on a closed fd.
        unsafe {
            libc::kevent(
                self.kq_fd,
                changes.as_ptr(),
                changes.len() as _,
                std::ptr::null_mut(),
                0,
                std::ptr::null(),
            );
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
        // SAFETY: `kq_fd` is owned exclusively by this `Reactor` and
        // still open at this point.
        unsafe {
            libc::close(self.kq_fd);
        }
    }
}
