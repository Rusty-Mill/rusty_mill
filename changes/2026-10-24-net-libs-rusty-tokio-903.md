---
category: Fixed
changelog: `rusty_tokio`'s `TcpStream::poll_shutdown` succeeds on a connection the peer already reset, as tokio's does; the `h2` interop oracles in `rusty_h2` and `rusty_hyper` now run on `rusty_tokio` (#903).
---
## 2026-10-24 - Net libs: h2 oracles on rusty_tokio, shutdown parity (#903)

- **Fixed:** `TcpStream::poll_shutdown` (and so `AsyncWriteExt::shutdown`) in `rusty_tokio` now treats `NotConnected` as success, as tokio does. Before, shutting down a stream the peer had already reset failed. Under `h2`, that left a closed connection returning the same error from every later `accept`. A loop that ran until `accept` returned `None` then spun forever, and dropping the runtime under it hung.
- **Changed:** the live interop tests against the `h2` crate in `rusty_h2` and `rusty_hyper` run the oracle on a second `rusty_tokio` runtime, not on tokio. A small test-only adapter gives `rusty_tokio` sockets the `tokio::io` traits `h2` names. The `tokio` dev-dependency of both crates shrinks to `default-features = false, features = ["io-util"]`: trait definitions only, no runtime.
- **Unchanged:** the shipped builds of `rusty_http`, `rusty_request`, `rusty_h2` and `rusty_hyper` already used `rusty_tokio` or no runtime, with `tokio` an opt-in feature for consumers still on real tokio. `rusty_http` and `rusty_request` keep a `tokio` dev-dependency to test that feature.
- **Verified:** a new `rusty_tokio` test (`shutting_down_a_reset_stream_succeeds`) fails without the fix with `NotConnected` and passes with it. The `h2` graceful-shutdown interop test hung in 19 of 20 runs before the fix and passed 30 of 30 after it.
- **Known limitations:** crates.io `h2` is still a test-only oracle in these two crates, on purpose, and a shipped dependency of `ts-control`, which a follow-up ports to `rusty_h2`.
