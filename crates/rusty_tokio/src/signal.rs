//! Async signal handling: [`ctrl_c`] resolves once on the next `SIGINT`
//! (Unix) / Ctrl+C (Windows); [`signal`] (Unix-only) returns a [`Signal`]
//! that fires every time a given [`SignalKind`] arrives, for as long as
//! it's held. On Windows, [`windows::ctrl_break`]/[`windows::ctrl_close`]/
//! [`windows::ctrl_logoff`]/[`windows::ctrl_shutdown`] cover the four
//! additional `SetConsoleCtrlHandler` events with no POSIX equivalent --
//! see that submodule's own docs, and this module's "Windows" section
//! below, for why the API forks here rather than forcing every platform
//! through one generic `SignalKind` surface.
//!
//! **The self-pipe trick (Unix).** A signal handler can only safely do a
//! very limited set of things (a short, fixed list of async-signal-safe
//! functions -- notably not allocate, not lock a mutex, not touch most
//! of the runtime this crate would otherwise reach for), so
//! `handle_signal` does exactly one thing: an async-signal-safe
//! `write(2)` of the signal number to a pre-created pipe's write end,
//! whose fd is stashed in a plain [`AtomicI32`] so the handler can find
//! it without allocating or locking. Everything else -- looking up which
//! listeners care about that signal number, waking them -- happens later
//! in `reader_loop`, an ordinary spawned task reading the pipe's read
//! end through the same reactor every socket in this crate uses. This is
//! the standard, portable way real-world signal handling is built
//! (tokio's own driver, and most other signal-handling libraries, use
//! the identical shape); doing real work *inside* the OS signal handler
//! itself is the actual footgun this sidesteps.
//!
//! **Coalescing, not queuing.** Each listener's own `ListenerState`
//! is a single pending flag, not a growing counter -- if the same signal
//! kind arrives twice before a listener gets around to polling, that's
//! observed as one `Some(())`, not two. This matches how tokio's own
//! `Signal` behaves, and how signal delivery already tends to coalesce
//! at the OS level (a signal is not itself a queue).
//!
//! **Idempotent, additive installation.** `signal(kind)` installs a
//! `sigaction` handler for `kind` the *first* time any caller asks for
//! it, and never again afterward for that same kind -- calling it twice
//! for `SIGINT`, say, installs nothing the second time, it just adds
//! another independent listener that gets woken alongside the first.
//! Only signal numbers a caller actually requests are ever touched;
//! nothing here preemptively claims every signal, so a process's own
//! handlers for anything this crate was never asked about are left
//! completely alone.
//!
//! **Global, not per-`Runtime`.** The pipe, the reader task, and the
//! `sigaction` installations are process-wide state, set up once (lazily,
//! on the first `signal`/`ctrl_c` call) and reused for the life of the
//! process -- signals themselves are a process-wide concept, there's no
//! such thing as "the SIGINT for this one `Runtime`" if more than one
//! happens to be running. The reader task itself does run on whichever
//! `Runtime` happened to be current at that first call, though -- in the
//! (unusual) case of multiple concurrent `Runtime`s in one process, only
//! that first one's reactor and scheduler actually drive signal delivery
//! for every listener, regardless of which `Runtime` later callers are
//! on. Matches this crate's realistic, single-runtime-per-process usage;
//! not something to design around further without an actual need.
//!
//! # Windows
//!
//! Windows has no POSIX signal model at all -- the nearest equivalent is
//! [`SetConsoleCtrlHandler`](https://learn.microsoft.com/windows/console/setconsolectrlhandler),
//! which delivers a narrower, differently-shaped set of events
//! (Ctrl+C, Ctrl+Break, console-window-close, logoff, shutdown) than
//! POSIX signals, with no honest equivalent of `SIGTERM`/`SIGHUP`/
//! `SIGQUIT`/`SIGALRM`/`SIGCHLD`/`SIGPIPE`/`SIGUSR1`/`SIGUSR2`/`SIGWINCH`
//! at all. Rather than silently no-op those on Windows (a real behavioral
//! gap masquerading as success) or bolting Windows-only event names onto
//! `SignalKind` (blurring "exists on this platform" with "will simply
//! never fire"), the generic `signal`/`SignalKind` surface stays
//! `#[cfg(unix)]`-only -- calling it from Windows-targeted code is a
//! compile error, not a silent runtime no-op -- and a separate
//! `#[cfg(windows)]`-only [`windows`] submodule covers the four
//! console-control events with no Unix equivalent, mirroring tokio's own
//! `tokio::signal::windows` split exactly (same four event names, same
//! per-kind-listener-type shape). [`ctrl_c`] is the one function that
//! stays genuinely cross-platform: `SIGINT` on Unix, `CTRL_C_EVENT` on
//! Windows, both through the identical "resolves once on the next
//! interrupt" contract.
//!
//! The self-pipe trick still applies, structurally unchanged -- a
//! `SetConsoleCtrlHandler` callback runs on an OS-created thread with
//! none of a POSIX signal handler's async-signal-safety restrictions (it
//! can allocate, lock, block -- it's an ordinary thread, not interrupt
//! context), so it does a plain blocking one-byte write instead of the
//! `write(2)`-only self-pipe Unix needs, but the shape is the same:
//! push into a channel, let an ordinary spawned task read the other end
//! through the reactor. The channel itself is a synchronously-bootstrapped
//! loopback TCP pair (`127.0.0.1`, ephemeral port) rather than a real
//! pipe: this crate's Windows reactor (`io::reactor::windows`) is
//! socket-only (`io::reactor::RawIo` is `RawSocket`, not an arbitrary
//! `HANDLE`), and Windows has no anonymous `socketpair(2)`/`pipe(2)`
//! equivalent usable with it. See `docs/decision-request-windows-process-signal-ipc.md`
//! for the full reasoning and the option that wasn't chosen.

