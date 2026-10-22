---
category: Changed
changelog: "Docs: the owner's answers to the `rusty_axum` stage 1 questions are recorded: `rusty_tokio` only, axum and `rmcp` dropped from dev-dependencies too, `agentgateway` drops axum only (#879)."
---
## 2026-10-22 - rusty_axum: owner answers to the stage 1 questions

- **Changed:** `docs/research/RUSTY-AXUM-SCOPE.md` §6.5 records the owner's answers. `rusty_axum` runs on `rusty_tokio`, with no `tokio` dependency. How consumers still on tokio libraries bridge (a per-poll tokio handle adapter in their own code) is recorded as a proposal. "Drop axum" includes dev-dependencies. `agentgateway` drops axum only; hyper and tower are tracked in #885 and #886.
- **Changed:** ADR-0002 Amendment 1 gets a status line: `rmcp` is to be replaced even as a dev-only wire-format oracle, so keeping it in `[dev-dependencies]` is an intermediate state.
- **Known limitations:** how MCP conformance is tested once the `rmcp` oracle is gone is not decided. Docs only; no code, dependency or default changes. Stage 2 has not started.
