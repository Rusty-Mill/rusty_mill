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
use std::sync::atomic::{AtomicBool, Ordering};
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

/// Per-registered-fd readiness state: one bit each for readable and
/// writable, plus the waker to fire when that bit flips on.
pub(crate) struct ScheduledIo {
    readable: AtomicBool,
    writable: AtomicBool,
    read_waker: Mutex<Option<Waker>>,
    write_waker: Mutex<Option<Waker>>,
}

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
            readable: AtomicBool::new(true),
            writable: AtomicBool::new(matches!(initial, InitialReadiness::Optimistic)),
            read_waker: Mutex::new(None),
            write_waker: Mutex::new(None),
        }
    }

    fn poll_ready(&self, cx: &mut Context<'_>, interest: Interest) -> Poll<()> {
        let (flag, waker_slot) = match interest {
            Interest::Read => (&self.readable, &self.read_waker),
            Interest::Write => (&self.writable, &self.write_waker),
        };
        if flag.load(Ordering::Acquire) {
            return Poll::Ready(());
        }
        *waker_slot.lock().unwrap() = Some(cx.waker().clone());
        // Re-check after registering the waker: the reactor thread may
        // have flipped the bit between our first load and taking the
        // lock above, and if we didn't check again that wakeup would be
        // lost (nothing left to observe the flag flip).
        if flag.load(Ordering::Acquire) {
            return Poll::Ready(());
        }
        Poll::Pending
    }

    fn clear(&self, interest: Interest) {
        match interest {
            Interest::Read => self.readable.store(false, Ordering::Release),
            Interest::Write => self.writable.store(false, Ordering::Release),
        }
    }

    /// Called by a backend's event loop when it observes `interest` is
    /// ready on this fd. Plain private visibility -- not `pub(super)` --
    /// is enough: `epoll`/`kqueue` are child modules of `reactor`, and
    /// Rust's default visibility already reaches every descendant of the
    /// defining module.
    fn mark_ready(&self, interest: Interest) {
        let (flag, waker_slot) = match interest {
            Interest::Read => (&self.readable, &self.read_waker),
            Interest::Write => (&self.writable, &self.write_waker),
        };
        flag.store(true, Ordering::Release);
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
        match op() {
            Ok(v) => return Poll::Ready(Ok(v)),
            Err(e) if e.kind() == io::ErrorKind::WouldBlock => {
                io.clear(interest);
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

/// Clears `io`'s cached `interest` readiness -- called after a
/// `WouldBlock` proves the previous "ready" signal was stale. The
/// [`poll_io`]/[`ready_io`] loop above does this internally on every
/// `WouldBlock`; [`super::AsyncFdReadyGuard::clear_ready`] and
/// [`super::readiness::try_io`] expose the same step for callers doing
/// their own I/O outside this module.
pub(crate) fn clear_ready(io: &std::sync::Arc<ScheduledIo>, interest: Interest) {
    io.clear(interest)
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