use std::io;
use std::sync::{Arc, Mutex, OnceLock, Weak};
use std::task::{Context, Poll, Waker};

#[cfg(unix)]
use crate::io::reactor::{ready_io, Interest, ScheduledIo};
#[cfg(unix)]
use crate::io::socket;
#[cfg(unix)]
use crate::runtime::Handle;
#[cfg(unix)]
use libc::c_int;
#[cfg(unix)]
use std::os::fd::{AsRawFd, FromRawFd, OwnedFd};
#[cfg(unix)]
use std::sync::atomic::{AtomicBool, AtomicI32, Ordering};

#[cfg(windows)]
use crate::io::reactor::{ready_io, Interest, ScheduledIo};
#[cfg(windows)]
use crate::io::socket::{self, windows::WindowsTcpStream};
#[cfg(windows)]
use crate::runtime::Handle;
#[cfg(windows)]
use std::net::{Ipv4Addr, SocketAddr, SocketAddrV4};
#[cfg(windows)]
use std::os::windows::io::AsRawSocket;
#[cfg(windows)]
use std::sync::atomic::{AtomicBool, AtomicU64, Ordering};
#[cfg(windows)]
use windows_sys::Win32::Foundation::{BOOL, FALSE, TRUE};
#[cfg(windows)]
use windows_sys::Win32::System::Console::{SetConsoleCtrlHandler, CTRL_C_EVENT};

/// Generous headroom past the highest standard POSIX signal number (31)
/// -- this crate only hands out constructors for the common named
/// signals, but [`SignalKind::from_raw`] accepts anything in range.
#[cfg(unix)]
const NSIG: usize = 64;

/// The self-pipe's write end -- read only from inside `handle_signal`,
/// a real OS signal handler, so a plain relaxed atomic load is all it
/// can safely do; never mutated again once `global` first sets it.
#[cfg(unix)]
static PIPE_WRITE_FD: AtomicI32 = AtomicI32::new(-1);

/// Shared by every listener flavor on every platform: [`Signal`] (Unix)
/// and every listener type in [`windows`] (Windows) -- a single pending
/// flag (see this module's own "Coalescing" docs) plus the waker to fire
/// once it flips.
struct ListenerState {
    pending: AtomicBool,
    waker: Mutex<Option<Waker>>,
}

/// One slot per possible signal number.
#[cfg(unix)]
struct SignalSlot {
    /// Whether a `sigaction` handler has been installed for this signal
    /// number yet. Checked and set while holding `listeners`'s own lock
    /// (not a separate atomic-swap dance) so "is it installed" and
    /// "append this listener" happen as one atomic step -- otherwise two
    /// callers racing `signal()` for the same brand-new kind could both
    /// decide they need to install it (harmless: `sigaction` with the
    /// same handler twice is a no-op-shaped redundant syscall, not a
    /// correctness bug) while a subtler race -- one caller's listener
    /// getting appended *before* installation actually succeeds, then
    /// that installation failing -- would leave a listener registered
    /// for a signal nothing will ever actually deliver notice of.
    installed: bool,
    listeners: Vec<Weak<ListenerState>>,
}

#[cfg(unix)]
struct Global {
    slots: Vec<Mutex<SignalSlot>>,
    /// Kept alive only so the write end's fd stays open for the whole
    /// process lifetime, matching what `handle_signal` assumes; never
    /// read back out.
    _write_fd: OwnedFd,
}

#[cfg(unix)]
static GLOBAL: OnceLock<io::Result<Global>> = OnceLock::new();

