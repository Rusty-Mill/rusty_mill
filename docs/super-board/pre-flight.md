# Pre-flight

Needed before `super-board run` (from lint, 2026-10-14; issue #685).

## Credentials
- [✓] GitHub token with `repo`, `project`, `workflow` scopes (`gh auth status`)

## Tools
- [✓] `gh` CLI authenticated as baileyrd
- [✓] Rust toolchain 1.98.1 (`cargo`, `rustc`), pinned by `rust-toolchain.toml`
- [✓] Node (for `node --check` of the wave workflow)
- [✓] Python 3 with `PYTHONUTF8=1` set (Windows: the setup scripts crash on non-ASCII output without it)

## Environment
- [✓] Base branch `staging` exists on `origin`
- [✓] `crates/apps/rusty_tick` is a workspace member (Cargo.toml:94)
