---
category: Changed
changelog: rusty_term runs on rusty_tokio instead of tokio (TUI runtime and web bridge); tokio is no longer a dependency.
---
## 2026-10-23 - rusty_term: run on rusty_tokio

- **Changed:** the TUI runtime (`src/runtime`, its module renamed from `tokio_rt` to `reactor`) and the `web-bridge` server now run on the in-house `rusty_tokio` runtime, and `tokio` is dropped from rusty_term's dependencies. Behaviour is unchanged: same reactor-driven PTY and tty I/O on Unix, the same blocking-thread bridge on Windows, and the same ~60 Hz repaint coalescing.
- **Changed:** `rusty_tokio::select!` arms are plain expressions, so each loop now picks a small `Wake` event in the `select!` and acts on it in a `match` below.
- **Known limitations:** parking_lot is still a dependency. The Windows path is checked by cross-compilation only, not run.
