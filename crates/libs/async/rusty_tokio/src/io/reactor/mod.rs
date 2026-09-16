//! The I/O reactor: one background thread blocked in the OS's readiness
//! syscall (`epoll_wait` on Linux, `kevent` on macOS/BSD), translating
//! readiness events into waker calls. Level-triggered, on purpose --
//! edge-triggered epoll/kqueue demands that every reader drain a fd
//! until it sees `EWOULDBLOCK` or risk missing events forever, which is
//! an easy invariant to get subtly wrong. Level-triggered costs one
//! extra syscall in the common case and is much harder to misuse.
//!
//! [`ScheduledIo`] (the per-fd readiness state), [`Interest`], and the
//! [`poll_io`]/[`ready_io`] helpers built on them are shared by every
//! backend -- only the actual OS readiness syscall and how fds get
//! registered with it differ, in `epoll.rs`/`kqueue.rs`/`io_uring.rs`.
//! All three expose the identical `Reactor::{new, start, register,
//! deregister, shutdown}` surface this module re-exports, so nothing
//! above this module (or in `tcp.rs`/`udp.rs`/`unix.rs`) needs its own
//! `#[cfg]` for which backend is live.
//!
//! A fourth combination exists on Linux: the `io-uring-reactor` feature
//! (off by default) swaps `epoll.rs` for `io_uring.rs` at compile time
//! -- see that module's docs for scope (readiness only, via
//! `IORING_OP_POLL_ADD`; the actual `read`/`write` syscalls are
//! unchanged) and why a broader io_uring integration isn't attempted.

#[cfg(all(target_os = "linux", not(feature = "io-uring-reactor")))]
mod epoll;
#[cfg(all(target_os = "linux", not(feature = "io-uring-reactor")))]
pub(crate) use epoll::Reactor;

#[cfg(all(target_os = "linux", feature = "io-uring-reactor"))]
mod io_uring;
#[cfg(all(target_os = "linux", feature = "io-uring-reactor"))]
pub(crate) use io_uring::Reactor;

#[cfg(any(
    target_os = "macos",
    target_os = "freebsd",
    target_os = "openbsd",
    target_os = "netbsd",
    target_os = "dragonfly"
))]
mod kqueue;
#[cfg(any(
    target_os = "macos",
    target_os = "freebsd",
    target_os = "openbsd",
    target_os = "netbsd",
    target_os = "dragonfly"
))]
pub(crate) use kqueue::Reactor;

#[cfg(target_os = "windows")]
mod windows;
#[cfg(target_os = "windows")]
pub(crate) use windows::Reactor;

use std::io;
use std::sync::atomic::{AtomicUsize, Ordering};
use std::sync::Mutex;
use std::task::{Context, Poll, Waker};

/// The raw platform I/O handle every backend's `register`/`deregister`
/// takes -- a plain fd on Unix, a `SOCKET` handle on Windows (IOCP has no
/// fd concept at all; sockets there are `HANDLE`-like values, not small
/// integers). Nothing above this module (`tcp.rs`/`udp.rs`) needs its
/// own `#[cfg]` for which one it is; see [`AsRawIo`] for how a concrete
/// socket type hands one over uniformly.
#[cfg(unix)]
pub(crate) type RawIo = std::os::fd::RawFd;
#[cfg(windows)]
pub(crate) type RawIo = std::os::windows::io::RawSocket;

/// The owning counterpart of [`RawIo`] -- an `OwnedFd` on Unix, an
/// `OwnedSocket` on Windows. Used for socket-creation return types and
/// `From<OwnedIo>` conversions into a concrete platform socket type,
/// mirroring `RawIo`'s role for borrowed access.
#[cfg(unix)]
pub(crate) type OwnedIo = std::os::fd::OwnedFd;
#[cfg(windows)]
pub(crate) type OwnedIo = std::os::windows::io::OwnedSocket;

/// Hands over a [`RawIo`] regardless of platform -- a thin, uniform
/// wrapper over `AsRawFd`/`AsRawSocket` so `tcp.rs`/`udp.rs` can call
/// `.as_raw_io()` once instead of branching on `#[cfg(unix)]`/
/// `#[cfg(windows)]` at every call site.
pub(crate) trait AsRawIo {
    fn as_raw_io(&self) -> RawIo;
}

