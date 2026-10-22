---
category: Changed
changelog: "ADR-0002 now records `rusty_mcp_axum` as an adapter that stays (Tier A) until the repo-wide axum decision, instead of a crate on its way out."
---
## 2026-10-22 - `rusty_mcp_axum` is Tier A until axum is decided (issue #875)

- **Changed:** ADR-0002 Amendment 1 gains a status line classifying `rusty_mcp_axum` as Tier A. Retiring it needs `rp-server`, `remind_me_remote`, `adk-mcp` and `agentgateway` off axum, and `rusty_serve` has no router, nesting or middleware to move them onto. No in-house replacement exists; options B and C in `RUSTY-AXUM-SCOPE.md` stay undecided.
- **No code change.**
