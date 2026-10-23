---
category: Changed
changelog: "ADR-0002 now records the decision to build `rusty_axum` and classifies `rusty_mcp_axum` as a transitional crate to delete once its consumers move onto it."
---
## 2026-10-22 - `rusty_axum` decided; `rusty_mcp_axum` is Tier T (issue #875)

- **Changed:** ADR-0002 Amendment 1 gains a status line: the workspace builds its own `rusty_axum`, and `rusty_mcp_axum` is Tier T (owner, seam, milestone stated) until `rp-server`, `remind_me_remote`, `adk-mcp` and `agentgateway` move onto it. `RUSTY-AXUM-SCOPE.md` records the decision (drop `axum` repo-wide, option C on `rusty_tokio`), marks its older "stay on A" recommendation superseded, and leaves server-loop feasibility and the first consumer open (#879).
- **No code change.**
