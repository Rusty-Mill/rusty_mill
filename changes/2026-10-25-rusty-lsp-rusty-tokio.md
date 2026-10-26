---
category: Changed
changelog: rusty_lsp's runtime feature runs on rusty_tokio, and its public runtime signatures name rusty_tokio types (breaking, #911).
---
## 2026-10-25 - rusty_lsp on rusty_tokio (#911)

- **Changed (breaking):** `rusty_lsp`'s `runtime` and `tcp` features use `rusty_tokio` instead of crates.io `tokio`. `Server::stdio`, `Server::from_tcp`, `serve_tcp`, `transport::buffered` and the `testing` harness take or return `rusty_tokio` types and must run on a `rusty_tokio` runtime. Readers and writers passed to `Server::new` and `transport::{read_message, write_message}` must be `Send`. The `rusty-tokio` feature remains as an alias of `runtime`. The owner approved the break; no workspace crate uses the runtime features.
- **Added:** `rusty_tokio::sync::watch::Receiver::wait_for`, `AsyncBufRead` for `rusty_tokio::io::Take`, and `Debug` for `rusty_tokio::sync::Notify`, matching tokio.
- **Fixed:** `rusty_tokio::io::Stdout::flush` now flushes `std::io::Stdout`'s line buffer. It was a no-op, so bytes after the last newline (an LSP response body) stayed buffered until the next write or exit.
- **Fixed:** a dropped `rusty_tokio::sync::watch::Receiver::changed()` wait now removes its waker, and a re-polled wait keeps one. Before, each wait that lost a `select!` left a waker behind until the next send, so a long-lived loop such as `rusty_lsp`'s server grew them for the connection's lifetime.
- **Fixed:** `rusty_tokio::io::Stdout` and `Stderr` keep one operation slot, and switching between write and flush finishes an operation left in flight by a cancelled caller first. Before, a flush or shutdown after a cancelled write (and a write after a cancelled flush) could wait forever on the stream lock that the abandoned operation held.