#[cfg(unix)]
impl<T: std::os::fd::AsRawFd> AsRawIo for T {
    fn as_raw_io(&self) -> RawIo {
        self.as_raw_fd()
    }
}

#[cfg(windows)]
impl<T: std::os::windows::io::AsRawSocket> AsRawIo for T {
    fn as_raw_io(&self) -> RawIo {
        self.as_raw_socket()
    }
}

/// Duplicates the underlying handle into an owned [`OwnedIo`], regardless
/// of platform -- a thin, uniform wrapper over
/// `AsFd::try_clone_to_owned`/`AsSocket::try_clone_to_owned` so
/// `tcp.rs`/`udp.rs`'s `into_std` methods (which need an owned handle to
/// hand to `std`) don't need their own `#[cfg(unix)]`/`#[cfg(windows)]`.
pub(crate) trait TryCloneIo {
    fn try_clone_io(&self) -> io::Result<OwnedIo>;
}

#[cfg(unix)]
impl<T: std::os::fd::AsFd> TryCloneIo for T {
    fn try_clone_io(&self) -> io::Result<OwnedIo> {
        self.as_fd().try_clone_to_owned()
    }
}

#[cfg(windows)]
impl<T: std::os::windows::io::AsSocket> TryCloneIo for T {
    fn try_clone_io(&self) -> io::Result<OwnedIo> {
        self.as_socket().try_clone_to_owned()
    }
}

#[derive(Clone, Copy, PartialEq, Eq)]
pub(crate) enum Interest {
    Read,
    Write,
}

/// Per-registered-fd readiness state: one word each for readable and
/// writable, plus the waker to fire when readiness flips on.
///
/// Each word packs the ready bit ([`READY`]) with an edge counter in
/// the bits above it, bumped on every [`mark_ready`](Self::mark_ready).
/// The counter is what makes clearing readiness safe under the
/// edge-triggered backends (`EPOLLET`, `EV_CLEAR`, and the one-shot
/// AFD/io_uring polls, all re-armed only after an event is consumed):
/// a caller that saw "ready", tried its syscall, and got `WouldBlock`
/// clears the bit through [`clear_if_unchanged`](Self::clear_if_unchanged)
/// with the [`ReadyToken`] it took *before* the attempt, and that clear
/// only lands if no edge arrived in between. Without the counter, an
/// edge delivered between the failed attempt and the clear was wiped
/// -- and, being an edge, never re-reported -- so the next wait on that
/// direction hung until something else happened to poke the fd. That
/// was the mechanism behind rare stalls of the shape "write happened
/// after my `WouldBlock`, `readable()` never woke".
pub(crate) struct ScheduledIo {
    readable: AtomicUsize,
    writable: AtomicUsize,
    read_waker: Mutex<Option<Waker>>,
    write_waker: Mutex<Option<Waker>>,
}

/// The ready bit of a [`ScheduledIo`] direction word.
const READY: usize = 1;
/// One step of the edge counter packed above [`READY`].
const TICK: usize = 2;

/// A direction's readiness word as observed at one instant -- taken
/// with [`ScheduledIo::snapshot`] before attempting a syscall, and
/// handed back to [`ScheduledIo::clear_if_unchanged`] after a
/// `WouldBlock`, so the clear can be refused if an edge arrived in
/// between. Deliberately opaque: nothing outside this module inspects
/// it.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) struct ReadyToken(usize);

/// How a freshly registered fd's readiness bits start out -- the one
/// choice a backend's `register_with` takes from its caller.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) enum InitialReadiness {
    /// Both directions assumed ready until a `WouldBlock` proves
    /// otherwise -- right for almost every fd (see
    /// `ScheduledIo::with_initial`), and what plain `register` uses.
    Optimistic,
    /// Writable starts *cleared*: the first write-side wait blocks until
    /// the backend actually reports the fd writable (or failed). For a
    /// non-blocking `connect` that returned in-progress, this is the
    /// only correct start -- see [`InitialReadiness::for_connect`].
    WritePending,
}

