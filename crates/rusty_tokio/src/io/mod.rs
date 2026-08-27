//! Non-blocking networking: `epoll` on Linux, `kevent` on macOS, IOCP +
//! the AFD-poll trick on Windows -- see `reactor/mod.rs` for the
//! shared/per-backend split, `reactor/kqueue.rs`'s docs for the caveat
//! that this crate's own integration with the macOS backend is
//! compile-checked (`cargo check --target x86_64-apple-darwin`) but has
//! never been run on real hardware, and `reactor/windows.rs`'s docs for
//! the identical caveat on Windows (`cargo check --target
//! x86_64-pc-windows-gnu`). A fourth backend, `reactor/io_uring.rs`,
//! swaps `epoll` for `IORING_OP_POLL_ADD` on Linux behind the
//! `io-uring-reactor` feature (off by default) -- see that module's docs
//! for scope and why.
//!
//! Socket bind/connect/accept/addressing is built on `rustils`' concrete
//! `platform_linux::{LinuxTcpListener, LinuxTcpStream, LinuxUdpSocket,
//! LinuxUnixListener, LinuxUnixStream}` on Linux,
//! `platform_macos::{MacosTcpListener, MacosTcpStream, MacosUdpSocket,
//! MacosUnixListener, MacosUnixStream}` on macOS (see `socket/mod.rs`'s
//! docs for the small remainder that's still hand-rolled on both), and
//! `platform_windows::{WindowsUnixListener, WindowsUnixStream}` on
//! Windows for `unix.rs` specifically (see that module's own docs, and
//! `docs/decision-request-windows-process-signal-ipc.md`, for why
//! Windows leans on rustils there but not for `tcp.rs`/`udp.rs`, which
//! stay on the hand-rolled `socket::windows` arm below) -- shaped
//! identically enough between the backends that `tcp.rs`/`udp.rs`/
//! `unix.rs` each need only a `#[cfg]`-gated type alias, not their own OS
//! branching. `UnixDatagram` is the one exception -- rustils has no
//! `AF_UNIX` datagram support at all, on *any* platform, so
//! `unix_datagram.rs` wraps `std::os::unix::net::UnixDatagram` directly
//! instead; see that module's own docs for why, and for why it stays
//! `#[cfg(unix)]`-only (no stable Windows equivalent to wrap the same
//! way -- `std::os::windows::net` is nightly-only as of this writing).
//!
//! `unix.rs` (`UnixListener`/`UnixStream`) compiles on `unix`/`windows`
//! both; its bare pre-bind `UnixSocket` builder and `UnixStream::pair`
//! stay `#[cfg(unix)]`-only -- see that module's own docs for the
//! specific rustils-side gap (no owned-socket adoption on Windows yet)
//! and OS-level absence (no anonymous `AF_UNIX` pair on Windows) behind
//! each. `unix_datagram.rs` stays `#[cfg(unix)]`-only entirely, per
//! above.

mod addr;
#[cfg(unix)]
mod async_fd;
mod async_io;
mod buffered;
#[cfg(feature = "futures-io-compat")]
mod compat;
mod duplex;
mod interest;
mod join;
mod lookup;
#[cfg(unix)]
mod pipe;
pub(crate) mod reactor;
mod readiness;
mod simplex;
pub(crate) mod socket;
mod split;
mod stdio;
mod tcp;
mod udp;
#[cfg(any(unix, windows))]
mod unix;
#[cfg(unix)]
mod unix_datagram;
#[cfg(all(target_os = "linux", feature = "io-uring-fs"))]
mod uring_fs;
mod util;

pub use addr::ToSocketAddrs;
#[cfg(unix)]
pub use async_fd::{AsyncFd, AsyncFdReadyGuard, TryIoError};
pub use async_io::{
    copy, copy_bidirectional, copy_bidirectional_with_sizes, copy_buf, AsyncBufRead,
    AsyncBufReadExt, AsyncRead, AsyncReadExt, AsyncSeek, AsyncSeekExt, AsyncWrite, AsyncWriteExt,
    Chain, FillBuf, ReadBuf, Take,
};
pub use buffered::{BufReader, BufStream, BufWriter, Lines};
#[cfg(feature = "futures-io-compat")]
pub use compat::Compat;
pub use duplex::{duplex, DuplexStream};
pub use interest::{Interest, Ready};
pub use join::{join, Join};
pub use lookup::{lookup_host, LookupHost};
#[cfg(unix)]
pub use pipe::{pipe, PipeOpenOptions, PipeReceiver, PipeSender};
pub use simplex::{simplex, SimplexStream};
pub use split::{split, SplitReadHalf, SplitWriteHalf};
pub use stdio::{stderr, stdin, stdout, Stderr, Stdin, Stdout};
pub use tcp::{
    OwnedReadHalf, OwnedWriteHalf, ReadHalf, ReuniteError, TcpListener, TcpSocket, TcpStream,
    WriteHalf,
};
pub use udp::{UdpSocket, MAX_UDP_DATAGRAM_SIZE};
#[cfg(unix)]
pub use unix::{gid_t, pid_t, uid_t, UnixSocket};
#[cfg(any(unix, windows))]
pub use unix::{
    OwnedUnixReadHalf, OwnedUnixWriteHalf, UnixListener, UnixReadHalf, UnixReuniteError,
    UnixSocketAddr, UnixStream, UnixWriteHalf,
};
// `UnixStream::peer_cred`'s return type -- not available on generic BSD
// yet, see that method's own docs, so `UCred` doesn't exist there either.
#[cfg(any(target_os = "linux", target_os = "macos"))]
pub use unix::UCred;
#[cfg(unix)]
pub use unix_datagram::UnixDatagram;
#[cfg(all(target_os = "linux", feature = "io-uring-fs"))]
pub use uring_fs::{
    global_driver as uring_global_driver, remove_file as uring_remove_file,
    remove_file_on as uring_remove_file_on, rename as uring_rename, rename_on as uring_rename_on,
    BoxFuture as UringBoxFuture, BufResult, IoBuf, IoBufMut, IoUringDriver, OpDriver,
    OpenOptions as UringOpenOptions, SimDriver, UringFile,
};
pub use util::{empty, repeat, sink, Empty, Repeat, Sink};
