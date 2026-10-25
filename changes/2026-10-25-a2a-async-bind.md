---
category: Fixed
changelog: "`rusty_a2a`'s gRPC client transport tests build again after `rusty_tokio`'s `TcpListener::bind` became async (#996)."
---
## 2026-10-25 - rusty_a2a gRPC client tests build again on main

- **Fixed:** `tests/grpc_client_transport.rs` still called the synchronous `bind(SocketAddr)` that #996 replaced with an `async fn`. Its two synchronous setup sites now bind with std and adopt the listener with `TcpListener::from_std` inside the entered runtime. This is the caller #1033 does not cover.