impl InitialReadiness {
    /// The right initial state for a socket whose non-blocking
    /// `connect` just returned `outcome`.
    ///
    /// The optimistic default is *wrong* for a connect still in flight:
    /// `TcpStream::connect` waits for writability and then reads
    /// `SO_ERROR` to learn whether the connect succeeded, but with the
    /// writable bit pre-set that check runs immediately, while the
    /// handshake is still pending, sees no error yet, and hands back a
    /// "connected" stream that isn't -- whatever the connect later
    /// resolves to (including refused) is never observed by `connect`
    /// itself. Linux masked this: a loopback `connect(2)` reports
    /// `EINPROGRESS` but has already processed the handshake -- or the
    /// peer's RST -- inside the call, so by the time the optimistic
    /// check ran, `SO_ERROR` and writability were already settled; and
    /// for a remote peer the first `send` in `SYN_SENT` returns `EAGAIN`,
    /// so the reactor wait happened on the first write instead. On
    /// Windows a loopback connect is genuinely still pending when
    /// `connect` returns, and a refused one sat forever
    /// (Rusty-Mill/rusty_mill#137).
    ///
    /// Starting write-pending is safe even if the connect completes
    /// before or during registration: every backend reports an fd's
    /// *current* state at registration time (`EPOLL_CTL_ADD` with
    /// `EPOLLET`, `EV_ADD` with `EV_CLEAR`, `IORING_OP_POLL_ADD`, and an
    /// `IOCTL_AFD_POLL` on an already-writable socket all complete
    /// immediately), so the writable edge is never lost. Clearing the
    /// bit *after* registration would not be safe under those
    /// edge-triggered backends -- an edge delivered in between would be
    /// wiped and never re-reported -- which is why this is a
    /// registration-time choice rather than a `clear` call.
    pub(crate) fn for_connect(outcome: super::socket::ConnectOutcome) -> Self {
        match outcome {
            super::socket::ConnectOutcome::Established => InitialReadiness::Optimistic,
            super::socket::ConnectOutcome::InProgress => InitialReadiness::WritePending,
        }
    }
}

impl ScheduledIo {
    fn with_initial(initial: InitialReadiness) -> Self {
        ScheduledIo {
            // Optimistic by default: assume both directions are ready
            // until a WouldBlock proves otherwise. This matches every
            // real fd's actual state right after it's created (a
            // listener can usually be written to immediately; an
            // already-established socket is writable), and a wrong
            // guess just costs one wasted syscall attempt. The one fd
            // where it's *not* a safe guess -- a socket whose connect is
            // still in flight, where the wasted attempt is the `SO_ERROR`
            // check that decides whether `connect` succeeded -- asks for
            // `WritePending` instead; see `InitialReadiness::for_connect`.
            readable: AtomicUsize::new(READY),
            writable: AtomicUsize::new(match initial {
                InitialReadiness::Optimistic => READY,
                InitialReadiness::WritePending => 0,
            }),
            read_waker: Mutex::new(None),
            write_waker: Mutex::new(None),
        }
    }

    fn word(&self, interest: Interest) -> &AtomicUsize {
        match interest {
            Interest::Read => &self.readable,
            Interest::Write => &self.writable,
        }
    }

    fn poll_ready(&self, cx: &mut Context<'_>, interest: Interest) -> Poll<()> {
        let word = self.word(interest);
        let waker_slot = match interest {
            Interest::Read => &self.read_waker,
            Interest::Write => &self.write_waker,
        };
        if word.load(Ordering::Acquire) & READY != 0 {
            return Poll::Ready(());
        }
        *waker_slot.lock().unwrap() = Some(cx.waker().clone());
        // Re-check after registering the waker: the reactor thread may
        // have flipped the bit between our first load and taking the
        // lock above, and if we didn't check again that wakeup would be
        // lost (nothing left to observe the flag flip).
        if word.load(Ordering::Acquire) & READY != 0 {
            return Poll::Ready(());
        }
        Poll::Pending
    }

    /// The `interest` direction's readiness word right now. Take it
    /// *before* the syscall whose `WouldBlock` might lead to a clear.
    fn snapshot(&self, interest: Interest) -> ReadyToken {
        ReadyToken(self.word(interest).load(Ordering::Acquire))
    }

