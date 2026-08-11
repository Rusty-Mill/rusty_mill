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
