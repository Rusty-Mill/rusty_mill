# Decision request: Windows async process/signal/local-IPC

Status: **decided** (sign-off obtained in-session before implementation
started; recorded here per this crate's convention of not having its own
decision-request doc format — mirrors rustils' `docs/decision-request-*.md`
shape, since this repo has none of its own and no other governance model
(no RFC doc, no consumer-gate rule) beyond ordinary issue-driven review).

## Context

Before this change, `process`/`signal`/`UnixStream`/`UnixListener`/
`UnixDatagram` were `#[cfg(unix)]`-gated out of the crate entirely on
Windows (README, "Built on rustils" and the `UnixDatagram` bullet). This
closes that gap for `process`, `signal`, and `UnixStream`/`UnixListener`
(not `UnixDatagram` — see "Non-goals" below).

Two design forks had no single obviously-correct answer and were put to
the repo owner for sign-off before any code was written. Both were
resolved in favor of the smaller, precedent-matching option.

## Decision 1: child process stdio I/O model on Windows

**Chosen: `spawn_blocking`-backed, matching `fs::File`/`stdio` exactly.**

Confirmed directly in code before deciding: `src/io/reactor/mod.rs`'s
`RawIo`/`AsRawIo` types are hard-typed to `RawSocket` on Windows
(`#[cfg(windows)] pub(crate) type RawIo = std::os::windows::io::RawSocket;`).
The IOCP+AFD-poll reactor (`io/reactor/windows.rs`) is fundamentally
socket-only — it has no path to register an arbitrary pipe `HANDLE`. Genuine
non-blocking child-stdio I/O needs a second, structurally different,
completion-based mechanism (`OVERLAPPED` + `ReadFile`/`WriteFile` bound to
IOCP directly, tracked per-operation rather than per-handle-readiness-bit)
— comparable in scope to the IOCP reactor backend itself (issue #6's own
three-option design pass), not a small extension of `ScheduledIo`.

Given that, `ChildStdin`/`ChildStdout`/`ChildStderr` on Windows reuse the
exact shape `fs::File` already established for operations a reactor can't
drive: each wraps the real blocking `std::process::ChildStdin`/`Stdout`/
`Stderr`, dispatching each `poll_read`/`poll_write` to `spawn_blocking`. See
`process/mod.rs`'s Windows-arm doc comments for the state machine (a
`ReadState`/`WriteState` `Idle`/`Busy`/`Poisoned` enum per direction,
mirroring `fs::File`'s `State` — simpler here since each of the three types
only ever does one operation kind, unlike `File`, which must discriminate
between interleaved read/write/seek calls on the same value).

Cost, stated plainly: one parked blocking-pool thread per in-flight
read/write, not readiness-based concurrency. For a daemon's process
supervision use case (moderate numbers of children, not thousands of
concurrent high-throughput pipes) this is the right trade, and it's
consistent with what this crate already ships for `fs::File`/`io::stdin`/
`stdout`/`stderr`. True overlapped I/O remains a legitimate future
upgrade, scoped the same way issue #6 scoped the IOCP reactor itself
(needs its own design pass, its own real-hardware verification) — not
attempted here.

## Decision 2: which `SignalKind` surface exists on Windows

**Chosen: mirror tokio's own split, not a single generic surface.**

`signal::ctrl_c()` stays the one cross-platform entry point (`SIGINT` on
Unix, `CTRL_C_EVENT` on Windows). The generic `signal::signal(SignalKind)`
function and every *named* `SignalKind` constructor (`hangup`/`quit`/
`terminate`/`alarm`/`child`/`pipe`/`user_defined1`/`user_defined2`/
`window_change`) stay `#[cfg(unix)]` — they simply don't exist on Windows,
a compile error at the call site, not a silent `Unsupported` at runtime.
None of them have an honest Windows equivalent: Windows has no `SIGTERM`
analog at all (`CTRL_CLOSE`/`CTRL_SHUTDOWN` are a different, narrower,
best-effort-grace-period concept, not a signal a process can catch and
ignore indefinitely the way `SIGTERM` can be), and the rest (`SIGHUP`/
`SIGALRM`/`SIGCHLD`/`SIGPIPE`/`SIGUSR1`/`SIGUSR2`/`SIGWINCH`) have no
Windows concept whatsoever.

