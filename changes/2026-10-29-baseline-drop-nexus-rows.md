---
category: Removed
changelog: rusty_baseline no longer lists the frozen nexus-cli and nexus-memory-hub; every products.txt row must resolve in the workspace.
---
## 2026-10-29 - Drop frozen Nexus rows from rusty_baseline

- **Removed:** the `nexus-cli` and `nexus-memory-hub` rows from `crates/tools/rusty_baseline/products.txt`. Both have been outside the workspace since Nexus was frozen (2026-10-07). The products test loses its `KNOWN_OUTSIDE_WORKSPACE` exemption, so every row must now resolve via `cargo metadata`. Follow-up from #1213's review.
- **Known limitations:** none.
