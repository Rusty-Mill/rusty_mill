---
category: Added
changelog: `rusty_tokio` resolves tokio's module paths (`net`, `net::tcp`, `net::unix`, `runtime`, `signal::unix`, `io::unix`, `task::spawn_blocking`) as re-exports of its own items, so a migration keeps its `use` lines.
---
## 2026-10-24 - rusty_tokio: tokio-compatible module paths (#897)

- **Added:** `rusty_tokio::net` re-exports the socket types from `io` under tokio's names. The TCP halves are under `net::tcp`. The Unix halves, `SocketAddr`, `UCred`, the uid/gid/pid aliases and `pipe::{pipe, Sender, Receiver, OpenOptions}` are under `net::unix`, unprefixed as in tokio. Also added: `signal::unix::{signal, Signal, SignalKind}`, `io::unix::{AsyncFd, AsyncFdReadyGuard, TryIoError}` and `task::spawn_blocking`. Each is the same item as its existing path, and every existing path still resolves (gap G1 in `docs/research/ASYNC-RUNTIME-INVENTORY.md`).
- **Changed:** the `runtime` module is now public, so `runtime::{Builder, Handle, Runtime, ...}` resolves. It exposes only the ten types already re-exported at the crate root. Its crate-internal items stay `pub(crate)`. The private `io::unix` module is renamed `io::unix_socket` to free the `io::unix` path.
- **Fixed:** `Runtime`'s docs no longer link to the private `RuntimeKind`, which removes six rustdoc warnings.
- **Verified:** `tests/tokio_paths.rs` checks every new path against its existing path with `TypeId`, runs `task::spawn_blocking`, and builds a current-thread runtime and a TCP round trip through `runtime::` and `net::` alone. Also verified: all of `cargo test -p rusty_tokio --all-features`, clippy `-D warnings`, fmt, and `cargo check` for `x86_64-pc-windows-gnu`.
- **Known limitations:** paths alone do not make a migration a pure rename. A trial rename of four G1-only crates still failed. The causes:
  - `TcpListener::bind` is sync and takes a `SocketAddr` (tokio's is `async` over `ToSocketAddrs`);
  - `ToSocketAddrs` is not implemented for `&String`;
  - `Builder::enable_all`, `UnixListener::from_std` and `ReadBuf::initialize_unfilled` are missing;
  - `AsyncRead` is not implemented for `std::io::Cursor`;
  - `rusty_http`'s transport is bound to tokio's I/O traits (#903).

  These are tracked separately.
