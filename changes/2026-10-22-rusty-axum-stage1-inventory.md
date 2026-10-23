---
category: Changed
changelog: "Docs: `RUSTY-AXUM-SCOPE.md` records the stage 1 inventory of the 13 axum consumers and a measured yes on `rusty_tokio` driving a many-connection accept loop (#879)."
---
## 2026-10-22 - rusty_axum stage 1: consumer inventory and accept-loop check

- **Changed:** `docs/research/RUSTY-AXUM-SCOPE.md` §6 lists what each of the 13 axum consumers uses, read from source: router, extractors, responses, middleware and tower layers, SSE, server. No consumer uses WebSocket or multipart. `agentgateway` is a hyper server that uses axum only for a body type and the MCP mount.
- **Changed:** the accept-loop question is answered yes by measurement: one `rusty_tokio` accept loop on 2 workers held 8,000 concurrent keep-alive connections and served 160,000 requests with a constant thread count.
- **Known limitations:** every consumer's handlers depend on `tokio` (`reqwest`, `sqlx`, `tonic`, timers), and tokio calls panic inside a `rusty_tokio` task. §6.5 asks the owner whether `rusty_axum` gets a `tokio` driver as a migration bridge. Docs only; no code, dependency or default changes.
