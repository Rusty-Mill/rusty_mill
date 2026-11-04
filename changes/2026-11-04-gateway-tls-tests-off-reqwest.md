---
category: Changed
changelog: agentgateway's TLS suite drives the gateway with rusty_hyper and rusty_tls, and tokio and reqwest leave its dev-dependencies.
---
## 2026-11-04 - Gateway test suite free of tokio

- **Changed:** `agentgateway`'s `tls.rs` runs on `#[rusty_tokio::test]`. The HTTP/2 and ALPN
  checks use `rusty_hyper`'s client, which refuses a connection that did not negotiate `h2`, and
  the SNI checks speak TLS with `rusty_tls` to `127.0.0.1` under an invented hostname instead of
  `reqwest`'s `resolve`. `tokio` and `reqwest` are no longer dev-dependencies, so no
  `agentgateway*` crate has tokio, tokio-util, reqwest, rmcp, axum or hyper anywhere in its
  normal, dev or build graph (card #410, part of #686).
- **Known limitations:** the SNI checks send hand-written HTTP/1.1 rather than h2, since what
  they test is which certificate or route a name selects.
