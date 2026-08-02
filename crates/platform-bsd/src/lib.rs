//! # platform-bsd — the BSD backend (net-only slice, rustils#48/#86)
//!
//! Layering (RFC v2 §4.1, mirroring `platform-linux`): `ffi` (raw
//! bindings, curated in `ffi::libc_surface`) → `sys` (safe wrappers;
//! **all `unsafe` in this crate lives there**, each block with a
//! documented invariant) → the trait impls at the crate root, which
//! contain no `unsafe`.
//!
//! ## Scope: net only
//!
//! Forced by a real gap, not speculation: building `rusty_tokio`'s
//! kqueue reactor backend for macOS/BSD had no `platform` backend to sit
//! on, so its socket lifecycle (`src/io/socket/macos.rs`) got hand-rolled
//! against raw `libc` a second time — the exact duplication `platform`'s
//! Net slice already solved once for Linux. `Net`/`TcpStream`/
//! `TcpListener`/`UnixStream`/`UnixListener`/`UdpSocket` is therefore all
//! this crate implements; `fs`/`process`/`security`/`term`/`signals`
//! are out of scope until a consumer forces them the same way (RFC v2
//! §3), the same discipline every other surface in this workspace
//! follows.
//!
//! ## Scope: which BSDs
//!
//! Landed macOS-only (rustils#48) and widened to generic BSD in
//! rustils#86, when `rusty_tokio`#116 wanted the same kqueue reactor on
//! FreeBSD/OpenBSD and would otherwise have hand-rolled a *third* socket
//! lifecycle against raw `libc` — the very duplication #48 was filed to
//! stop. The gate is `macos`/`freebsd`/`openbsd`/`netbsd`/`dragonfly`:
//! every OS whose reactor story is `kqueue`, which is exactly the set of
//! consumers that would otherwise duplicate this code.
//!
//! `ios`/`tvos`/`watchos` are Darwin and would very likely compile
//! unchanged, but are deliberately excluded until a consumer asks (RFC
//! v2 §3) — no CI leg in this workspace could verify them, and an
//! unverified claim of support is worse than none.
//!
//! ## BSD vs. Linux: the three real syscall differences
//!
//! What this crate does differently from `platform-linux`, and why each
//! difference is safe on *every* BSD in the gate above rather than only
//! on the Darwin it was first written for:
//!
//! - **No `SOCK_CLOEXEC`/`SOCK_NONBLOCK` socket-type flags at
//!   `socket(2)`.** Darwin genuinely has neither; FreeBSD, OpenBSD,
//!   NetBSD and DragonFly all do. This crate takes the portable subset —
//!   `fcntl(F_SETFD, FD_CLOEXEC)` after creation — which is correct on
//!   all five. See "the cost of the portable subset" below.
//!   `set_nonblocking` (the rustils#41 escape hatch, ported here too)
//!   covers the `SOCK_NONBLOCK` half via `fcntl(F_SETFL)` the same way.
//! - **No `accept4(2)`.** Same shape: absent on Darwin, present on the
//!   other four. Plain `accept(2)` plus the same post-creation
//!   `fcntl(F_SETFD, FD_CLOEXEC)` on the returned fd is used throughout.
//! - **A leading length byte on every sockaddr variant** —
//!   `sin_len`/`sin6_len`/`sun_len`, which Linux's variants don't have.
//!   Unlike the first two this one is *universal* across the BSDs: it is
//!   the 4.4BSD `sockaddr` layout, which Darwin inherited along with
//!   everyone else. Handled by building each address via `zeroed()` +
//!   field assignment (`sys::net`) rather than a full struct literal, so
//!   the extra field never needs naming.
//!
//! ### The cost of the portable subset
//!
//! Taking the Darwin-compatible path on all five targets is a real
//! trade, not a free one: on the four BSDs that *do* have `SOCK_CLOEXEC`
//! and `accept4`, setting close-on-exec as a separate `fcntl` leaves a
//! window in which another thread's `fork`+`exec` can leak the fd into
//! its child. `sys::net::set_cloexec`'s own doc comment covers the race
//! in detail. It is accepted here for the same reason #48 accepted it
//! for macOS: `platform`'s process surface has no broader thread-safety
//! story today, one code path is easier to keep correct than two, and
//! closing the window would mean threading a per-OS `cfg` split through
//! every socket-creating function to buy an atomicity nothing in this
//! workspace currently depends on. If a consumer ever does, the split is
//! mechanical and the suites in `tests/` already pin the behavior it
//! would have to preserve.
//!
//! ## Verification status
//!
//! Not cross-compiled against a real macOS SDK from this Linux workspace
//! (no linker for it here) — that leg is `cargo check`/`clippy --target
//! x86_64-apple-darwin` plus a real `macos-latest` CI job, which is how
//! #48's own `AF_UNIX` divergence (see `sys::net::from_sockaddr_un`) got
//! caught in the first place. rustils#86 added the same real-OS gate for
//! the widened targets: FreeBSD and OpenBSD each run the full suite in
//! CI inside a VM, and `x86_64-unknown-freebsd`/`x86_64-unknown-netbsd`
//! get a fast cross-compile pre-check alongside the Windows one.
//!
//! That gate paid for itself on its first run, exactly as #48's did:
//! OpenBSD failed `bsd_unix_conforms` on a second, distinct `sun_path`
//! divergence — `getsockname` on a *bound* socket returning the path
//! with the rest of the buffer as NUL padding. Every static check
//! passed on that code. See `sys::net::from_sockaddr_un`. Widening the
//! gate on inference alone would have shipped it.
//!
//! DragonFly is the one target in the gate with neither: no prebuilt
//! `std` for `cargo check --target` (tier 3) and no CI runner. It is
//! included because its socket surface is FreeBSD's, but that is
//! reasoning by inheritance, not evidence — treat it as untested.

#![cfg(any(
    target_os = "macos",
    target_os = "freebsd",
    target_os = "openbsd",
    target_os = "netbsd",
    target_os = "dragonfly"
))]
#![deny(unsafe_code)] // opted back in, narrowly, inside sys/ modules only

pub mod ffi;
pub mod sys;

mod net;

pub use net::{BsdNet, BsdTcpListener, BsdTcpStream, BsdUdpSocket, BsdUnixListener, BsdUnixStream};
