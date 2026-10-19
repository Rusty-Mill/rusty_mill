---
category: Changed
changelog: "`rusty_acp`, `rusty_a2a`, `rusty_lsp`, the three `adk-*` crates and `rusty_tokio` reach `tracing` through the `rusty_log` facade. No behaviour change."
---
## 2026-10-19 - protocol, adk and runtime crates move onto rusty_log (tracing facade, step 2, third area)

- **Changed:** `rusty-acp`, `rusty_a2a`, `rusty_lsp`, `adk-graph`, `adk-mcp`, `adk-a2a` and `rusty_tokio` depend on `rusty_log` instead of `tracing` (and, for `rusty-acp`'s tests and examples, instead of a pinned `tracing-subscriber`, now the workspace one through the `subscriber` feature). All `tracing::` and `tracing_subscriber::` paths in their sources, tests and examples are `rusty_log::` and `rusty_log::tracing_subscriber::` (21 files). The facade re-exports the same crates, so no log line, span or filter changes.
- **Optional features:** `rusty_lsp` and `rusty_tokio` keep their `tracing` cargo feature, now `tracing = ["dep:rusty_log"]`, so nothing changes for a consumer that enables it, and a default build still carries no optional external crate. For `rusty_tokio` the one difference is that the optional dependency now comes in through the workspace `tracing` (default features, so `tracing-attributes` as well) instead of its own minimal `std`-only pin; the opt-in `tracing_instrumentation` test passes.
- **Toolchain floor:** `rusty_log` declared `rust-version = "1.88"`, which would have raised the declared floor of consumers such as `rusty_lsp` (1.85, with its `tracing` feature). Nothing in it needs 1.88: it only re-exports `tracing` 0.1 and `tracing-subscriber` 0.3, both 1.65, so it now declares 1.65. `rusty_lsp --features tracing` checks on Rust 1.85. Not affected by this change: the declared floors of `rusty-acp` (1.86), `adk-*` (1.85) and `inventory-core` (1.82) were already unmet on `main`, because third-party (`icu_*`) and workspace (`rusty_libc`, `rusty_rand`) dependencies need 1.88 or edition 2024.
- **Policy:** the seven crates leave `TRACING_BASELINE` in the same PR (15 crates left).
- **Verified:** clippy `-D warnings` and the full tests (all features, 149 test binaries) of the seven crates pass, with one exception that also fails on the unchanged base: `rusty_a2a`'s `pinned_client_connects_to_the_validated_address_not_a_fresh_dns_lookup` needs a network path the sandbox proxy blocks; CI runs it.
- **Not changed:** the other 15 baseline crates (apps and the tailscale family), which follow.
- **Depends on:** the gateway migration PR.
