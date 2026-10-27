---
category: Changed
changelog: "**Breaking:** `rusty-mcp` is renamed `rusty_mcp_middleware` (0.13.0) and moves to `crates/libs/protocol/rusty_mcp_middleware`; its API is unchanged apart from the crate path (#876)."
---
## 2026-10-27 - rusty-mcp is renamed rusty_mcp_middleware (#876)

- **Why:** since #886 the crate holds only the auth, trace and OpenTelemetry pieces that sit in front of an MCP endpoint, plus the blocking `rusty_serve` adapters, so `rusty-mcp` read as "the MCP crate" when the server and client are elsewhere. The owner chose the rename on #876. The crate also moves out of the old standalone layout (`rusty_mcp/crates/rusty-mcp`) to sit beside `rusty_mcp_proto`, `rusty_mcp_server` and `rusty_mcp_client_native`.
- **Changed:** the package is `rusty_mcp_middleware` 0.13.0, imported as `rusty_mcp_middleware::{auth, trace, otel, serve}`. Features (`jwt`, `otel`, `serve`) are unchanged. `agentgateway`, `agentgateway-auth` and `agentgateway-mcp` are updated, as are the root workspace entry, the `PINNED_EXTERNAL_DEPENDENCIES` key in `check_workspace_deps.py`, the crate's README and CHANGELOG (moved with it), the root README, ADR-0002 (a status line), the async-runtime inventory, `vendor/README.md`, three `deny.toml` comments and `docs/WORKSPACE-MAP.md`.
- **Unchanged on the wire:** the OpenTelemetry instrumentation scope stays `rusty-mcp` and the export thread stays `rusty-mcp-otlp`, so existing dashboards and collector filters keep matching.
- **Known limitations:** `rusty-mcp-client` and `rusty-mcp-demo` stay under `crates/libs/protocol/rusty_mcp/`, which now has a short README pointing here. Historical change fragments, `RELEASE_NOTES.md` and research notes keep the old name. The frozen nexus crates, outside the workspace, still name `rusty-mcp`.