/// The only thing that runs inside the actual OS signal handler --
/// async-signal-safe by construction: one atomic load, one `write(2)`,
/// nothing else. See this module's own docs for why real work happens
/// later, in `reader_loop`, instead.
#[cfg(unix)]
extern "C" fn handle_signal(signum: c_int) {
    let fd = PIPE_WRITE_FD.load(Ordering::Relaxed);
    if fd < 0 {
        return;
    }
    let byte = signum as u8;
    // SAFETY: async-signal-safe -- `write(2)` is on the POSIX list of
    // functions safe to call from a signal handler. `fd` is a valid,
    // process-lifetime-owned pipe write end once this handler could
    // possibly run at all (installed only after the pipe already
    // exists). A short write to a pipe with room for at least one byte
    // (this module never lets it fill: `reader_loop` drains every byte
    // it can see on every wake) cannot itself block or partially write.
    unsafe {
        libc::write(fd, (&byte as *const u8).cast(), 1);
    }
}

#[cfg(unix)]
fn install_handler(signum: c_int) -> io::Result<()> {
    // SAFETY: `action` is fully initialized before `sigaction` reads it
    // (every field either zeroed or explicitly set below); `signum` is
    // caller-validated to be in range by `signal`'s own bounds check
    // before this is ever called.
    unsafe {
        let mut action: libc::sigaction = std::mem::zeroed();
        action.sa_sigaction = handle_signal as *const () as usize;
        libc::sigemptyset(&mut action.sa_mask);
        // SA_RESTART: a syscall interrupted by this signal resumes
        // instead of failing with EINTR -- the same "don't surprise
        // unrelated code elsewhere in the process" reasoning
        // `SA_RESTART` always carries, since this handler's own effect
        // (a two-byte pipe write) is otherwise invisible to whatever
        // the process was already doing when the signal arrived.
        action.sa_flags = libc::SA_RESTART;
        if libc::sigaction(signum, &action, std::ptr::null_mut()) != 0 {
            return Err(io::Error::last_os_error());
        }
    }
    Ok(())
}

#[cfg(unix)]
fn make_pipe() -> io::Result<(OwnedFd, OwnedFd)> {
    let mut fds = [0 as c_int; 2];
    // See `io::pipe::new_pipe`'s equivalent comment: `pipe2` is a Linux
    // extension originally, but every BSD in this crate's gate except
    // Darwin has since picked it up too -- only macOS genuinely lacks it.
    #[cfg(any(
        target_os = "linux",
        target_os = "freebsd",
        target_os = "openbsd",
        target_os = "netbsd",
        target_os = "dragonfly"
    ))]
    // SAFETY: `fds` is a valid, exclusively-borrowed 2-element out-param
    // for the call's duration.
    let rc = unsafe { libc::pipe2(fds.as_mut_ptr(), libc::O_CLOEXEC | libc::O_NONBLOCK) };
    #[cfg(target_os = "macos")]
    // SAFETY: same as the Linux arm; macOS has no `pipe2`, so
    // `O_CLOEXEC`/`O_NONBLOCK` are set via `fcntl` right after instead
    // (the same two-step reasoning `socket::new_tcp_socket`'s own macOS
    // arm documents).
    let rc = unsafe { libc::pipe(fds.as_mut_ptr()) };
    if rc != 0 {
        return Err(io::Error::last_os_error());
    }
    // SAFETY: both fds were just returned by `pipe`/`pipe2` above, valid,
    // otherwise-unowned, and each wrapped exactly once.
    let (read_fd, write_fd) =
        unsafe { (OwnedFd::from_raw_fd(fds[0]), OwnedFd::from_raw_fd(fds[1])) };
    #[cfg(target_os = "macos")]
    {
        socket::set_nonblocking(read_fd.as_raw_fd(), true)?;
        socket::set_nonblocking(write_fd.as_raw_fd(), true)?;
        // SAFETY: both fds are caller-owned and open; `FD_CLOEXEC` is
        // the sole variadic argument `F_SETFD` expects.
        unsafe {
            libc::fcntl(read_fd.as_raw_fd(), libc::F_SETFD, libc::FD_CLOEXEC);
            libc::fcntl(write_fd.as_raw_fd(), libc::F_SETFD, libc::FD_CLOEXEC);
        }
    }
    Ok((read_fd, write_fd))
}

#[cfg(unix)]
async fn reader_loop(io: Arc<ScheduledIo>, read_fd: OwnedFd) {
    let raw_fd = read_fd.as_raw_fd();
    loop {
        let mut buf = [0u8; 64];
        let n = match ready_io(&io, Interest::Read, || socket::read(raw_fd, &mut buf)).await {
            Ok(n) if n > 0 => n,
            // The write end lives in `Global` for the whole process
            // lifetime, so `n == 0` (EOF) should never actually happen;
            // an error here means the reactor itself is in trouble.
            // Either way, nothing sensible to do but stop reading --
            // every future `signal()` call already registered its
            // listener, but none will ever be woken again.
            _ => return,
        };
        for &signum in &buf[..n] {
            dispatch(signum as c_int);
        }
    }
}

