# Architecture

## Overview

`rusty_libc` is a `#![no_std]`, zero-dependency, Linux-only crate that issues
raw syscalls via inline asm instead of linking prototypes against glibc. It
exists to replace the third-party `libc` FFI-bindings crate inside
[rush](https://github.com/baileyrd/rush), a shell, so rush's own binary no
longer depends on `libc`'s struct layouts and prototypes for the syscalls it
uses. It is not a general-purpose libc: the API surface tracks exactly what
rush needs (job control, terminal handling, filesystem ops, exec/fork), grown
through successive implementation-review rounds (see
[REVIEW.md](./REVIEW.md)), not a from-scratch reimplementation of POSIX.

## Boundaries

The one real port/adapter seam in this crate is **arch vs. everything else**:
`arch` is the only place that knows how to make a syscall on a given CPU
(`syscall0`..`syscall6`, `-errno` decoding); every other module is a thin,
safe wrapper translating a Linux subsystem's syscalls into a typed Rust API
over that port, never issuing `asm!` itself.

| Port | Adapter(s) | Notes |
| ---- | ---------- | ----- |
| `arch::syscall0..6` (raw syscall ABI) | `arch::x86_64`, `arch::aarch64` | Per-arch `core::arch::asm!` stubs behind one shared signature; this is the only `unsafe` asm in the crate. |
| Kernel struct layouts | `fd`, `fs`, `process`, `signal`, `wait`, `termios`, `tty`, `rlimit`, `time`, `umask` | Each module owns one Linux subsystem's structs/constants/wrappers, always the **kernel's** layout (checked by `const _:` size/offset assertions), never glibc's. |
| vDSO fast path | `vdso` (internal) | Optional, transparent speed-up for `time::clock_gettime`; falls back to the raw syscall on any resolution failure, so it is never a correctness dependency. |
| `std::io::Error` interop | `Errno`'s `std` feature impl | Opt-in bridge for consumers that work in `std::io::Result`; the `no_std` core has no knowledge of it. |

## Structure

This is a single small library crate, not a modular monolith of services —
there's no independent-scaling, team-boundary, or fault-isolation forcing
function here, so the "split into services" question doesn't apply. Within
the crate, the module boundary *is* the syscall subsystem boundary (one file
per kernel area: `fd.rs`, `fs.rs`, `process.rs`, `signal.rs`, `wait.rs`,
`termios.rs`, `tty.rs`, `rlimit.rs`, `time.rs`, `umask.rs`), which is the
natural cut for a syscall-shim crate and is kept flat rather than nested
further.

## Data flow

A typical call: consumer calls a safe wrapper (e.g. `fd::open`) → wrapper
builds the raw integer/pointer arguments and calls `arch::syscallN` → the
per-arch `asm!` stub traps into the kernel and returns the raw `usize` →
`arch::from_ret`/`from_ret_i32` decodes the `-4095..-1` error window into
`Result<T, Errno>` → the wrapper returns that typed result. No allocation,
no hidden buffering, no libc TLS `errno` — a raw `-errno` never escapes the
crate, and buffers are always caller-owned (see e.g. `fd::read`,
`fs::getdents64`).

## Key decisions
See [docs/adr/](./docs/adr/) for the record of individual decisions and their
tradeoffs, and [REVIEW.md](./REVIEW.md) for the fuller implementation-review
log this crate was actually built through.

## Non-goals

- **Not portable beyond Linux.** `cfg(target_os = "linux")` gates the whole
  crate; macOS/BSD have no stable syscall ABI to target this way and must
  keep going through their platform libc.
- **Does not remove glibc from a consumer's binary.** Rust `std` still links
  glibc on `*-linux-gnu`; this crate removes the third-party `libc` crate's
  FFI surface, not glibc itself. A truly glibc-free binary is a separate goal
  (static musl).
- **Not a general POSIX/libc reimplementation.** No networking, no threads,
  no dynamic loading, no `mmap`/allocator primitives — only what an
  interactive job-control shell's fork/exec/signal/terminal/filesystem path
  actually needs. See REVIEW.md's Round 3 for the currently-tracked gap list.
- **`fork`/`vfork` safety is deliberately narrow.** The raw `fork` here does
  not reset glibc's internal locks the way glibc's own `fork()` does; see
  `process::fork`'s doc comment for exactly when it's safe to call.
