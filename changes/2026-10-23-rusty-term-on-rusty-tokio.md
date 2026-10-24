---
category: Changed
changelog: rusty_term runs on rusty_tokio instead of tokio (TUI runtime and web bridge); tokio is no longer a dependency.
---
## 2026-10-23 - rusty_term: run on rusty_tokio

- **Changed:** the TUI runtime (`src/runtime`, its module renamed from `tokio_rt` to `reactor`) and the `web-bridge` server now run on the in-house `rusty_tokio` runtime, and `tokio` is dropped from rusty_term's dependencies. Behaviour is unchanged: same reactor-driven PTY and tty I/O on Unix, the same blocking-thread bridge on Windows, and the same ~60 Hz repaint coalescing.
- **Changed:** the render loops and web-bridge sessions choose their next event with a round-robin `fair_select` instead of `select!`. `rusty_tokio::select!` polls in fixed order, so a flooded output channel or a re-notified repaint could starve resize, shutdown and stats events. With `fair_select`, every ready source wins within one round. The loop picks a small `Wake` event and acts on it in a `match`.
- **Fixed:** the Unix runtime registers its own close-on-exec duplicates of the PTY master (`F_DUPFD_CLOEXEC`) instead of the raw number from the safe `BackendHandle::pty_fd`, so a backend that closes or reuses its descriptor can no longer leave the reactor polling, reading or writing an unrelated file. This is the same I/O-safety boundary as RUSTSEC-2026-0386.
- **Known limitations:** parking_lot is still a dependency. The Windows path is checked by cross-compilation only, not run.
- **Fixed:** regenerated the workspace map after merging main so the distinct `rusty_term`, `rusty_axum_tokio` and `rk-app` dependencies on `rusty_tokio` are all counted.