#[cfg(unix)]
fn dispatch(signum: c_int) {
    let Some(Ok(global)) = GLOBAL.get() else {
        return;
    };
    let Some(slot) = global.slots.get(signum as usize) else {
        return;
    };
    let mut slot = slot.lock().unwrap();
    slot.listeners.retain(|weak| {
        let Some(listener) = weak.upgrade() else {
            // The `Signal` this listener belonged to was dropped --
            // drop the now-dangling weak reference too instead of
            // carrying it forever.
            return false;
        };
        listener.pending.store(true, Ordering::Release);
        if let Some(waker) = listener.waker.lock().unwrap().take() {
            waker.wake();
        }
        true
    });
}

#[cfg(unix)]
fn global() -> io::Result<&'static Global> {
    let result = GLOBAL.get_or_init(|| -> io::Result<Global> {
        let reactor = Handle::current().shared.reactor.clone();
        let (read_fd, write_fd) = make_pipe()?;
        PIPE_WRITE_FD.store(write_fd.as_raw_fd(), Ordering::Relaxed);
        let io = reactor.register(read_fd.as_raw_fd())?;
        crate::spawn(reader_loop(io, read_fd));

        let slots = (0..NSIG)
            .map(|_| {
                Mutex::new(SignalSlot {
                    installed: false,
                    listeners: Vec::new(),
                })
            })
            .collect();
        Ok(Global {
            slots,
            _write_fd: write_fd,
        })
    });
    match result {
        Ok(global) => Ok(global),
        // `io::Error` isn't `Clone`, and initialization failing at all
        // is exceptionally unlikely (a `pipe()`/reactor-registration
        // failure) -- report it the same way every call after the
        // first would otherwise see it (a fresh, equivalent error),
        // rather than trying to hand back the original.
        Err(e) => Err(io::Error::new(e.kind(), e.to_string())),
    }
}

/// A signal kind -- either one of the common named constructors below,
/// or [`SignalKind::from_raw`] for anything else. Unix-only -- see this
/// module's own "Windows" docs for why, and for the [`windows`]
/// submodule that covers Windows' own console-control events instead.
#[cfg(unix)]
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub struct SignalKind(c_int);

#[cfg(unix)]
impl SignalKind {
    pub fn from_raw(signum: c_int) -> SignalKind {
        SignalKind(signum)
    }

    pub fn as_raw_value(&self) -> c_int {
        self.0
    }

    pub fn hangup() -> SignalKind {
        SignalKind(libc::SIGHUP)
    }

    pub fn interrupt() -> SignalKind {
        SignalKind(libc::SIGINT)
    }

    pub fn quit() -> SignalKind {
        SignalKind(libc::SIGQUIT)
    }

    pub fn terminate() -> SignalKind {
        SignalKind(libc::SIGTERM)
    }

    pub fn alarm() -> SignalKind {
        SignalKind(libc::SIGALRM)
    }

    pub fn child() -> SignalKind {
        SignalKind(libc::SIGCHLD)
    }

    pub fn pipe() -> SignalKind {
        SignalKind(libc::SIGPIPE)
    }

    pub fn user_defined1() -> SignalKind {
        SignalKind(libc::SIGUSR1)
    }

    pub fn user_defined2() -> SignalKind {
        SignalKind(libc::SIGUSR2)
    }

    pub fn window_change() -> SignalKind {
        SignalKind(libc::SIGWINCH)
    }
}

/// A listener for one [`SignalKind`], firing every time it arrives for
/// as long as this value is held. Dropping it stops it from being woken
/// -- other listeners for the same kind (including ones registered
/// before or after) are unaffected. Unix-only -- see [`windows`] for the
/// per-kind listener types Windows uses instead.
///
/// # Panics
/// [`signal`] (which every `Signal` is created through) panics if
/// called outside a running [`crate::Runtime`].
#[cfg(unix)]
pub struct Signal {
    listener: Arc<ListenerState>,
}

#[cfg(unix)]
impl Signal {
    /// Resolves once this signal kind next arrives -- immediately, if
    /// it already has since the last call (or since this `Signal` was
    /// created, for the first call). Always `Some(())`; the `Option`
    /// shape exists only for consistency with `recv`-style methods
    /// elsewhere in this crate -- a real OS signal source never
    /// meaningfully "ends".
    pub async fn recv(&mut self) -> Option<()> {
        std::future::poll_fn(|cx| self.poll_recv(cx)).await
    }

