---
category: Changed
changelog: `rusty-mcp` drops `percent-encoding` (now `rusty_percent`) and `thiserror` (hand-written `Display` and `std::error::Error`); baggage values no longer escape `-_.~`.
---
## 2026-10-10 - rusty-mcp: percent-encoding and thiserror removed (sovereignty list item 1)

- **Changed:** `rusty-mcp` no longer depends on `percent-encoding` or `thiserror`. Baggage values are encoded and decoded with `rusty_percent`. The four error enums (`TokenError`, `AuthConfigError`, `JwtValidatorError`, `OtelError`) have hand-written `Display` and `std::error::Error` impls with the same messages and sources; `OtelError` keeps its `From<ExporterBuildError>`. No public type, variant or message changed.
- **Behaviour:** `to_header_value` no longer percent-encodes `-`, `_`, `.` and `~` (RFC 3986 unreserved; valid baggage either way). Every other byte is encoded as before.
- **Deviation from the plan:** `RUSTY-MCP-SOVEREIGNTY.md` item 1 named `rusty_err` for `thiserror`. Its derive implements `rusty_err::Error`, not `std::error::Error`, so adopting it would break `#[source]` and `?` in `agentgateway-auth` and `agentgateway`. Plain impls drop the dependency without that cost.
- **Verified:** `rusty-mcp` tests (all features), clippy `-D warnings` on `rusty-mcp` and the gateway crates, `cargo-shear`, `check_workspace_deps.py`, `check_workspace_layers.py`. New tests pin the error messages and sources and the baggage encoding; each fails with its change reverted.
- **Known limitations:** `rusty_percent::decode` and the old decoder agree on `+` and on invalid UTF-8 (both lossy), checked by reading, not by a differential test.
