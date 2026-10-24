---
category: Changed
changelog: `ts-control` speaks HTTP/2 to the Tailscale control server through `rusty_h2` instead of the crates.io `h2` crate.
---
## 2026-10-24 - ts-control: HTTP/2 over rusty_h2

- **Changed:** `ts-control`'s HTTP/2 session over the Noise channel uses `rusty_h2`'s client driver instead of crates.io `h2`, which leaves the crate's dependencies along with `bytes` and `http`.
  - The crate itself stays on tokio. `rusty_h2`'s driver needs only `rusty_tokio`'s I/O traits, not its runtime: a private adapter (`src/rusty_io.rs`) gives tokio streams those traits, and tokio spawns the driver.
  - Moving the whole crate to `rusty_tokio` stays with the rusty_tailscale family (#907), since `ts-engine` runs it on tokio.
- **Changed:** the HTTP/2 handshake timeout now covers the wait for the server's first SETTINGS frame, not just the client preface write. A peer that completes the Noise handshake but never speaks HTTP/2 still fails with `ClientError::Timeout`.
- **Changed (public API):** `ClientError::H2` wraps `rusty_h2::driver::Error` instead of `h2::Error`. The only dependent, `ts-engine`, wraps it with `#[from]` and needs no change.
- **Verified:**
  - ts-control's unit tests run against a `rusty_h2` server, and its Go `controlbase` interop tests pass.
  - A one-off run against Go's own `net/http` HTTP/2 server (h2c, Go 1.24) worked: a unary request, a streamed length-prefixed body, and a 404 mapped to `ClientError::HttpStatus`. Real control servers use that implementation.
- **Known limitations:** the Headscale end-to-end scripts under `interop/` need Docker and were not run.