    fn poll_recv(&self, cx: &mut Context<'_>) -> Poll<Option<()>> {
        poll_listener(&self.listener, cx).map(Some)
    }
}

/// Shared by [`Signal::poll_recv`] (Unix) and every listener type in
/// [`windows`] (Windows) -- the same re-check-after-register shape
/// [`crate::io::reactor::ScheduledIo::poll_ready`] uses, for the
/// identical "don't lose a wakeup that raced registration" reason.
fn poll_listener(listener: &ListenerState, cx: &mut Context<'_>) -> Poll<()> {
    if listener.pending.swap(false, Ordering::AcqRel) {
        return Poll::Ready(());
    }
    *listener.waker.lock().unwrap() = Some(cx.waker().clone());
    if listener.pending.swap(false, Ordering::AcqRel) {
        return Poll::Ready(());
    }
    Poll::Pending
}

/// Listens for `kind`. Installs a `sigaction` handler for it the first
/// time any caller asks for this particular kind (see this module's own
/// docs); every call, including the first, adds an independent listener
/// that gets woken on every occurrence from here on. Unix-only.
///
/// # Panics
/// Panics if called outside a running [`crate::Runtime`].
#[cfg(unix)]
pub fn signal(kind: SignalKind) -> io::Result<Signal> {
    let global = global()?;
    let signum = kind.0 as usize;
    let Some(slot) = global.slots.get(signum) else {
        return Err(io::Error::new(
            io::ErrorKind::InvalidInput,
            "signal number out of range",
        ));
    };

    let listener = Arc::new(ListenerState {
        pending: AtomicBool::new(false),
        waker: Mutex::new(None),
    });

    let mut slot = slot.lock().unwrap();
    if !slot.installed {
        install_handler(kind.0)?;
        slot.installed = true;
    }
    slot.listeners.push(Arc::downgrade(&listener));
    drop(slot);

    Ok(Signal { listener })
}

/// Resolves once on the next `SIGINT` ("Ctrl-C" at an interactive
/// terminal). Equivalent to `signal(SignalKind::interrupt())?.recv()`,
/// for the common case of only ever caring about one occurrence.
///
/// # Panics
/// Panics if called outside a running [`crate::Runtime`].
#[cfg(unix)]
pub async fn ctrl_c() -> io::Result<()> {
    signal(SignalKind::interrupt())?.recv().await;
    Ok(())
}

// =========================================================================
// Windows
// =========================================================================
//
// See this module's own top-level "Windows" docs for the design. Short
// version: one `SetConsoleCtrlHandler` callback (installed once, ever --
// unlike Unix's per-signal-number `sigaction`, there is only one console
// handler registration mechanism, covering every event) writes a single
// byte (which event fired) to a loopback-socket self-pipe; an ordinary
// spawned task reads the other end through the reactor and wakes whichever
// listeners asked for that event, reusing the same `ListenerState`/
// `poll_listener` shape `Signal` uses on Unix.

/// One slot per console-control event this module hands out a listener
/// constructor for (`CTRL_C_EVENT` = 0 through `CTRL_SHUTDOWN_EVENT` = 6
/// -- indices 3/4 are unused by any named event, left as permanently-empty
/// slots rather than a sparser map for a fixed, tiny event set).
#[cfg(windows)]
const NCTRL: usize = 7;

#[cfg(windows)]
struct WindowsSlot {
    listeners: Vec<Weak<ListenerState>>,
}

#[cfg(windows)]
struct WindowsGlobal {
    slots: [Mutex<WindowsSlot>; NCTRL],
    /// Kept alive only so the write end's socket stays open for the
    /// whole process lifetime, matching what `console_ctrl_handler`
    /// assumes; never read back out. See `Global::_write_fd`'s
    /// identical Unix-arm role.
    _write_sock: WindowsTcpStream,
}

#[cfg(windows)]
static WINDOWS_GLOBAL: OnceLock<io::Result<WindowsGlobal>> = OnceLock::new();

/// The self-pipe's write end -- read only from inside
/// `console_ctrl_handler`. Unlike Unix's `PIPE_WRITE_FD`, nothing here
/// forces a relaxed-only load for async-signal-safety reasons (a console
/// control handler runs on an ordinary thread, not in interrupt
/// context) -- plain `AtomicU64` (the same width as `RawSocket`) is used
/// anyway, matching `PIPE_WRITE_FD`'s shape for the identical
/// "handler finds it without a lock" purpose. `u64::MAX` (never a valid
/// `SOCKET`) is the "not yet initialized" sentinel.
#[cfg(windows)]
static PIPE_WRITE_SOCKET: AtomicU64 = AtomicU64::new(u64::MAX);

