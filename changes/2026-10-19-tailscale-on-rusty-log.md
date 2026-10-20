---
category: Changed
changelog: "The tailscale crates (`ts-*`) reach `tracing` through the `rusty_log` facade, which empties `TRACING_BASELINE`. No behaviour change."
---
## 2026-10-19 - the tailscale crates move onto rusty_log; the baseline is empty (tracing facade, step 2, last area)

- **Changed:** `ts-daemon`, `ts-derp`, `ts-engine`, `ts-localapi`, `ts-magicsock`, `ts-net` and `ts-wg` depend on `rusty_log` instead of `tracing` (and, for `ts-daemon`, `tracing-subscriber`, through the `subscriber` feature). All `tracing::` and `tracing_subscriber::` paths in their sources are `rusty_log::` and `rusty_log::tracing_subscriber::` (7 files). The facade re-exports the same crates, so no log line, span or filter changes.
- **Policy:** the seven crates leave `TRACING_BASELINE`, which is now empty: no workspace crate depends on `tracing` or a `tracing-*` crate directly except `rusty_log`. The strict rule that replaces the baseline is the last step of `docs/research/TRACING-FACADE-PLAN.md`.
- **Verified:** clippy `-D warnings` and the full tests (all features, 16 test binaries) of the seven crates pass.
- **Depends on:** the app crates migration PR.
