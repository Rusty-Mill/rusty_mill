//! # rusty_libc
//!
//! A `#![no_std]`, zero-dependency, Linux-only crate that replaces the `libc`
//! FFI-bindings crate for [rush](https://github.com/baileyrd/rush) by issuing
//! raw syscalls via inline asm instead of linking prototypes against glibc.
//!
//! See `DESIGN.md` for the required API surface, hard problems, phasing, and
//! testing strategy.
//!
//! ## Conventions
//!
//! - Safe wrappers return [`Result<T, Errno>`](arch::Errno); a raw `-errno`
//!   return value never escapes the crate. Callers needing a
//!   `std::io::Error` should use `Error::from_raw_os_error(errno.0)` — this
//!   crate does **not** write glibc's TLS `errno`, so `last_os_error()` is
//!   not meaningful after these calls.
//! - `unsafe` is confined to the [`arch`] module (asm) and the pointer-taking
//!   ioctl internals; the public API is safe except where inherently not.
//! - All struct layouts are the **kernel's**, checked by `const _:`
//!   size/offset assertions, never copied from glibc headers.

// Build as `no_std` normally, but let the built-in test harness (and the
// optional `std` feature, which enables `std`-interop impls) pull in `std`.
#![cfg_attr(not(any(test, feature = "std")), no_std)]
// The entire crate is Linux-only; on other targets it compiles to nothing.
#![cfg(target_os = "linux")]
// `offset_of!` in const assertions needs a recent compiler; it is stable.
#![allow(clippy::missing_safety_doc)]

pub mod arch;
pub mod dns;
pub mod epoll;
pub mod eventfd;
pub mod fd;
pub mod fs;
pub mod inotify;
pub mod mmap;
pub mod process;
pub mod ptrace;
pub mod rand;
pub mod rlimit;
pub mod signal;
pub mod socket;
pub mod sysinfo;
pub mod termios;
pub mod time;
pub mod tty;
pub mod umask;
pub mod wait;

// Internal: vDSO symbol resolution backing the `time` fast paths.
mod vdso;

pub use arch::{from_ret, Errno};
