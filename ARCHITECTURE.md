# Architecture

## Overview

`rusty_tokio` is a hand-rolled async runtime for Rust, built from scratch on
`std` — no `tokio`, no `mio`. It exists to actually understand how an async
runtime works, not to replace tokio. The scheduler, reactor, timer wheel, and
async sync primitives are all original code here.

Non-goals are stated in full at the bottom of this file; the short version is
that this is not a drop-in tokio replacement and does not chase its full
surface.

## Boundaries

The layering below follows `ATLAS-LAYER-0001` (higher layers depend on a lower
layer's declared interface, never its implementation detail) and
`ATLAS-DEP-0001` (depend toward stable abstractions rather than concrete
backends). The `platform` / `platform-linux` / `platform-bsd` /
`platform-windows` split this crate consumes from
[rustils](https://github.com/baileyrd/rustils) is the same port/adapter shape
`ATLAS-001` Chapter 21 uses as its worked example.

Each row names exactly one component responsible for translating across the
boundary, per `ATLAS-BOUND-0001`.

| Port | Adapter(s) | Notes |
| ---- | ---------- | ----- |
| `io::reactor` — readiness registration and wakeup | `epoll.rs` (Linux), `kqueue.rs` (macOS/BSD), `windows.rs` (IOCP + AFD), `io_uring.rs` (Linux, opt-in) | Selected at compile time by target, plus the opt-in `io-uring-reactor` feature. Everything above the reactor sees registration/readiness only, never the backend. |
| `io::socket` — socket lifecycle (bind/connect/accept/addressing) | `posix.rs` via rustils' `platform`/`platform-linux`/`platform-bsd`; `windows.rs` directly on `windows-sys` | The one asymmetric row. Two syscalls stay hand-rolled on POSIX because rustils' API can't express them yet; `tcp.rs`/`udp.rs` stay on the hand-rolled Windows layer rather than `platform-windows`. See `docs/decision-request-windows-process-signal-ipc.md`. |
| `io::{AsyncRead, AsyncWrite}` — this crate's own I/O traits | `tcp.rs`, `udp.rs`, `unix.rs`, `pipe.rs`, `stdio.rs`, `duplex.rs`, `simplex.rs` | The crate's central interface. `ATLAS-IFACE-0001` applies directly: these are consumed by both callers and implementers, so a new required method is breaking even when it looks additive. |
| `io::compat` — foreign async I/O traits | `futures_io::{AsyncRead, AsyncWrite}` for `Compat<T>` (opt-in) | Deliberately an external contract: the value is that crates written against `futures-io` accept our stream types unmodified. |
| `runtime` — task scheduling | `mod.rs` (multi-threaded work-stealing), `current_thread.rs` (single queue), `thread_per_core.rs` (pinned, no stealing) | Three flavors behind one `Runtime`/`Handle` surface. Per-worker queues and the injector are `crossbeam-deque`; the current-thread flavor keeps a plain `Mutex<VecDeque<_>>` since there is no stealing to speed up. |
| `runtime::blocking` — blocking work offload | `spawn_blocking` pool | `fs::` is built entirely on this. `io::UringFile` (opt-in) is the deliberate exception — see below. |
| `task::trace` — task instrumentation | `tracing::Span`, shaped to tokio's console wire format (opt-in) | Same posture as `io::compat`: the external format *is* the deliverable, so `console-subscriber`/`tokio-console` work unmodified. |

## Structure

A modular monolith, matching the `ATLAS-DEP-0010` default — one crate, narrow
internal module boundaries, no service extraction. The one split is
`rusty_tokio-macros`, and it is forced rather than chosen: a `proc-macro = true`
crate cannot export anything alongside its proc-macros, so `#[main]`/`#[test]`
cannot live in the main crate. This mirrors tokio's own `tokio`/`tokio-macros`
split and is the same forcing-function test `ATLAS-DEP-0010` asks for.

External dependencies are audited in [`dependency-audit.md`](./dependency-audit.md).
`libc` and `windows-sys` are a deliberate floor, consistent with rustils' RFC v2.

## Data flow

A `TcpStream::read` on the multi-threaded runtime:

1. A worker thread polls the task; the read hits `WouldBlock`.
2. `io::readiness` registers the fd's `Interest` with the reactor and parks the
   task's waker against that registration.
3. The worker finds no other ready task, so it tries to steal from a sibling's
   queue, then falls back to blocking in the reactor's wait call
   (`epoll_wait`/`kevent`/AFD poll).
4. The kernel reports readiness. The reactor maps the event back to the
   registration and wakes the stored waker.
5. The task is pushed back onto a run queue and re-polled; the read now
   completes against the same fd.

`io::UringFile` (opt-in) deliberately bypasses steps 2–4: an io_uring
read/write hands the kernel a raw pointer for the operation's whole duration,
so it uses owned buffers (`IoBuf`/`IoBufMut`/`BufResult`) rather than borrowed
ones. A future can be dropped mid-poll, and a borrowed-buffer API cannot keep
the caller's buffer alive across that. `fs::File` stays 100% `spawn_blocking`
for exactly this reason.

## Key decisions

See [docs/adr/](./docs/adr/) for the record of individual decisions and their
tradeoffs. Several load-bearing decisions currently live in issue threads and
long-form manifest comments rather than ADRs — notably the `crossbeam-deque`
adoption (#8), the io_uring scope limit (#9), and the Windows socket-layer
split. Migrating those into ADRs is outstanding.

## Non-goals

- **A drop-in tokio replacement.** Compatible in shape where that is cheap
  (`#[main]`, the console wire format), not in surface area.
- **Hand-rolling verified lock-free data structures.** `crossbeam-deque` is a
  deliberate exception to the from-scratch rule: a Chase-Lev deque is real
  unsafe concurrent code and this project has no `loom`-based verification set
  up to trust a new implementation. The integration tests that hold the
  scheduler/reactor/timer logic to that bar do not extend to it.
- **A full io_uring runtime.** The `io-uring-reactor` feature is readiness-only
  (`IORING_OP_POLL_ADD`); the actual read/write syscalls are unchanged.
  `io-uring-fs` is separate, positional-file-only, and opt-in.
- **A pausable clock, or multiple runtime flavors behind `#[main]` arguments.**
  See #56.
