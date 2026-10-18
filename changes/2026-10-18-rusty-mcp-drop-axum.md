---
category: Changed
changelog: "**Breaking:** `rusty-mcp` is 0.11.0 and no longer depends on `axum` in production. `LimitsLayer`, `RequireAuthLayer` and `McpMetricsLayer` are generic over the response body instead of returning `axum::response::Response`; the gateway needed no change."
---
## 2026-10-18 - rusty-mcp 0.11.0: the tower layers drop axum (sovereignty item 6)

- **Why:** `axum` was the largest external dependency left in `rusty-mcp`, and only the three tower layers used it, for `Response`, `IntoResponse` and the body type of their refusals.
- **Change:** `Limited`, `RequireAuth` and `McpMetrics` now implement `Service<Request<_>, Response = http::Response<B>, Error = Infallible>` and return the same `http::Response<B>`. `Limited` and `RequireAuth` write a refusal (503, 504, 401, 403, 400), so they need `B: From<String>`; `McpMetrics` only observes the status, so it needs no bound. `axum::body::Body` satisfies `From<String>`, so mounting the layers on an `axum::Router` works as before.
- **Breaking:** a wrapped service whose `Response` is not an `http::Response` (an `IntoResponse` tuple, say) no longer fits; return `http::Response<B>` from it. `rusty-mcp` goes from 0.10.0 to 0.11.0. In this workspace the only consumer is the agent gateway (`agentgateway`, `agentgateway-auth`, `agentgateway-mcp`), which compiles unchanged and passes its tests.
- **Dependencies:** `axum` moves from `[dependencies]` to `[dev-dependencies]`, for the tests and doc examples that mount the layers on a router. Its entry is deleted from `PINNED_EXTERNAL_DEPENDENCIES` in `.github/scripts/check_workspace_deps.py`, which leaves six: `http`, `tower-layer`, `tower-service`, `tokio`, `tracing`, `tracing-subscriber`. No dependency is added or bumped.
- **Docs:** ADR-0002 Amendment 1, `MCP-NATIVE-PLAN.md` A6 and `RUSTY-MCP-SOVEREIGNTY.md` (item 6) record the result.
- **Verified:** `cargo clippy -D warnings` and the full tests of `rusty-mcp` (all features, doc tests) and of the three gateway crates; the in-crate tests of the layers now use `String` bodies, so they would not compile against an axum-only signature.
- **Not changed:** the tower pair, which stays until the gateway leaves axum; the other 12 axum crates.
