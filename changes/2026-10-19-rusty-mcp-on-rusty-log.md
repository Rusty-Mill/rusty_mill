---
category: Changed
changelog: "`rusty-mcp` reaches `tracing` through the `rusty_log` facade instead of depending on `tracing` and `tracing-subscriber` itself. No behaviour or API change."
---
## 2026-10-19 - rusty-mcp moves onto rusty_log (tracing facade, step 2, first area)

- **Changed:** `rusty-mcp` now depends on `rusty_log` (feature `subscriber`) instead of `tracing` and `tracing-subscriber`; every `tracing::` and `tracing_subscriber::` path in its sources and tests is `rusty_log::` and `rusty_log::tracing_subscriber::`. The facade re-exports the same crates, so types are identical: the agent gateway composes `SpanLayer` with its own `tracing_subscriber` unchanged, and no log line or span changes. Public API is unchanged, so the version stays 0.11.0.
- **Policy:** `rusty-mcp`'s `tracing` and `tracing-subscriber` entries leave `PINNED_EXTERNAL_DEPENDENCIES` (four external dependencies remain: `http`, `tower-layer`, `tower-service`, `tokio`) and its row leaves `TRACING_BASELINE`, in the same PR as its manifest change, as the plan requires. `rusty_log` is a path dependency, so it is not on the external pin list. The workspace now has the `rusty_log` entry under `[workspace.dependencies]`.
- **Docs:** ADR-0002 Amendment 1, `MCP-NATIVE-PLAN.md` A6 and `RUSTY-MCP-SOVEREIGNTY.md` (items 10 and 12) record it.
- **Verified:** clippy `-D warnings` and the full `rusty-mcp` tests (all features, doc tests) and the three gateway crates' tests pass.
- **Not changed:** the other 30 baseline crates, which migrate one area at a time per `docs/research/TRACING-FACADE-PLAN.md`.
- **Depends on:** the PR adding the `rusty_log` crate and `TRACING_BASELINE`.
