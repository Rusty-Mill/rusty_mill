---
category: Added
changelog: "`rusty_a2a`: `AgentServices::serve_grpc_on(listener)` serves the gRPC binding on a listener the caller already bound; the crate's gRPC tests use it, which ends their `AddrInUse` flake under parallel load."
---
## 2026-10-27 - rusty_a2a: serve gRPC on a bound listener

- **Why:** the gRPC tests picked a port by binding `127.0.0.1:0`, released it, and had `serve_grpc` bind it again later. Under parallel load another test could take the port in between, and the test failed with `AddrInUse` (1 of 150 runs of the ten gRPC test binaries, three at a time, on `main`). Two files worked around it with a 20-try retry loop and a 100 ms sleep per try.
- **Added (`rusty_a2a`):** `AgentServices::serve_grpc_on(std::net::TcpListener)` serves the gRPC binding on a listener the caller has already bound, so the port is known before serving and nothing can take it. `serve_grpc(addr)` now binds and calls it; its behaviour is unchanged.
- **Tests:** the ten gRPC test files bind first and pass the listener; the retry loops are gone. `dropping_serve_grpc_closes_the_listener` now waits for an answered call, not for the port to be held, to show the server took the listener over before it is dropped. The same stress run afterwards: 0 failures in 300 runs.
- **Known limitations:** `server::push::tests::pinned_client_connects_to_the_validated_address_not_a_fresh_dns_lookup` needs DNS and fails in the sandbox, as on `main`.
