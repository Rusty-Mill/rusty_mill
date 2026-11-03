---
category: Changed
changelog: rp-mcp runs on rusty_tokio and drops tokio; rp-router's timers and spawns move to rusty_tokio.
---
## 2026-10-29 - rp-mcp off tokio

- **Changed (breaking):** `rp_mcp::native::register` and
  `rp_mcp::GatewaySource::new` take a `rusty_tokio::runtime::Handle` instead
  of a tokio one (owner-approved; rp-mcp is their only caller). `rp_mcp::build`
  is unchanged: it uses the calling task's rusty_tokio runtime, or the
  fallback runtime when called from tokio. rp-mcp has no tokio dependency,
  normal or dev (#910).
- **Changed:** `rp-router`'s retry and fusion timers, background spawns and
  SQLite `spawn_blocking` run on `rusty_tokio`, so the router can be driven
  from either runtime. Only the tokio-postgres connection task still needs
  the tokio runtime that opens Postgres persistence.
- **Known limitations:** `rp-router` keeps tokio for tokio-postgres; tests
  that need tokio's wiremock stay on `#[tokio::test]` and install the
  fallback runtime.