    /// Clears `interest` readiness -- but only if the direction's word
    /// still equals `token`, i.e. no [`mark_ready`](Self::mark_ready)
    /// has run since that snapshot was taken. Returns whether it
    /// cleared. A `false` means an edge arrived during the caller's
    /// syscall: the bit stays set so the caller's next `poll_ready`
    /// passes straight through and the syscall is retried, instead of
    /// the edge being lost.
    ///
    /// A single compare-and-swap, not a compare-then-store: with two
    /// steps the reactor could bump the counter and set the bit between
    /// them and the store would still wipe it.
    fn clear_if_unchanged(&self, interest: Interest, token: ReadyToken) -> bool {
        self.word(interest)
            .compare_exchange(
                token.0,
                token.0 & !READY,
                Ordering::AcqRel,
                Ordering::Acquire,
            )
            .is_ok()
    }

    /// Called by a backend's event loop when it observes `interest` is
    /// ready on this fd. Plain private visibility -- not `pub(super)` --
    /// is enough: `epoll`/`kqueue` are child modules of `reactor`, and
    /// Rust's default visibility already reaches every descendant of the
    /// defining module.
    fn mark_ready(&self, interest: Interest) {
        let waker_slot = match interest {
            Interest::Read => &self.read_waker,
            Interest::Write => &self.write_waker,
        };
        // Bump the edge counter and set the ready bit in one atomic
        // step, so a concurrent `clear_if_unchanged` holding an older
        // token either lands entirely before this (and is then
        // overridden by it) or fails its compare -- never interleaves.
        let _ = self
            .word(interest)
            .fetch_update(Ordering::AcqRel, Ordering::Acquire, |word| {
                Some(word.wrapping_add(TICK) | READY)
            });
        if let Some(waker) = waker_slot.lock().unwrap().take() {
            waker.wake();
        }
    }
}

/// Run `op` once `interest` readiness is available, in a `Poll`-based
/// shape rather than an `async fn` -- the primitive [`super::async_io`]'s
/// `poll_read`/`poll_write` need, since they can't `.await` anything
/// themselves. [`ready_io`] below is just this wrapped in `poll_fn` for
/// callers that can.
pub(crate) fn poll_io<T>(
    io: &std::sync::Arc<ScheduledIo>,
    interest: Interest,
    cx: &mut Context<'_>,
    mut op: impl FnMut() -> io::Result<T>,
) -> Poll<io::Result<T>> {
    // Coop budget check first, before even looking at readiness -- see
    // `crate::coop`'s module docs for why a socket that's already
    // readable still needs to yield once budget runs out, rather than
    // handing the read/write straight over.
    if crate::coop::poll_proceed(cx).is_pending() {
        return Poll::Pending;
    }
    loop {
        if io.poll_ready(cx, interest).is_pending() {
            return Poll::Pending;
        }
        // Snapshot before the attempt, so a `WouldBlock` clears only
        // the readiness that was already stale when we started -- an
        // edge that lands during `op` keeps the bit set and the loop
        // simply tries again (see `ScheduledIo`'s docs).
        let token = io.snapshot(interest);
        match op() {
            Ok(v) => return Poll::Ready(Ok(v)),
            Err(e) if e.kind() == io::ErrorKind::WouldBlock => {
                io.clear_if_unchanged(interest, token);
                continue;
            }
            Err(e) => return Poll::Ready(Err(e)),
        }
    }
}

/// Waits for `interest` readiness on `io`, without attempting any
/// operation itself -- the building block behind [`super::AsyncFd`]'s
/// `readable`/`writable` (Unix-only) and, cross-platform, the generic
/// `readable`/`writable`/`ready`/`try_io` methods on `TcpStream`/
/// `UdpSocket`/`UnixStream` (see `super::readiness`), which hand the
/// actual I/O back to the caller instead of performing it internally
/// the way [`poll_io`] does.
pub(crate) fn poll_ready(
    io: &std::sync::Arc<ScheduledIo>,
    interest: Interest,
    cx: &mut Context<'_>,
) -> Poll<()> {
    io.poll_ready(cx, interest)
}

