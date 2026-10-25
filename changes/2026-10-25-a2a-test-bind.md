---
category: Fixed
changelog: "`rusty_a2a`'s gRPC client transport tests build again after `rusty_tokio`'s `TcpListener::bind` became async (#996)."
---
## 2026-10-25 - rusty_a2a gRPC client tests build again

- **Fixed:** #1014 merged two calls to `rusty_tokio::io::TcpListener::bind(addr).unwrap()` after #996 had made `bind` an `async fn`, so `tests/grpc_client_transport.rs` stopped compiling on main. A `local_listener` helper now registers a std listener with `from_std` under the runtime the test has entered.
