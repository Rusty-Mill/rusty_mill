# rusty_mill — project notes for the proposed board

Super-board is deferred and unavailable in this checkout. The
[configuration proposal](proposed-config.md) is inactive documentation; neither
these notes nor any local tooling copy establishes automatic merge or migration
authority. See [deferred pre-flight](pre-flight.md).

- Cargo workspace monorepo of Rust crates, merged via `git subtree` under `crates/` (groups: apps, foundation, libs, platform, tools). See [README](../../README.md), [architecture](../../ARCHITECTURE.md), and [workspace map](../WORKSPACE-MAP.md).
- Toolchain pinned in `rust-toolchain.toml`; dependency policy in `deny.toml`.
- Validate the affected packages with `cargo check|test|clippy -p <crate>` and repository gates. The proposed `cargo check --workspace` board command alone does not satisfy required CI or review.
- Frozen: `crates/apps/nexus` (see its FROZEN.md) — excluded from workspace and CI; do not touch.
- Decisions live in [docs/adr](../adr/). [CONTRIBUTING](../../CONTRIBUTING.md) governs reviewed, green-CI PRs and merge commits through the merge queue.
- The Windows `PYTHONUTF8=1` note concerns deferred super-board scripts, not an installed prerequisite.
