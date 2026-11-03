---
category: Changed
changelog: "Breaking: adk-mcp runs on rusty_tokio; mcp-tool-server no longer depends on tokio."
---
## 2026-11-03 - adk-mcp moves off tokio

- **Changed (breaking):** `adk-mcp` no longer depends on tokio (card #719, #908). Tool calls
  run on the `rusty_tokio` runtime the transport was started on, so call `serve_stdio`,
  `serve_http` and `router` from inside a `rusty_tokio` runtime. `serve_stream` now takes
  `rusty_tokio::io::AsyncRead`/`AsyncWrite` halves. The `stdio`, `http` and `client` features
  no longer turn on tokio features.
- **Changed:** the `mcp-tool-server` example runs on `#[rusty_tokio::main]` and drops tokio.
  Both transports were smoke-tested with a real `tools/call`.
- **Changed:** the version-negotiation tests now close the session and check that the server
  returns cleanly, instead of leaving it running when the runtime drops.