/// The only thing `console_ctrl_handler` needs to decide safely without
/// touching `WINDOWS_GLOBAL` under contention it can't otherwise
/// coordinate with: whether *any* listener anywhere is currently asking
/// for the event that just fired, so an event nobody registered for can
/// return `FALSE` (let the next handler / default OS action run) rather
/// than being silently swallowed. Reused, not process-specific: the
/// handler still writes the actual event byte through the self-pipe;
/// `dispatch` (on the reader task) does the real per-listener wake-up
/// work, exactly mirroring Unix's `handle_signal`/`dispatch` split.
#[cfg(windows)]
fn console_event_has_listeners(ctrl_type: u32) -> bool {
    let Some(Ok(global)) = WINDOWS_GLOBAL.get() else {
        return false;
    };
    let Some(slot) = global.slots.get(ctrl_type as usize) else {
        return false;
    };
    !slot.lock().unwrap().listeners.is_empty()
}

/// The only thing that runs inside the actual `SetConsoleCtrlHandler`
/// callback. Unlike Unix's `handle_signal`, this runs on an ordinary
/// OS-created thread (not interrupt/signal context), so it's under no
/// async-signal-safety restriction -- it can lock a mutex
/// ([`console_event_has_listeners`] does) and it does a plain *blocking*
/// one-byte socket write rather than needing Unix's non-blocking,
/// async-signal-safe `write(2)`. Still kept minimal on purpose: real
/// work (waking listeners) happens later in `reader_loop`, the same
/// split Unix's own docs explain.
#[cfg(windows)]
extern "system" fn console_ctrl_handler(ctrl_type: u32) -> BOOL {
    if !console_event_has_listeners(ctrl_type) {
        return FALSE;
    }
    let sock = PIPE_WRITE_SOCKET.load(Ordering::Relaxed);
    if sock == u64::MAX {
        return FALSE;
    }
    let byte = [ctrl_type as u8];
    // Best-effort: nothing sensible to do with a write failure from
    // inside this callback, and the reader task treats "nothing more
    // ever arrives" the same way Unix's does (see `reader_loop`).
    let _ = socket::write(sock, &byte);
    TRUE
}

/// A plain blocking TCP socket, deliberately *not* flipped non-blocking
/// the way `io::socket::windows::new_tcp_socket` always does (that one
/// exists specifically for the non-blocking-before-connect dance
/// `TcpStream`/`UnixStream::connect` need) -- routed through it anyway
/// (rather than a hand-rolled `WinSock::socket()` call) purely to reuse
/// its `WSAStartup`-once guarantee, which this module has no other way
/// to trigger (`io::socket::windows::wsa_init` is private to that
/// module). This crate's self-pipe bootstrap wants a genuinely blocking
/// `connect`/`accept` pair instead -- see [`windows_socket_pair`].
#[cfg(windows)]
fn new_blocking_tcp_socket(addr: SocketAddr) -> io::Result<std::os::windows::io::OwnedSocket> {
    let sock = socket::new_tcp_socket(addr)?;
    socket::set_nonblocking(sock.as_raw_socket(), false)?;
    Ok(sock)
}

/// Windows' self-pipe equivalent: a loopback TCP pair, synchronously
/// bootstrapped (both sockets genuinely blocking, not the non-blocking-
/// connect-then-poll-for-writability dance `TcpStream::connect` needs --
/// a loopback handshake settles synchronously either way, and this runs
/// once, lazily, with no reactor necessarily available to drive polling
/// yet). See this module's own top-level "Windows" docs for why a
/// loopback socket pair stands in for a real pipe here at all (this
/// crate's Windows reactor is socket-only).
#[cfg(windows)]
fn windows_socket_pair() -> io::Result<(WindowsTcpStream, WindowsTcpStream)> {
    use crate::io::socket::windows::WindowsTcpListener;

    let loopback0 = SocketAddr::V4(SocketAddrV4::new(Ipv4Addr::LOCALHOST, 0));
    let listener_sock = new_blocking_tcp_socket(loopback0)?;
    socket::bind(listener_sock.as_raw_socket(), loopback0)?;
    socket::listen(listener_sock.as_raw_socket(), 1)?;
    let listener = WindowsTcpListener::from(listener_sock);
    let addr = listener.local_addr().map_err(socket::from_platform_err)?;

    let connector_sock = new_blocking_tcp_socket(addr)?;
    // A blocking `connect()` to loopback blocks briefly (sub-millisecond
    // in practice) until the handshake genuinely completes, then returns
    // -- no `WSAEWOULDBLOCK`/`SO_ERROR`-polling ambiguity the way a
    // non-blocking connect would have (`SO_ERROR` alone can't
    // distinguish "still connecting" from "connected" without first
    // waiting for write-readiness, which needs a reactor this bootstrap
    // deliberately avoids depending on).
    socket::connect(connector_sock.as_raw_socket(), addr)?;
    let connector = WindowsTcpStream::from(connector_sock);

    // The connect above only returns once the handshake has completed,
    // so the listener already has a queued connection by now -- this
    // `accept()` (on a still-blocking listener socket) returns
    // immediately rather than genuinely blocking.
    let (accepted, _peer) = listener.accept().map_err(socket::from_platform_err)?;

    Ok((accepted, connector))
}

