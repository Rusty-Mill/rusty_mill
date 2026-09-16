//! The one blocking `UnixStream` type `client.rs`/`attach.rs` both use,
//! picked per platform.
//!
//! `std::os::unix::net::UnixStream` does not exist at all on Windows --
//! confirmed live in CI, not something a Linux-only build catches. On
//! Windows this crate uses `uds_windows` instead: a blocking
//! `UnixStream` with the same API shape (`connect`, `try_clone`,
//! `shutdown`, `Read`/`Write`), backed by WinSock's own `AF_UNIX`
//! support (Windows 10 1803+) -- the same address family
//! `sessionmgr-daemon` reaches through `rusty_tokio`'s own hand-rolled
//! WinSock layer, just via an existing crate instead of a second
//! from-scratch implementation. See `Cargo.toml`'s own comment on why
//! this crate cannot depend on `rusty_tokio` at all.

#[cfg(unix)]
pub use std::os::unix::net::UnixStream;
#[cfg(windows)]
pub use uds_windows::UnixStream;
