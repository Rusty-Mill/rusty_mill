---
category: Fixed
changelog: "`rp-server` and `rusty_mcp_server`'s mount tests build again after `rusty_tokio`'s `TcpListener::bind` became async (#996)."
---
## 2026-10-25 - rp-server builds again on main

- **Fixed:** #996 made `rusty_tokio::io::TcpListener::bind` an `async fn` over `ToSocketAddrs` and removed `bind_addrs`. Two callers had merged in the meantime, `rp-server` (#1007) and `rusty_mcp_server`'s `tests/mount.rs` (#1006), so main stopped compiling there. `rp-server`'s `main` now awaits `bind(addr)`. Its integration tests and the mount tests register a std listener with `from_std` (rp-server) or await `bind` inside the runtime (mount).