#[cfg(windows)]
async fn reader_loop(io: Arc<ScheduledIo>, read_sock: WindowsTcpStream) {
    loop {
        let mut buf = [0u8; 64];
        let n = match ready_io(&io, Interest::Read, || {
            socket::read(read_sock.as_raw_socket(), &mut buf)
        })
        .await
        {
            Ok(n) if n > 0 => n,
            // The write end lives in `WindowsGlobal` for the whole
            // process lifetime, so `n == 0` (EOF) should never actually
            // happen -- see `reader_loop`'s (Unix) identical comment.
            _ => return,
        };
        for &ctrl_type in &buf[..n] {
            dispatch(ctrl_type as u32);
        }
    }
}

#[cfg(windows)]
fn dispatch(ctrl_type: u32) {
    let Some(Ok(global)) = WINDOWS_GLOBAL.get() else {
        return;
    };
    let Some(slot) = global.slots.get(ctrl_type as usize) else {
        return;
    };
    let mut slot = slot.lock().unwrap();
    slot.listeners.retain(|weak| {
        let Some(listener) = weak.upgrade() else {
            return false;
        };
        listener.pending.store(true, Ordering::Release);
        if let Some(waker) = listener.waker.lock().unwrap().take() {
            waker.wake();
        }
        true
    });
}

#[cfg(windows)]
fn global() -> io::Result<&'static WindowsGlobal> {
    let result = WINDOWS_GLOBAL.get_or_init(|| -> io::Result<WindowsGlobal> {
        let reactor = Handle::current().shared.reactor.clone();
        let (read_sock, write_sock) = windows_socket_pair()?;
        PIPE_WRITE_SOCKET.store(write_sock.as_raw_socket(), Ordering::Relaxed);
        read_sock
            .set_nonblocking(true)
            .map_err(socket::from_platform_err)?;
        let io = reactor.register(read_sock.as_raw_socket())?;
        crate::spawn(reader_loop(io, read_sock));

        // SAFETY: `console_ctrl_handler` has the exact `PHANDLER_ROUTINE`
        // signature (`extern "system" fn(u32) -> BOOL`) `Some(..)` needs
        // to coerce into; `TRUE` *adds* this handler rather than
        // removing it -- every call through `global()` installs, never
        // uninstalls (matching `install_handler`'s Unix-arm idempotence:
        // installed exactly once, on the first call, here implicitly via
        // `OnceLock::get_or_init` rather than a per-signal-number `bool`
        // since there's only one handler registration to make at all).
        let installed = unsafe { SetConsoleCtrlHandler(Some(console_ctrl_handler), TRUE) };
        if installed == FALSE {
            return Err(io::Error::last_os_error());
        }

        let slots = std::array::from_fn(|_| {
            Mutex::new(WindowsSlot {
                listeners: Vec::new(),
            })
        });
        Ok(WindowsGlobal {
            slots,
            _write_sock: write_sock,
        })
    });
    match result {
        Ok(global) => Ok(global),
        // See `global`'s (Unix) identical comment: `io::Error` isn't
        // `Clone`, so every call after the first that observes a failed
        // initialization gets a fresh, equivalent error instead.
        Err(e) => Err(io::Error::new(e.kind(), e.to_string())),
    }
}

#[cfg(windows)]
fn listen_for(ctrl_type: u32) -> io::Result<Arc<ListenerState>> {
    let global = global()?;
    let Some(slot) = global.slots.get(ctrl_type as usize) else {
        return Err(io::Error::new(
            io::ErrorKind::InvalidInput,
            "unrecognized console control event",
        ));
    };
    let listener = Arc::new(ListenerState {
        pending: AtomicBool::new(false),
        waker: Mutex::new(None),
    });
    slot.lock()
        .unwrap()
        .listeners
        .push(Arc::downgrade(&listener));
    Ok(listener)
}

/// Resolves once on the next Ctrl+C (`CTRL_C_EVENT`). Equivalent to
/// `signal(SignalKind::interrupt())?.recv()` on Unix, through a
/// genuinely different mechanism underneath (`SetConsoleCtrlHandler`
/// instead of `sigaction`/`SIGINT`) -- see this module's own "Windows"
/// docs.
///
/// # Panics
/// Panics if called outside a running [`crate::Runtime`].
#[cfg(windows)]
pub async fn ctrl_c() -> io::Result<()> {
    let listener = listen_for(CTRL_C_EVENT)?;
    std::future::poll_fn(|cx| poll_listener(&listener, cx)).await;
    Ok(())
}

