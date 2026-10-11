---
category: Changed
changelog: `rusty-mcp` moves to 0.7.0: removing serde changed its public API (`VerifiedToken::claims`, `TraceContext::from_meta`/`apply_to`, `ProtectedResourceMetadata::to_value`), so it can no longer stay at 0.6.0.
---
## 2026-10-11 - rusty-mcp 0.7.0 (breaking, pre-1.0)

- **Changed:** version 0.6.0 to 0.7.0. The `serde`/`serde_json` removal (sovereignty list item 2) changed three public surfaces: `VerifiedToken::claims` and `with_claims` use `rusty_json::Value`; `TraceContext::from_meta` and `apply_to` take `&rusty_json::Map`; `ProtectedResourceMetadata` no longer implements `Serialize` and has `to_value()` instead. 0.6.0 was cut earlier the same day with the old API.
- **Consumers:** none outside this workspace (`publish = false`; the workspace dependency is a path with no version requirement). `agentgateway`, `agentgateway-auth` and `agentgateway-mcp` already compile against the new API on `main`.
- **Known limitations:** the bump is a version label only; it adds no code. Anyone pinning the git tag `v0.6.0` (see the crate's `CHANGELOG.md`) keeps the old API.
