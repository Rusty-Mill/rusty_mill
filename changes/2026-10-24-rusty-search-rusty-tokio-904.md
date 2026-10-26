---
category: Changed
changelog: The rusty_search crates run on `rusty_tokio` instead of tokio, and their HTTP backends use `rusty_request`'s default `rusty_tokio` backend; `rusty_tokio`'s `RwLock` and `Mutex` gain tokio's `Default` and `From<T>` (#904).
---
## 2026-10-24 - rusty_search on rusty_tokio (#904)

- **Changed:** these ten crates depend on `rusty_tokio` instead of tokio, and none keeps a normal or dev tokio dependency:
  - `rusty-search`, `rusty-search-memory`;
  - `rusty-search-algolia`, `rusty-search-azure-search`, `rusty-search-cloud`;
  - `rusty-search-elasticsearch`, `rusty-search-opensearch`, `rusty-search-solr`;
  - `rusty-search-sqlite-fts5`, `rusty-search-tantivy`.

  The rest of the change:
  - **Crate rename:** `tokio::sync::RwLock`, `tokio::time::sleep` and `#[tokio::test]`/`#[tokio::main]` become their `rusty_tokio` paths.
  - **HTTP backends:** they drop `rusty_request`'s `tokio` feature and use its default `rusty_tokio` backend. No crate outside the family depends on them.
  - **Docs:** the README example uses `#[rusty_tokio::main]`.
- **Added (`rusty_tokio`, tokio parity):** `Default` (for `T: Default`) and `From<T>` for `sync::RwLock` and `sync::Mutex`. `rusty-search-memory` derives `Default` over a `RwLock`.
- **Fixed (`rusty_tokio`):** `sync::RwLock` could deadlock, and cancelled waiters could strand others. Each waiter now keeps its place in one FIFO queue until it acquires or its future is dropped. The two failures were:
  - **A woken reader went back behind a later writer.** A writer held the lock, then a reader queued, then a writer queued. When the first writer dropped, only the reader was woken, and it re-queued behind the second writer, which was never woken: a deadlock.
  - **Cancelled waiters stayed in the queue.** A `write()` future dropped while queued kept holding back every later reader, and a dropped waiter could strand the waiters behind it.

  Now a queued reader waits only for a writer ahead of it. A queued writer acquires when it reaches the front, and a dropped waiter leaves the queue and wakes whoever that makes eligible.
- **Unchanged:** `rusty-search-meilisearch` stays on tokio, because `meilisearch-sdk` runs on it (tier C).
- **Verified:** the family's tests pass with all features: 220, the same count as on tokio. New `rusty_tokio` tests cover the new impls and, polling by hand in an exact order, the three `RwLock` cases above; each failed before the fix. clippy (`-D warnings`) and fmt pass.
