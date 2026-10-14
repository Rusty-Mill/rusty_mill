# rusty_mill — project notes for the board

- Cargo workspace monorepo of `baileyrd/*` Rust crates, merged via `git subtree` under `crates/` (groups: apps, foundation, libs, platform, tools). See README.md, ARCHITECTURE.md, docs/WORKSPACE-MAP.md.
- Toolchain pinned in `rust-toolchain.toml`; dependency policy in `deny.toml`.
- Verify: `cargo check --workspace` (board verify command); tests via `cargo test -p <crate>`.
- Frozen: `crates/apps/nexus` (see its FROZEN.md) — excluded from workspace and CI; do not touch.
- Decisions live in docs/adr/. Contribution rules: CONTRIBUTING.md.
- Windows host: set `PYTHONUTF8=1` when running super-board python scripts.