A new `#[cfg(windows)]`-only `signal::windows` submodule
(`ctrl_break`/`ctrl_close`/`ctrl_logoff`/`ctrl_shutdown`, each returning a
distinct listener type with its own `recv()`) covers the four
`SetConsoleCtrlHandler` events with no POSIX equivalent, matching
`tokio::signal::windows`'s own shape exactly (same four event names, same
per-kind-listener-type API).

**Structural shape reused, not reinvented**, per the brief's own guidance:
the self-pipe trick, ported to Windows as a synchronously-bootstrapped
loopback TCP pair (`127.0.0.1`, ephemeral port — Windows has no anonymous
`socketpair(2)`/`pipe(2)` equivalent usable with this crate's
socket-only reactor). `SetConsoleCtrlHandler`'s callback runs on an
ordinary OS-created thread, *not* under POSIX signal-handler restrictions
(no async-signal-safety list — it can allocate, lock, block), so it does a
plain blocking one-byte `send()` on the loopback pair's write half instead
of the `write(2)`-only self-pipe trick Unix needs; an ordinary spawned
task then reads the other half through the same reactor every socket in
this crate already uses (`ready_io`/`Interest::Read`), exactly mirroring
Unix's `reader_loop`.

## Decision 3 (discovered mid-implementation, not originally flagged): rustils' AF_UNIX Windows escape hatch is real but incomplete

The brief's contingency ("if rustils' Windows net layer needs its own
upstream gap filed first, that's expected, not a failure state") predicted
one shape of problem and found a narrower one.

**What's confirmed, by reading rustils' actual pinned commit
(`93b00ce964284d93ea6cec2581b3543f08df8f2d`), not this crate's own
(stale) comments about it:** `platform-windows` *does* have the
raw-handle + non-blocking escape hatch (rustils#59/PR #60, already
merged and included in the pinned rev) — `WindowsUnixStream`/
`WindowsUnixListener` (alongside `WindowsTcpStream`/`WindowsTcpListener`/
`WindowsUdpSocket`) implement `AsRawSocket`, `set_nonblocking`, and
concrete (non-`Box<dyn Trait>`) `connect`/`bind`/`accept` constructors —
the same shape `platform_linux`/`platform_bsd` already provide, and this
crate's own issue #6 thread (`baileyrd/rusty_tokio#6`) independently
confirmed the same thing on 2026-07-21, one day before shipping the
current hand-rolled `io/socket/windows.rs` TCP/UDP layer anyway, with a
closing comment that doesn't match its own thread's analysis. **This is a
real, pre-existing inconsistency in this repo, not something introduced
here** — flagged for the maintainer, not silently fixed: migrating
`tcp.rs`/`udp.rs` off the hand-rolled `windows-sys` layer onto
`platform_windows` is a legitimate follow-up, but it's a large, unrelated
refactor of already-shipped, already-tested (see "Verification" below)
code, out of scope for this change. `io/unix.rs`'s new Windows arm uses
`platform_windows::{WindowsUnixStream, WindowsUnixListener}` directly, as
the brief asked — the *first* thing in this crate to actually do so.

**What's still missing, found while wiring it up:** rustils#59 gave
`AsRawSocket` (borrow-only) but deliberately *not* `AsSocket`/
`FromRawSocket`/`IntoRawSocket`-style ownership-transfer adoption (its own
PR description: "not required for this issue's core ask"). Concretely,
there's no way to construct a `WindowsUnixStream`/`WindowsUnixListener`
from an already-open raw `SOCKET` the way `platform_linux::LinuxUnixStream`'s
`From<OwnedFd>` lets Linux/BSD hand-roll a non-blocking-before-connect
socket and adopt it after. This blocks exactly one thing: Unix's
`UnixStream::connect`/`connect_addr` pattern (create the socket
non-blocking *before* `connect(2)`, connect, then adopt) has no Windows
equivalent through `platform_windows` alone.