/// `io`'s `interest` readiness word right now -- take it before a
/// syscall whose `WouldBlock` should clear readiness, and hand it to
/// [`clear_ready_if_unchanged`] afterwards. [`poll_io`]/[`ready_io`]
/// do this internally; [`super::AsyncFdReadyGuard`] and
/// [`super::readiness::try_io`] do it for callers running their own
/// I/O outside this module.
pub(crate) fn snapshot_ready(io: &std::sync::Arc<ScheduledIo>, interest: Interest) -> ReadyToken {
    io.snapshot(interest)
}

/// Clears `io`'s cached `interest` readiness after a `WouldBlock`
/// proved the "ready" signal was stale -- unless a fresh readiness edge
/// arrived since `token` was taken, in which case the bit is left set
/// (and `false` returned) so the next wait passes straight through
/// instead of losing that edge. See [`ScheduledIo`]'s docs.
pub(crate) fn clear_ready_if_unchanged(
    io: &std::sync::Arc<ScheduledIo>,
    interest: Interest,
    token: ReadyToken,
) -> bool {
    io.clear_if_unchanged(interest, token)
}

/// Run `op` in a loop, waiting for `interest` readiness on `io` between
/// attempts, until it succeeds or fails with something other than
/// `WouldBlock`.
pub(crate) async fn ready_io<T>(
    io: &std::sync::Arc<ScheduledIo>,
    interest: Interest,
    mut op: impl FnMut() -> io::Result<T>,
) -> io::Result<T> {
    std::future::poll_fn(|cx| poll_io(io, interest, cx, &mut op)).await
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::task::Waker;

    fn is_ready(io: &ScheduledIo, interest: Interest) -> bool {
        let mut cx = Context::from_waker(Waker::noop());
        io.poll_ready(&mut cx, interest).is_ready()
    }

    #[test]
    fn optimistic_starts_ready_both_ways_and_write_pending_does_not() {
        let io = ScheduledIo::with_initial(InitialReadiness::Optimistic);
        assert!(is_ready(&io, Interest::Read));
        assert!(is_ready(&io, Interest::Write));
        let io = ScheduledIo::with_initial(InitialReadiness::WritePending);
        assert!(is_ready(&io, Interest::Read));
        assert!(!is_ready(&io, Interest::Write));
    }

    #[test]
    fn clear_with_no_intervening_edge_clears_and_mark_ready_restores() {
        let io = ScheduledIo::with_initial(InitialReadiness::Optimistic);
        let token = io.snapshot(Interest::Read);
        assert!(io.clear_if_unchanged(Interest::Read, token));
        assert!(!is_ready(&io, Interest::Read));
        io.mark_ready(Interest::Read);
        assert!(is_ready(&io, Interest::Read));
    }

    /// The race this exists for: an edge delivered between the failed
    /// attempt (snapshot) and the clear must survive the clear.
    #[test]
    fn clear_after_an_intervening_edge_is_refused_and_readiness_survives() {
        let io = ScheduledIo::with_initial(InitialReadiness::Optimistic);
        let token = io.snapshot(Interest::Read);
        io.mark_ready(Interest::Read);
        assert!(!io.clear_if_unchanged(Interest::Read, token));
        assert!(is_ready(&io, Interest::Read));
        // A fresh snapshot taken after that edge clears normally.
        let token = io.snapshot(Interest::Read);
        assert!(io.clear_if_unchanged(Interest::Read, token));
        assert!(!is_ready(&io, Interest::Read));
    }

    #[test]
    fn a_stale_token_never_clears_even_after_many_edges() {
        let io = ScheduledIo::with_initial(InitialReadiness::Optimistic);
        let token = io.snapshot(Interest::Write);
        for _ in 0..1000 {
            io.mark_ready(Interest::Write);
        }
        assert!(!io.clear_if_unchanged(Interest::Write, token));
        assert!(is_ready(&io, Interest::Write));
    }

    #[test]
    fn directions_are_independent() {
        let io = ScheduledIo::with_initial(InitialReadiness::Optimistic);
        let read = io.snapshot(Interest::Read);
        io.mark_ready(Interest::Write);
        assert!(io.clear_if_unchanged(Interest::Read, read));
        assert!(!is_ready(&io, Interest::Read));
        assert!(is_ready(&io, Interest::Write));
    }
}