/// Windows-only console-control events with no POSIX equivalent --
/// `SetConsoleCtrlHandler`'s four events besides Ctrl+C (which
/// [`super::ctrl_c`] already covers, cross-platform). See the parent
/// module's own "Windows" docs for why these live in their own
/// submodule instead of being folded into a generic `SignalKind`, and
/// for the self-pipe-over-loopback-socket mechanism every listener type
/// here shares.
///
/// Mirrors `tokio::signal::windows`'s own shape exactly: one type per
/// event, each with its own `recv()`, each coalescing repeated
/// occurrences into a single pending notification the same way
/// [`super::Signal`] does on Unix.
#[cfg(windows)]
pub mod windows {
    use super::{listen_for, poll_listener, ListenerState};
    use std::io;
    use std::sync::Arc;
    use windows_sys::Win32::System::Console::{
        CTRL_BREAK_EVENT, CTRL_CLOSE_EVENT, CTRL_LOGOFF_EVENT, CTRL_SHUTDOWN_EVENT,
    };

    /// Delivered when the process is running as a console process and
    /// Ctrl+Break is pressed. No Unix equivalent (`SIGINT`'s nearest
    /// analog is [`super::ctrl_c`], not this).
    pub struct CtrlBreak(Arc<ListenerState>);

    /// Listens for `CTRL_BREAK_EVENT`.
    ///
    /// # Panics
    /// Panics if called outside a running [`crate::Runtime`].
    pub fn ctrl_break() -> io::Result<CtrlBreak> {
        listen_for(CTRL_BREAK_EVENT).map(CtrlBreak)
    }

    impl CtrlBreak {
        /// Resolves once this event next arrives -- see [`super::Signal::recv`]
        /// for the identical coalescing/immediate-if-already-pending
        /// contract.
        pub async fn recv(&mut self) -> Option<()> {
            std::future::poll_fn(|cx| poll_listener(&self.0, cx)).await;
            Some(())
        }
    }

    /// Delivered when the console window is being closed. Windows gives
    /// the process a short grace period to handle this before it's
    /// force-terminated regardless of what this handler does -- there is
    /// no way to indefinitely veto a console close the way ignoring
    /// `SIGTERM` can indefinitely ignore a Unix termination request.
    pub struct CtrlClose(Arc<ListenerState>);

    /// Listens for `CTRL_CLOSE_EVENT`.
    ///
    /// # Panics
    /// Panics if called outside a running [`crate::Runtime`].
    pub fn ctrl_close() -> io::Result<CtrlClose> {
        listen_for(CTRL_CLOSE_EVENT).map(CtrlClose)
    }

    impl CtrlClose {
        /// See [`CtrlBreak::recv`].
        pub async fn recv(&mut self) -> Option<()> {
            std::future::poll_fn(|cx| poll_listener(&self.0, cx)).await;
            Some(())
        }
    }

    /// Delivered when the current user is logging off. Not sent to
    /// services (only interactive console processes) -- no Unix
    /// equivalent.
    pub struct CtrlLogoff(Arc<ListenerState>);

    /// Listens for `CTRL_LOGOFF_EVENT`.
    ///
    /// # Panics
    /// Panics if called outside a running [`crate::Runtime`].
    pub fn ctrl_logoff() -> io::Result<CtrlLogoff> {
        listen_for(CTRL_LOGOFF_EVENT).map(CtrlLogoff)
    }

    impl CtrlLogoff {
        /// See [`CtrlBreak::recv`].
        pub async fn recv(&mut self) -> Option<()> {
            std::future::poll_fn(|cx| poll_listener(&self.0, cx)).await;
            Some(())
        }
    }

    /// Delivered when the system is shutting down. Same short-grace-period
    /// caveat as [`CtrlClose`] -- this is a notice, not an indefinitely
    /// vetoable request the way ignoring `SIGTERM` is on Unix.
    pub struct CtrlShutdown(Arc<ListenerState>);

    /// Listens for `CTRL_SHUTDOWN_EVENT`.
    ///
    /// # Panics
    /// Panics if called outside a running [`crate::Runtime`].
    pub fn ctrl_shutdown() -> io::Result<CtrlShutdown> {
        listen_for(CTRL_SHUTDOWN_EVENT).map(CtrlShutdown)
    }

    impl CtrlShutdown {
        /// See [`CtrlBreak::recv`].
        pub async fn recv(&mut self) -> Option<()> {
            std::future::poll_fn(|cx| poll_listener(&self.0, cx)).await;
            Some(())
        }
    }
}
