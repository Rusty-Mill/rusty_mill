# Changelog

All notable changes to this repo are documented here.
Format: Added / Changed / Deprecated / Removed / Fixed / Security, newest first.

## [Unreleased]
### Changed
- `rusty-search-core` now depends on
  [`rusty_serde`](https://github.com/baileyrd/rusty_serde) instead of
  `serde`/`serde_json` - `Document`, `Query`, `VectorQuery`, `Sort`,
  `SearchRequest`, `Hit`, `SearchResults`, and `Schema`'s types all derive
  `rusty_serde::{Serialize, Deserialize}` now, and `Document::fields`
  changes type from `serde_json::Map<String, serde_json::Value>` to
  `rusty_serde::Value` (JSON wire shape unchanged). **Step 1 of an XL,
  ecosystem-wide migration** (#19) - every backend crate still depends on
  real `serde`/`serde_json` and expects the old `Document::fields` type, so
  the workspace does not build as a whole until they're migrated too in
  follow-up work. `rusty_serde` gained the `to_value`/`from_value` pair
  `Document::from_serializable`/`into_serializable` need as a direct
  prerequisite for this (`rusty_serde` #50). `Document::set` now uses
  `rusty_serde`'s `Value::insert` (`rusty_serde` #51/#52) instead of the
  hand-rolled find-or-push logic that gap originally forced.
- `rusty-search-sqlite-fts5` now depends on
  [`rusty_sqlite`](https://github.com/baileyrd/rusty_sqlite) instead of
  `rusqlite` directly - same `rusqlite` underneath (re-exported), but with
  connection pragmas (WAL journaling, foreign key enforcement, a busy
  timeout) this crate didn't set before, and a typed `Fts5TableBuilder` in
  place of a hand-assembled `CREATE VIRTUAL TABLE ... USING fts5(...)`
  string. From the `sovereignty-loop` dependency audit (#16); tracked in
  #17.

### Fixed
- Removed unused, unresolvable path dependencies (`rusty_regx`, `rusty_wire`,
  `rusty_tokio`, `rusty_std`, `rusty_json`, `rusty_request`) accidentally
  committed straight to `main` pointing at sibling directories that don't
  exist in this repo, which broke `cargo build`/`cargo metadata` for the
  entire workspace. None were referenced by any source file.

### Added
- `rusty-search-sqlite-fts5`: a `SearchBackend` backed by SQLite's FTS5
  virtual table module - embedded like `rusty-search-tantivy`, but via SQL
  rather than an inverted-index library, with native SQL sorting/filtering
  on every field type and no fast-field opt-in needed. Wired into the
  `rusty-search` facade behind a new `sqlite-fts5` feature flag and into
  the `pluggable_backends` example. (#14)
- `rusty-search-core`: `VectorQuery` and `SearchRequest::vector`, plus
  `SearchBackend::supports_vector_search()` (default `false`) - settles
  the `Query` DSL's vector/hybrid design question without implementing
  hybrid search anywhere yet. All eight pre-existing backends now
  explicitly reject a `SearchRequest` with `vector: Some(_)`. (#14)
- ADR-0008: SQLite FTS5 backend design, and why `Query` doesn't grow a
  vector-similarity variant (a separate, additive `SearchRequest::vector`
  instead). (#14)
- README "Planned backends" list under Status: candidate engines being
  considered for future adapter crates (Typesense, Quickwit, Manticore
  Search, Redis/RediSearch, SQLite FTS5, a managed enterprise search SaaS,
  and vector/hybrid search as a longer-term stretch). (#13)
- CI workflow (`.github/workflows/ci-rust.yml`): `cargo fmt --check`,
  `cargo clippy --all-targets --all-features -- -D warnings`, and
  `cargo test --all-features` on every PR and push to `main`. (#12)
- Initial `rusty_search` workspace: `rusty-search-core` (the `SearchBackend`
  trait, `Document`, `Schema`, a composable `Query` DSL,
  `SearchRequest`/`SearchResults`), `rusty-search-memory` (dependency-free
  in-memory backend), `rusty-search-tantivy` (embedded Tantivy backend,
  in-memory or on-disk), and the `rusty-search` facade crate with
  `memory`/`tantivy` feature flags. (#1)
- Repo governance docs: PR/issue templates, CONTRIBUTING, CODE_OF_CONDUCT,
  SECURITY, ARCHITECTURE, RELEASE_NOTES. (#1)
- ADR-0001: object-safe `SearchBackend` trait over a shared query DSL. (#3)
- `rusty-search-elasticsearch`: a `SearchBackend` for a remote
  Elasticsearch/OpenSearch cluster over HTTP, wired into the `rusty-search`
  facade behind a new `elasticsearch` feature flag. (#6)
- ADR-0002: Elasticsearch backend design (local index/field-type registry,
  client-side id generation, genuinely non-scoring `filter` clauses). (#6)
- `rusty-search-meilisearch`: a `SearchBackend` for a remote Meilisearch
  instance built on the official `meilisearch-sdk` crate, wired into the
  `rusty-search` facade behind a new `meilisearch` feature flag. (#7)
- ADR-0003: Meilisearch backend design (official SDK over hand-rolled
  HTTP, async task-waiting making `commit()` a no-op, single-full-text-query
  restriction). (#7)
- `rusty-search-opensearch`: a `SearchBackend` for a remote OpenSearch
  cluster, wrapping `ElasticsearchBackend` rather than reimplementing its
  translation logic, wired into the `rusty-search` facade behind a new
  `opensearch` feature flag. (#8)
- ADR-0004: OpenSearch backend as a thin wrapper around
  `ElasticsearchBackend` instead of an independent reimplementation. (#8)
- `rusty-search-solr`: a `SearchBackend` for a remote Apache Solr
  instance, an independent implementation translating `Query` into a
  Lucene query string plus `fq` filters, wired into the `rusty-search`
  facade behind a new `solr` feature flag. (#9)
- ADR-0005: Solr backend as an independent implementation rather than a
  wrapper, contrasted with ADR-0004's OpenSearch decision. (#9)
- `rusty-search-algolia`: a `SearchBackend` for the hosted Algolia search
  SaaS, hand-rolled over `reqwest`, wired into the `rusty-search` facade
  behind a new `algolia` feature flag. (#10)
- ADR-0006: Algolia backend design (hand-rolled HTTP, async task-waiting
  making `commit()` a no-op, single-full-text-query restriction, no
  "match everything" literal to ground `must_not` against). (#10)
- `rusty-search-azure-search`: a `SearchBackend` for the hosted Azure AI
  Search service, hand-rolled over `reqwest`, translating `Query` into a
  full-Lucene-syntax `search` string plus a separate OData `$filter`,
  wired into the `rusty-search` facade behind a new `azure-search`
  feature flag. (#11)
- ADR-0007: Azure AI Search backend design (hand-rolled HTTP, two
  independent query grammars in one request, `sortable` mirroring
  Tantivy's fast fields, synchronous writes making `commit()` a no-op for
  a different reason than Meilisearch/Algolia's). (#11)

<!-- ## [0.1.0] - YYYY-MM-DD
### Added
- Initial release -->
