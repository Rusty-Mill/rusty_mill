---
category: Changed
changelog: "`agentgateway-mcp`'s tests serve their scripted gRPC processor on `rusty_hyper` instead of `axum`, dropping its `axum`, `tokio`, `tokio-util` and `http-body-util` dev-dependencies (#875)."
---
## 2026-10-27 - agentgateway-mcp's tests leave axum

- **Changed:** the scripted `ExtMcp` processor in `guardrails/tests.rs` serves HTTP/2 through `rusty_hyper::serve_connection` on a `rusty_tokio` runtime of its own, behind its own accept loop (which still counts connections). The gRPC framing, the `grpc-status` trailer and the trailers-only failure reply stay hand-written, so the processor still shares no gRPC code with the `rusty_tonic` client under test. Stopping it cancels the accept loop, frees the port and sends each connection a GOAWAY, as the `axum` graceful shutdown did.
- **Dependencies:** `axum`, `tokio`, `tokio-util` and `http-body-util` leave `agentgateway-mcp`'s `[dev-dependencies]`; the first-party `rusty_hyper` joins them, through a new root `[workspace.dependencies]` entry. No third-party dependency is added.
- **Known limitations:** `agentgateway`'s own integration tests and `rusty_mcp_middleware`'s JWKS test still use `axum` as a mock server; they move in follow-ups, after which the root `axum` entry can be reconsidered (`rusty_a2a`, `rusty_acp` and `rusty_mcp_client_native` also use it).
