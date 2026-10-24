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
- **Unchanged:** `rusty-search-meilisearch` stays on tokio, because `meilisearch-sdk` runs on it (tier C).
- **Verified:** the family's tests pass with all features: 220, the same count as on tokio. A new `rusty_tokio` test covers the new impls. clippy (`-D warnings`) and fmt pass.
