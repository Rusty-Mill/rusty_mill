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
- `rusty-search-memory` now depends on `rusty_serde` instead of
  `serde_json` - `eval.rs`/`sort.rs`'s field-value handling (term/range
  matching, field sort comparison) updated for `rusty_serde::Value`'s
  split `Int`/`UInt`/`Float` numeric variants and `Seq` (vs.
  `serde_json::Value`'s single `Number` and `Array`). Second crate of the
  migration tracked in #19, after `rusty-search-core`.
- `rusty-search-sqlite-fts5` now depends on `rusty_serde` instead of
  `serde_json` for its own `Document`/SQL-row conversion (`convert.rs`) -
  this crate never needed `serde_json::Value` for a third-party API
  contract (unlike `rusty-search-tantivy`'s `tantivy::Document::from_json_object`),
  so it's a straightforward swap. Third crate of the migration tracked in
  #19.
- `rusty-search-tantivy` now depends on `rusty_serde` for its own
  `Document`/`Query` handling, but keeps `serde_json` too -
  `tantivy::schema::document::TantivyDocument::from_json_object` is
  `tantivy`'s own API, hard-requiring literal `serde_json::Map`/`Value`
  since `tantivy` itself depends on real `serde_json`. `convert.rs` gains
  an explicit `rusty_value_to_json`/`json_value_to_rusty` conversion pair
  used only at that one boundary (`document_to_tantivy`/
  `tantivy_doc_to_document`), rather than threading `serde_json` through
  the rest of the crate. Fourth crate of the migration tracked in #19.
- `rusty-search-elasticsearch`, `-solr`, and `-algolia` follow the same
  pattern as `-tantivy`: each keeps `serde_json` alongside `rusty_serde`,
  since all three send/receive their `Document`s over HTTP through
  `reqwest`'s real-serde-backed `.json()`/`resp.json()` methods - the wire
  protocol, not one narrow API call like `tantivy`'s. Each crate's
  `convert.rs` gains the same `rusty_value_to_json`/`json_value_to_rusty`
  conversion pair, used only where a `Document` crosses that boundary;
  `rusty_serde::Value` is the type everywhere else that touches
  `Document`/`Query` (`compare_values` in each crate's `lib.rs`,
  `numeric_literal`/`coerce_range_bound`/`range_literal` in each crate's
  `query_map.rs`). Fifth, sixth, and seventh crates of the migration
  tracked in #19. `rusty-search-opensearch` (wraps `-elasticsearch`
  entirely - no `Document`/`Query` handling of its own, its one
  `serde_json` reference is an unrelated `#[cfg(test)]` helper) and
  `-cloud` (no `serde_json` at all) now build too, confirming neither
  needed a fix of their own - only `rusty-search-meilisearch` and
  `-azure-search` (and the top-level `rusty-search` facade, transitively)
  still block a full workspace build.
- `rusty-search-tantivy` and `rusty-search-sqlite-fts5` depend on
  [`rusty_time`](https://github.com/baileyrd/rusty_time) instead of
  `time` for RFC 3339 date parsing/validation on `Date`-typed fields.
  `rusty-search-tantivy::convert::parse_date` reconstructs a
  nanosecond-precision `tantivy::DateTime` via
  `tantivy::DateTime::from_timestamp_nanos` (`rusty_time` has no
  `time::OffsetDateTime`-shaped type for `DateTime::from_utc`). From the
  `sovereignty-loop` audit, completing the `time`/`uuid` follow-up;
  tracked in #23.
- All seven backend crates that generate client-side document ids
  (`rusty-search-algolia`, `-azure-search`, `-elasticsearch`,
  `-meilisearch`, `-solr`, `-sqlite-fts5`, `-tantivy`) depend on
  [`rusty_uuid`](https://github.com/baileyrd/rusty_uuid) instead of
  `uuid` - a genuine, complete sovereignty win (zero dependencies of its
  own), unlike the `rusqlite`→`rusty_sqlite` swap below. From the
  `sovereignty-loop` audit; tracked in #20.
- `rusty-search-sqlite-fts5` now depends on
  [`rusty_sqlite`](https://github.com/baileyrd/rusty_sqlite) instead of
  `rusqlite` directly - same `rusqlite` underneath (re-exported), but with
  connection pragmas (WAL journaling, foreign key enforcement, a busy
  timeout) this crate didn't set before, and a typed `Fts5TableBuilder` in
  place of a hand-assembled `CREATE VIRTUAL TABLE ... USING fts5(...)`
  string. From the `sovereignty-loop` dependency audit (#16); tracked in
  #17.

### Fixed
- CI: `actions/checkout@v4` now runs with `persist-credentials: false` in
  the `check` job. Its default github.com-wide auth header broke Cargo's
  fetch of the `rusty_serde` git dependency (pinned by commit, since it
  isn't on crates.io) - "failed to authenticate"/"revision not found"
  even though the commit was real and public, and unaffected by whether
  Cargo used its bundled libgit2 or `CARGO_NET_GIT_FETCH_WITH_CLI`'s
  system-git-CLI fallback (tried first; still failed under the same
  header, so not the actual fix). Blocked CI on every PR touching a crate
  migrated to `rusty_serde` (#19) until fixed.
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
