---
category: Fixed
changelog: `rp-server` builds again after #996: it binds with the async `TcpListener::bind` instead of the removed `bind_addrs`.
---
## 2026-10-25 - rp-server: build against the async TcpListener::bind

- **Fixed:** main stopped compiling `rp-server` once #996 (async `TcpListener::bind`, `bind_addrs` removed) and #1007 (`rp-server` on `rusty_axum`) merged in sequence; neither PR's own CI saw the other's change.
  - `src/main.rs` calls `bind(addr).await` in place of the removed `bind_addrs(addr).await`.
  - The synchronous test helper in `tests/http_endpoints.rs` binds a `std` listener and adopts it with `TcpListener::from_std`, as #996 did for every other synchronous caller.
- **Verified:** with these two changes, `cargo check --workspace --all-targets` passes on main. That check excluded the 6 crates that need GTK, D-Bus or ALSA. `cargo nextest run -p rp-server --all-features` passes all 147 tests, and clippy (`-D warnings`) and fmt pass.
