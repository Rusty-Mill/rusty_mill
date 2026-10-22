---
category: Changed
changelog: `rk-constrain` runs on `rusty_tokio` instead of `tokio` (the #686 pilot); its approval channel is exported as `approval_channel()` and type aliases, and `rusty_tokio` channel endpoints now implement `Debug`.
---
## 2026-10-22 - rk-constrain moves to rusty_tokio (async runtime pilot)

- **Changed:** `rk-constrain` no longer depends on `tokio`. Its approval channel uses `rusty_tokio::sync::{mpsc, oneshot}`, and its tests run on `#[rusty_tokio::test]`.
- **Changed (in-workspace API):** `rk-constrain` exports `approval_channel(capacity)` and the `ApprovalTx`, `ApprovalRx` and `ApprovalResponder` aliases. `ApprovalRequest::respond` is now an `ApprovalResponder`. `rk-agui`, `rk-app` and the desktop shell build the channel through these exports, so none of them names a runtime crate. They still run on tokio and drain the channel there, because `rusty_tokio`'s channels need no `rusty_tokio` runtime.
- **Added:** `rusty_tokio`'s `mpsc`, `oneshot`, `broadcast` and `watch` senders and receivers implement `Debug`, matching tokio (gap G13, found by the pilot).
- **Verified:** tests for `rk-constrain` (21), `rk-agui`, `rk-app`, `rk-mcp`, `rk-kernel`, `rk-feed` and `rusty_tokio`; clippy `-D warnings` and fmt on the touched workspace crates. A new test pins the `Debug` output of every channel endpoint.
- **Known limitations:** the desktop shell (`rusty_key/desktop/src-tauri`, outside the workspace, not built by CI) was edited but not compiled here, because the container has no GTK. Its lockfile is unchanged and gains `rusty_tokio` on its next build.
