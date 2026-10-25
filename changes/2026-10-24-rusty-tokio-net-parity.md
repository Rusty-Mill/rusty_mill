---
category: Changed
changelog: `rusty_tokio`'s `TcpListener::bind`, `UdpSocket::bind`, `UdpSocket::connect` and `UdpSocket::send_to` are now `async` over `ToSocketAddrs` like tokio's (breaking; `bind_addrs`/`connect_addrs` are removed). It also adds tokio's `ReadBuf` methods, `Builder::enable_*`, Unix `from_std`, and I/O impls for `Cursor`, `&[u8]`, `&mut T` and `Box<T>`.
---
## 2026-10-24 - rusty_tokio: tokio-shaped net constructors and in-memory I/O

- **Changed (breaking, owner-approved):** `TcpListener::bind`, `UdpSocket::bind` and `UdpSocket::connect` are `async fn`s generic over `ToSocketAddrs`, matching tokio. Before, they were synchronous and took only a `SocketAddr`.
  - Candidates are tried in order, and an IP literal skips the DNS lookup.
  - `UdpSocket::send_to` takes any `ToSocketAddrs` target.
  - The additive `bind_addrs`/`connect_addrs` they replace are removed.
  - Code that has to bind synchronously binds a `std` listener and adopts it with `from_std`, as with tokio.
  - Every in-workspace caller is updated:
    - `bind(..).unwrap()` becomes `bind(..).await.unwrap()`;
    - `"…".parse().unwrap()` arguments become the string itself;
    - synchronous test helpers use `std::net::TcpListener::bind` plus `from_std`.
- **Changed:** `UnixListener::bind` and `UnixStream::connect` take `impl AsRef<Path>`, as tokio's do. Before, they took `&Path`; existing callers still compile.
- **Added:**
  - `ToSocketAddrs` for every reference to a supported form (`&String`, `&SocketAddr`, ...), through a blanket `&T` impl as in tokio.
  - `Builder::enable_all`/`enable_io`/`enable_time`. These are no-ops, because every runtime already starts its reactor and timer.
  - `UnixListener::from_std` and `UnixStream::from_std` (Unix).
  - `ReadBuf::{initialize_unfilled, initialize_unfilled_to, put_slice, set_filled, filled_mut, clear}`.
  - `AsyncRead` for `&[u8]`.
  - `AsyncRead`/`AsyncBufRead`/`AsyncSeek` for `std::io::Cursor<T: AsRef<[u8]>>`.
  - `AsyncWrite` for `Cursor<&mut [u8]>`, `Cursor<&mut Vec<u8>>`, `Cursor<Vec<u8>>` and `Cursor<Box<[u8]>>`.
  - Forwarding `AsyncRead`/`AsyncBufRead`/`AsyncWrite`/`AsyncSeek` impls for `&mut T` and `Box<T>`.
- **Verified:**
  - `tests/net_parity.rs` (13 tests) covers every address form, hostname resolution, an unresolvable address, `from_std` for TCP and Unix, `AsRef<Path>`, the `enable_*` calls, the `ReadBuf` methods including their panics, and `Cursor` read/line/seek/write.
  - Generic code taking `&mut R` and `Box<dyn AsyncRead>`/`Box<dyn AsyncWrite>` compiles and runs.
  - `cargo test -p rusty_tokio --all-features`, clippy `-D warnings`, fmt, and `cargo check --all-targets --all-features` for all 28 crates that depend on `rusty_tokio`, plus a whole-workspace `cargo check --all-targets`.
- **Known limitations:**
  - The `AsyncReadExt`/`AsyncWriteExt` futures still require `Self: Send`, where tokio's are `Send` only when `Self` is. Generic code bounded only by `R: AsyncRead + Unpin` therefore still fails to compile and needs a `Send` bound.
  - `UnixListener::accept` still returns `Option<PathBuf>`, not tokio's `SocketAddr`.
  - `Handle::try_current` still returns an `Option`, not tokio's `Result`.
  - `rusty_http`'s transport is still bound to tokio's I/O traits (#903).
