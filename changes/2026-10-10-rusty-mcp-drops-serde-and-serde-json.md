---
category: Changed
changelog: `rusty-mcp` drops `serde` and `serde_json` for `rusty_json`; `VerifiedToken::claims` and `TraceContext::from_meta`/`apply_to` take `rusty_json` types, and `ProtectedResourceMetadata` swaps its `Serialize` derive for `to_value`.
---
## 2026-10-10 - rusty-mcp: serde and serde_json removed (sovereignty list item 2)

- **Changed (breaking, in-workspace consumers only):** `rusty-mcp` no longer depends on `serde` or `serde_json`. `VerifiedToken::claims` and `with_claims` use `rusty_json::Value`. `TraceContext::from_meta` and `apply_to` take `&rusty_json::Map`. `ProtectedResourceMetadata` no longer implements `Serialize`; use `to_value()` (it omits empty lists and an unset `resource_documentation`, as before).
- **Behaviour:** the metadata document keeps its content but its keys are now alphabetical, not in declaration order. The 401 and 503 bodies keep their keys, their order (already alphabetical) and `content-type: application/json`; a test pins the single-key form, the two-key 503 body was checked by reading.
- **Gateway:** `agentgateway-auth` decodes claims into `rusty_json::Value`. `TokenClaims::from_json` (in `agentgateway-mcp`) converts once, where claims join the request; a failure answers 500 instead of dropping the claims. The rules engine and `agentgateway-llm` still read `serde_json::Value`.
- **Verified:** `rusty-mcp`, `agentgateway-auth`, `agentgateway-mcp` and `agentgateway` tests (all features), clippy `-D warnings` on the four crates, `cargo-shear`, `check_workspace_deps.py`, `check_workspace_layers.py`. New tests pin the whole metadata document, the response `content-type` and body, and the claims conversion across every JSON type; the first two fail with their change reverted.
- **Known limitations:** `serde` is still in `rusty-mcp`'s dependency tree through `jsonwebtoken` (removed in item 4) and `rusty_json`'s `serde` feature (needed until items 4 and 5). The claims conversion goes through a JSON string, so it costs one serialize and one parse per authenticated request; no benchmark was taken.
