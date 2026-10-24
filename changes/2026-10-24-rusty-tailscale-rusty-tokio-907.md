---
category: Changed
changelog: The rusty_tailscale crates run on `rusty_tokio` instead of tokio; `rusty_tokio` gains `AsyncFd::register`, `mpsc` `try_recv`/`poll_recv`/`error`, and a `lookup_host` that takes `&str`; `rusty_http` gains `async_tokio::Replay` (#907).
---
## 2026-10-24 - rusty_tailscale on rusty_tokio (#907)

- **Changed:** these ten crates depend on `rusty_tokio` instead of tokio, and none keeps a tokio dependency, normal or dev:
  - `ts-stun`, `ts-tun`, `ts-derp`, `ts-magicsock`, `ts-localapi`, `ts-cli`;
  - `ts-control`, now on `rusty_h2`;
  - `ts-engine`, `ts-net`, `ts-daemon`.

  The rest of the change:
  - **Mostly a crate rename:** `tokio::` becomes `rusty_tokio::`, and `rusty_http`'s `tokio_native` adapter becomes `async_tokio`.
  - **`Send` bounds:** generic I/O helpers gained `+ Send` bounds, because `rusty_tokio`'s `AsyncReadExt`/`AsyncWriteExt` futures need `Self: Send`. Those helpers are `ts-derp`'s `read_frame`/`write_frame` and `ts-control`'s `controlbase::{connect, connect_deferred}`.
  - **ts-control adapter removed:** the tokio adapter ts-control used to drive `rusty_h2` is gone.
  - **ts-localapi test:** its hyper interop test reaches hyper through a small safe adapter onto `hyper::rt`, in place of `hyper-util`'s tokio adapter.
  - **Docs:** the README, DESIGN and PROTOCOL now name `rusty_tokio`.
- **Added (`rusty_tokio`, tokio parity):**
  - `unsafe AsyncFd::register`/`register_with_interest` and `AsyncFdRegisterError`, the I/O-safe constructors tokio recommends over `new`;
  - `mpsc::Receiver::try_recv`/`poll_recv` (bounded and unbounded), `mpsc::error::TryRecvError`, and tokio's `mpsc::error` path for the channel errors.
- **Changed (`rusty_tokio`):** `lookup_host` takes this crate's `ToSocketAddrs`, as tokio's does, so a borrowed `&str` resolves; before, it needed a `'static` `std::net::ToSocketAddrs`. Every form workspace code passes still works.
- **Added (`rusty_http`):** `async_tokio::Replay`, the `rusty_tokio` twin of `tokio_native::Replay`, for bytes of the next protocol that arrived bundled with an upgrade response.
- **Verified:**
  - The family's tests pass: 70, the same count as on tokio, including the Go `controlbase` interop tests.
  - A one-off check against Go's `net/http` HTTP/2 server passed again on `rusty_tokio`.
  - New `rusty_tokio` tests cover `register`, `try_recv`, `poll_recv` and a borrowed `lookup_host`.
  - Every crate that depends on `rusty_tokio` or `rusty_http` still compiles.
- **Known limitations:**
  - The Headscale end-to-end scripts under `interop/` need Docker and were not run.
  - `rusty_tokio`'s extension-method futures still need `Self: Send`.
