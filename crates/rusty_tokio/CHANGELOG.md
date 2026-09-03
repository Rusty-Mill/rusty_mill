# Changelog

All notable changes to `rusty_tokio` are recorded here, starting from the
first tagged release. Versions follow [Semantic Versioning](https://semver.org/):
a bump to the second number (`0.X.0`) means a breaking change -- including a
trait-identity break, the sharpest edge for crates generic over this one's
`AsyncRead`/`AsyncWrite` (see [#107](https://github.com/baileyrd/rusty_tokio/issues/107))
-- while a bump to the third (`0.1.X`) is purely additive.

No changelog was kept before this point; `git log` is the record for
anything prior to v0.2.0.

## [Unreleased]

### Added

- Windows support for `process::Command`/`Child`, `signal`, and
  `io::UnixStream`/`UnixListener` -- previously `#[cfg(unix)]`-gated out
  of the crate entirely on Windows. `process`: spawn/wait/kill match the
  Unix arm exactly (portable `std::process::Child` methods); piped
  `ChildStdin`/`ChildStdout`/`ChildStderr` are `spawn_blocking`-backed on
  Windows instead of reactor-driven (this crate's Windows reactor is
  socket-only; see `docs/decision-request-windows-process-signal-ipc.md`).
  `signal`: `signal::ctrl_c()` stays cross-platform; the generic
  `signal(SignalKind)`/named `SignalKind` constructors stay
  `#[cfg(unix)]`-only (no honest Windows equivalent for most of them), and
  a new `signal::windows` submodule (`ctrl_break`/`ctrl_close`/
  `ctrl_logoff`/`ctrl_shutdown`) covers Windows' own console-control
  events, mirroring `tokio::signal::windows`. `io::unix`: `UnixStream`/
  `UnixListener` now build on `platform_windows` (rustils#59's escape
  hatch) the same way Linux/BSD build on `platform_linux`/`platform_bsd`
  -- `UnixDatagram`, `UnixStream::pair`, and the bare `UnixSocket`
  builder stay `#[cfg(unix)]`-only (real, separate gaps -- no `AF_UNIX`
  datagram support in rustils on any platform, no anonymous `AF_UNIX`
  pair primitive on Windows at the OS level, and no owned-socket
  adoption in `platform_windows` yet -- see the design doc). Verified
  with a real `cargo build`/`cargo test` run on native Windows hardware
  in the same session (not just `cargo check --target
  x86_64-pc-windows-gnu`), which also exercised the pre-existing
  TCP/UDP/IOCP+AFD reactor code on real hardware for the first time (see
  the "What's deliberately not here (yet)" real-hardware-verification
  caveat in the README). No tracking issue number -- flagged here rather
  than fabricated; this session's `gh` access was unavailable to open one.

### Fixed

- The Windows reactor could permanently stop monitoring a socket if the
  `IOCTL_AFD_POLL` re-arm submitted after a completion (AFD poll is
  one-shot, unlike `epoll`/`kqueue`) itself failed -- the failure was
  silently discarded (`let _ = self.submit_poll(&state)`), so no further
  completion would ever arrive for that socket and any
  `readable()`/`writable()` wait registered on it afterward hung
  forever, observed only as nextest's own ~600s slow-timeout killing an
  otherwise-unrelated test (Rusty-Mill/rusty_mill#153). Both `event_loop`
  call sites now check the resubmission's result and, on failure, mark
  both directions ready (a `mark_orphaned` helper mirroring the existing
  bad-completion-status branch) so the caller's own next syscall
  discovers the real problem instead of the wait hanging with nothing
  left to wake it.

- Readiness is no longer lost when an edge lands between a failed
  syscall and the clear that follows it. Every `WouldBlock` path
  (`poll_io` behind `read`/`write`/`connect`/`accept`, `try_io` behind
  `try_read`/`try_write`/`try_peek`/..., and `AsyncFdReadyGuard::
  clear_ready`) used to attempt the syscall and then unconditionally
  clear the cached readiness bit. Under the edge-triggered backends
  (`EPOLLET`, `EV_CLEAR`, the one-shot AFD/io_uring polls) an edge the
  reactor delivered in that window was wiped and never re-reported, so
  the next `readable()`/`writable()` wait hung until something else
  poked the fd -- seen as `tests/peek.rs` stalling about 1 run in 60,
  and the plausible cause of similar rare hangs in dependents. Each
  direction's readiness word now packs an edge counter next to the
  ready bit; callers snapshot it before the syscall and the clear is a
  compare-and-swap that only lands if no edge arrived since. When it is
  refused, the bit stays set and the operation is simply retried.
  `AsyncFdReadyGuard::clear_ready` now follows real tokio's rule: an
  event that arrived after the guard was created survives the clear.

- `TcpStream::connect`/`connect_addr`, `TcpSocket::connect`, and
  `UnixStream::connect`/`connect_addr`/`UnixSocket::connect` now wait for
  a still-in-flight non-blocking connect to actually resolve before
  returning. They used to register the socket with the reactor's
  optimistic "already writable" default, so the `SO_ERROR` check that
  decides whether the connect succeeded ran immediately, saw no error
  yet, and handed back a stream that was still mid-handshake. Linux
  masked it (a loopback `connect(2)` reports `EINPROGRESS` but has
  already processed the handshake or RST inside the call, so the
  premature check saw a settled `SO_ERROR`); on Windows, where the
  result is genuinely still pending when `connect` returns, a refused
  loopback connect never surfaced at all (Rusty-Mill/rusty_mill#137). The platform `connect` helpers now report
  established-vs-in-progress, and an in-progress connect is registered
  write-pending (`reactor::InitialReadiness`) so the first writability
  wait is real. New `tests/tcp_connect_refused.rs` covers refused and
  successful connects on every CI platform.
- The `epoll` and `kqueue` backends now publish an fd's `ScheduledIo` in
  the reactor registry *before* arming the kernel registration, not
  after. Both report the fd's current readiness immediately and, being
  edge-triggered (`EPOLLET`/`EV_CLEAR`), exactly once -- so an event the
  reactor thread dequeued before the registry entry existed was dropped
  and never re-reported. The optimistic default had hidden that (a lost
  initial edge only cost one wasted syscall); with write-pending
  registration it was a permanent hang on a connect whose first edge was
  lost, seen as a rare stall of the new tests under parallel load. The
  `io_uring` and Windows backends already registered in this order.

## [0.2.0] - 2026-08-02

### Breaking

- `sync::Semaphore::acquire`/`acquire_many`/`acquire_owned`/`acquire_many_owned`
  now return `Result<SemaphorePermit, AcquireError>` (previously infallible).
  `try_acquire`/`try_acquire_many`/`try_acquire_owned`/`try_acquire_many_owned`
  now return `Result<SemaphorePermit, TryAcquireError>` (previously `Option`).
  This widening is what makes the new `close`/`is_closed` below possible at
  all. ([#122](https://github.com/baileyrd/rusty_tokio/issues/122))

### Added

- `sync::Semaphore::close`/`is_closed`: closing a semaphore wakes every
  queued waiter with `AcquireError` and fails every subsequent
  `acquire`/`try_acquire` call the same way, without disturbing permits
  already held. ([#122](https://github.com/baileyrd/rusty_tokio/issues/122))
- `io::uring_global_driver()`: the only public way to obtain a real,
  production `Arc<dyn OpDriver>` (io_uring-fs feature) from outside this
  crate -- exposes the existing process-wide singleton `UringFile::open`/
  `create` already use internally, rather than adding a second way to
  construct an `IoUringDriver`.
  ([#256](https://github.com/baileyrd/rusty_tokio/issues/256))
- Generic BSD (FreeBSD/OpenBSD/NetBSD/DragonFly) reactor and socket
  support, alongside the existing macOS backend -- the `kevent` reactor
  is unchanged (kqueue doesn't differ across the family), built on
  rustils' `platform-bsd` (widened from macOS-only `platform-macos`) for
  the socket layer. `UnixStream::peer_cred`/`UCred` stay Linux/macOS-only
  for now -- peer-credential retrieval genuinely diverges per BSD and no
  verified implementation exists yet for any of them.
  ([#116](https://github.com/baileyrd/rusty_tokio/issues/116))

### Fixed

- `Cargo.toml`'s `rusty_std` dependency is now a pinned `git` dependency
  instead of a `path` dependency, so this crate can actually be built as a
  git dependency from outside this repo's own multi-repo dev checkout.
  Requires the matching fix in `rusty_std` itself (its own `rusty_libc`/
  `rusty_win32` path deps converted the same way).
  ([#254](https://github.com/baileyrd/rusty_tokio/issues/254))
