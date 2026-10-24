---
category: Changed
changelog: `rusty_stream` (the durable log server) is renamed `rusty_journal`, freeing `rusty_stream` for the async `Stream` crate (#895).
---
## 2026-10-22 - rusty_stream renamed to rusty_journal

- **Changed:** the single-node durable log server moves from `crates/libs/async/rusty_stream` to `crates/libs/async/rusty_journal`. The package, the binary, the `rusty_journal::` paths, the Dockerfile, the CI Windows excludes and the live docs follow. `rusty_tokio`'s `rusty_stream_segment_log_recovery` test is now `rusty_journal_segment_log_recovery`.
- **Why:** the old name read as the workspace's async `Stream` library, and #686 listed it as the replacement for `futures`/`async-stream`, which it is not. `rusty_log` was taken (the `tracing` facade). The `rusty_stream` name goes to the new `Stream` crate (#896).
- **Unchanged:** the wire protocol, the on-disk segment format and the `RUSTY_STREAM_ADDR`/`RUSTY_STREAM_DATA_DIR` environment variables, so existing deployments and data keep working. Historical records (archived release notes, ADRs, past reviews, links to the old standalone repository) keep the old name.
- **Verified:** `cargo test -p rusty_journal` (76 tests), the renamed `rusty_tokio` test, clippy `-D warnings`, `generate_workspace_map.py --verify`, `check_workspace_deps.py`, `check_workspace_layers.py`.
- **Known limitations:** the crate's leftover standalone `Cargo.lock` and `.github/` templates from before the monorepo merge still say `rusty_stream`. Cargo ignores them inside the workspace.
