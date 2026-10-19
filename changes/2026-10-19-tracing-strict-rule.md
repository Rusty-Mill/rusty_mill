---
category: Changed
changelog: "The `dependency policy` job now fails when any workspace crate other than `rusty_log` depends on `tracing` or a `tracing-*` crate directly. This closes the `tracing` facade work (sovereignty item 12)."
---
## 2026-10-19 - only rusty_log may depend on tracing (tracing facade, step 3)

- **Policy:** `TRACING_BASELINE` was empty after the last migration, so it is replaced by the strict rule it was heading for. `check_workspace_deps.py`'s `tracing_facade_violations` reports every workspace crate other than `rusty_log` that depends on `tracing` or a `tracing-*` crate, in any dependency kind, naming the crate and the dependency and saying to use `rusty_log` instead. The baseline, its both-ways stale-entry rule and their tests are deleted; 8 unit tests cover the strict rule, and removing the facade exemption or the `tracing-*` prefix match each makes a test fail.
- **Docs:** `TRACING-FACADE-PLAN.md` is marked done; sovereignty list item 12, ADR-0002 Amendment 1 and `MCP-NATIVE-PLAN.md` A6 record it.
- **Not changed:** the build graph. `axum`, `h2`, `sqlx`, `aws-*` and other third-party crates keep their own `tracing` dependency, so `tracing` stays in `Cargo.lock`; only the workspace's direct use is confined. Replacing it with an in-house implementation (option C) is not planned.
- **Depends on:** the tailscale migration PR, which empties the baseline.