Two honest paths were available: (a) file the adoption gap upstream in
rustils (the same shape as rustils#41/#42/#59, scoped even narrower —
`From<OwnedSocket>`-equivalent for the five Windows socket types) and
block `UnixStream::connect` on it landing, or (b) use the escape hatch
that *does* exist today. Chosen: **(b) for now, with (a) recorded as the
concrete follow-up** — `UnixListener::bind`/`accept` (which never needed
adoption — `bind`/`accept` hand back the concrete type directly) use
`platform_windows` exactly like Linux/BSD, unchanged from the brief's
ask. `UnixStream::connect`/`connect_addr` dispatch rustils' own blocking
`WindowsUnixStream::connect(path)` through `spawn_blocking`, then flip
non-blocking and register with the reactor for every read/write after —
the same "one-time operation the reactor can't drive, spawn_blocking it,
then resume normal reactor-driven I/O" shape `fs::File::open`/`create`
already use, and consistent with Decision 1 above. `AF_UNIX` connect to a
local path has no real network RTT (unlike TCP), so the actual behavioral
cost of this is negligible in practice; it's flagged here because it's a
genuine, documented asymmetry with Unix's connect path, not because it's
expected to matter operationally. `UnixStream::pair()` (`socketpair(2)`)
and the bare pre-bind `UnixSocket` builder stay `#[cfg(unix)]`-only —
Windows has no anonymous `AF_UNIX` pair primitive at the OS level at all
(not a rustils gap, a real absence), and `UnixSocket::listen`/`connect`
hit the identical raw-adoption wall `UnixStream::connect` does, with no
`bind`/`accept`-shaped escape available the way `UnixListener` has.
`UnixSocketAddr` also has no honest Windows equivalent to Linux/Android's
abstract namespace (Windows `AF_UNIX` is pathname-only) — its Windows arm
wraps a plain `Option<PathBuf>` instead of `std::os::unix::net::SocketAddr`
(which doesn't exist on Windows at all), reusing this crate's own
pre-abstract-namespace representation; `from_abstract_name`/
`as_abstract_name` stay `#[cfg(any(target_os = "linux", target_os =
"android"))]`, unchanged.

`UnixListener`/`UnixStream` on Windows get `AsRawSocket` but not
`AsSocket`/`FromRawSocket`/`IntoRawSocket` — the same rustils-side
limitation (no owned-handle interop) applies there too. `TcpListener`/
`TcpStream`/`UdpSocket` keep full raw-handle trait parity on Windows
because they're this crate's *own* hand-rolled types with full control,
not `platform_windows`'s opaque ones.

## Non-goals reaffirmed

- `UnixDatagram` stays `#[cfg(unix)]`-only. rustils' `Net` trait has no
  `AF_UNIX` datagram support on *any* platform (not just Windows — see
  `io/unix_datagram.rs`'s own docs), and `std::os::windows::net` (the
  only other candidate, mirroring what `unix_datagram.rs` does on Unix)
  is nightly-only and unstable as of this writing (`windows_unix_domain_sockets`,
  rust-lang/rust#150487 — confirmed via the tracking PR, not assumed).
  Neither escape hatch exists yet; this is a real, separate,
  already-orthogonal gap, not something introduced or widened here.
- `TcpStream`/`TcpListener`/`UdpSocket` are untouched. See Decision 3.
- No Linux/macOS/BSD code touched.
- No io_uring-adjacent work.

## Verification

This session runs natively on `x86_64-pc-windows-gnu` (not the Linux
sandbox this crate's README describes as its usual dev environment) —
confirmed with `cargo build --all-targets` and a full `cargo test` run
*before* any change in this doc was implemented, which passed in full
(the pre-existing doctest failures are a sandboxed-tempdir `noexec`
artifact of this specific session, unrelated to any reactor/socket code).
That upgrades this change's own verification a full step past the
brief's stated baseline ("cross-compile type-checking only, mirroring the
existing Windows TCP/UDP reactor code's own caveat") — the new
process/signal/IPC code is verified with real `cargo build`/`cargo test`
execution on real Windows, not just `cargo check --target
x86_64-pc-windows-gnu`, and incidentally exercises the existing
TCP/UDP/IOCP+AFD reactor code for the first time on real hardware too
(bearing on issue #106, though closing that issue is outside this
change's scope).
