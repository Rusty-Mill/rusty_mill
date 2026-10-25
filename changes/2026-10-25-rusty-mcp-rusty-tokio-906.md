---
category: Changed
changelog: "The rusty_mcp crates' tests and `rusty-mcp`'s JWKS cache move to `rusty_tokio` where nothing needs tokio; `rusty_tokio::sync::RwLock` implements `Debug` (#906)."
---
## 2026-10-25 - rusty_mcp on rusty_tokio where it can be (#906)

- **Changed (`rusty-mcp`):**
  - The `jwt` feature's JWKS cache uses `rusty_tokio::sync::RwLock`, which needs no runtime, instead of tokio's.
  - The unit tests that need no runtime, in `auth::{jwt, layer, token}`, run on `#[rusty_tokio::test]`, and so does the OTel shutdown-hook test.
- **Changed (`rusty-mcp-client`):** its tests run on `rusty_tokio`, apart from the OAuth token-fetch tests.
- **Kept on tokio, each with a comment saying why:**
  - **`rusty-mcp`'s `limits` middleware:** its timeout and retry sleep run on its consumers' tokio runtimes (`agentgateway`).
  - **`rusty-mcp-client`'s `rusty_request` `tokio` feature:** its HTTP calls run on its consumers' tokio runtimes (`adk-mcp`, `rk-mcp`, `rp-mcp`). `rusty_tokio`'s timer and I/O need a `rusty_tokio` runtime, so these move when the consumers do.
  - **Oracle tests:** these test against tokio-native oracles:
    - `axum` and `rusty_mcp_axum` behind `rusty-mcp`'s layers;
    - the `rmcp` client in `rusty-mcp-demo` and `rusty_mcp_server`'s interop tests;
    - the `rmcp` server in `rusty_mcp_client_native`;
    - `rusty_mcp_client_native`'s `async_client`, which checks the client on its consumers' executor.

    Those libraries already run on `rusty_tokio` or need no runtime.
- **Added (`rusty_tokio`, tokio parity):** `sync::RwLock<T: Debug>` implements `Debug` as tokio's does. It shows the value when a read lock is free and `<locked>` otherwise.
