---
category: Changed
changelog: CI now tests and lints the Ai Env Manager desktop shell, which root jobs do not build.
---
## 2026-11-14 - CI: ai-env-gui job

- **Changed:** new `ai-env-gui (test, clippy)` job runs `cargo clippy -D warnings` and `cargo test --locked` in `crates/apps/ai_env_manager/gui`, installing WebKitGTK for the Tauri build. The planner runs it when that folder changes or when an `ai_env_*` package it path-depends on does, and `required-gate` waits for it. The crate is a standalone workspace (like `rusty_key`'s desktop shell), so no root job covered it.
- **Known limitations:** Linux only. No `rust-cache`: the repo reserves the cache quota for the workspace clippy and test targets (`test_ci_platforms`), so the job builds Tauri cold on each run.
