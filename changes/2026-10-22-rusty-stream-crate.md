---
category: Added
changelog: New `rusty_stream`: an in-house `Stream` trait, combinators, `stream!`/`try_stream!` and an unbounded channel, replacing `futures`/`futures-util`/`futures-core`/`async-stream` (#896).
---
## 2026-10-22 - rusty_stream: the in-house Stream crate

- **Added:** `crates/libs/async/rusty_stream` (with `rusty_stream-macros`) provides:
  - the `Stream` trait;
  - `StreamExt` (`next`, `map`, `filter`, `filter_map`, `then`, `buffered`, `take`, `collect`, `boxed`) and `TryStreamExt` (`try_next`, `map_ok`, `map_err`, `try_collect`);
  - `BoxStream`, `BoxFuture`, `stream::{iter, once, unfold, select_all}` and `future::{join_all, ready}`;
  - `channel::mpsc::unbounded`;
  - the `stream!`/`try_stream!` macros.

  This is the API that the 24 crates using `futures`, `futures-util`, `futures-core` or `async-stream` call (gap G7 in `docs/research/ASYNC-RUNTIME-INVENTORY.md`).
- **Design:**
  - The default build has no third-party dependencies and `#![forbid(unsafe_code)]`.
  - Adapters require `Unpin` streams and box the futures they hold, instead of pin projection.
  - The macros rewrite tokens with no `syn`. A `Yielder` passes each yielded value through a shared slot, with no thread-locals.
  - The `futures-core` feature adds `Compat<S>`, which converts in both directions for crates that speak `futures_core::Stream` (axum, reqwest, tonic).
- **Verified:** 20 integration tests cover every adapter, the channel (including a send after the receiver drops), both macros (`?`, a bare `return;`, closures with and without return types, nested `fn` items and nested macros inside the body), a `select_all` removal that must not skip a ready stream, a receiver drop with a queued value that owns a sender (no deadlock) and the compat round trip. Also clippy `-D warnings` and fmt on both crates. `cargo tree` shows that the default build pulls nothing outside the workspace.
- **Fixed (main integration):** regenerate the workspace map from locked all-features metadata so `rusty_tokio` counts the stream and `rusty_axum_tokio` dev dependents plus the landed `rk-app` and `rusty_term` dependencies (28 total).
- **Known limitations:**
  - Adapters need `Unpin` streams (use `.boxed()` or `Box::pin`).
  - `select_all` merges round-robin, not in `FuturesUnordered` order.
  - No workspace crate uses it yet; the family migrations (#908, #909, #910, #911) adopt it.
