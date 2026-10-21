---
category: Changed
changelog: "The provider (`rp-*`), homelab, board, runner, inventory and skillopt crates reach `tracing` through the `rusty_log` facade. No behaviour change."
---
## 2026-10-19 - the app crates move onto rusty_log (tracing facade, step 2, fourth area)

- **Changed:** `rp-router`, `rp-mcp`, `rp-server`, `rusty_homelab_mcp`, `rusty_mill_board`, `rusty_mill_runner`, `inventory-core` and `skillopt-cli` depend on `rusty_log` instead of `tracing` and `tracing-subscriber` (the binaries get the subscriber kit through `rusty_log`'s `subscriber` feature). All `tracing::` and `tracing_subscriber::` paths in their sources, tests and examples are `rusty_log::` and `rusty_log::tracing_subscriber::` (17 files). The facade re-exports the same crates, so no log line, span or filter changes.
- **Policy:** the eight crates leave `TRACING_BASELINE` in the same PR (7 crates left, all in the tailscale family).
- **Verified:** clippy `-D warnings` and the full tests (all features, 31 test binaries) of the eight crates pass.
- **Not changed:** the tailscale family, which is the last area.
- **Depends on:** the protocol, adk and runtime migration PR.
