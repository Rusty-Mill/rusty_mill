# Release Notes

No version tags yet — entries are tracked one per merged PR against `main`,
reverse chronological, each linking back to its PR.

---

## Issue #25 — Swap thiserror + anyhow for rusty_err across the whole workspace
**2026-08-12** · [#25](https://github.com/baileyrd/rusty_search/issues/25)

- **Changed:** `rusty-search-core` and all eight backend crates depend on
  [`rusty_err`](https://github.com/baileyrd/rusty_err) (pinned git
  dependency) instead of `thiserror`/`anyhow`. `SearchError` derives via
  `rusty_err::Error` - the same `#[error("...")]`/`#[from]` shape
  `thiserror` used, so every variant's message and `Serialization`'s
  `#[from] serde_json::Error` carried over unchanged. `Backend`'s field
  becomes `rusty_err::BoxError` (the `anyhow::Error` analog) instead of
  `anyhow::Error`, deliberately **without** `#[from]`/`#[source]` -
  `BoxError` doesn't implement `rusty_err::Error` itself by design, so it
  can't be used as a source field, matching `rusty_err`'s own test
  suite's shape. A new `SearchError::backend_msg(msg)` covers every
  `anyhow!("...")` ad-hoc-message call site (`rusty_err` has no
  `anyhow!`-style macro), backed by a small `BackendMessage` wrapper
  type. Added a `SearchError: Send + Sync` compile-time assertion test in
  `rusty-search-core`, since that's the exact property this swap hinged
  on.
- **From:** the `sovereignty-loop` audit, completing the last row of its
  original "immature candidate" bucket (`thiserror`/`anyhow`/`time`/
  `uuid` - `time` and `uuid` landed in #23/#20) now that both
  [`rusty_err#1`](https://github.com/baileyrd/rusty_err/issues/1) (derive
  macro + `BoxError`) and [`rusty_err#4`](https://github.com/baileyrd/rusty_err/issues/4)
  (`BoxError` made `Send + Sync`, found and reported after a probe
  crate's `assert_send::<SearchError>()` failed to compile against the
  first version) are merged upstream.
- **Not included:** `reqwest` → `rusty_request`, the remaining audit row.
  Investigated in detail (the API shape is a clean fit - raw-bytes
  body/response methods mean `rusty_search` can keep using real
  `serde_json` and use `rusty_request` purely as transport, and its error
  type is already `Send`/`Sync`-safe) but still blocked on dependency
  resolution: `rusty_tokio`/`rusty_tls`/`rusty_http` are `path`
  dependencies in `rusty_request`'s own `Cargo.toml`, the same bug class
  as the `rusty_time`/`rusty_std` issue fixed earlier. Filed as
  [`rusty_request#28`](https://github.com/baileyrd/rusty_request/issues/28).

## Issue #23 — Swap time for rusty_time in rusty-search-tantivy and rusty-search-sqlite-fts5
**2026-08-12** · [#23](https://github.com/baileyrd/rusty_search/issues/23)

- **Changed:** `rusty-search-tantivy` and `rusty-search-sqlite-fts5`
  depend on [`rusty_time`](https://github.com/baileyrd/rusty_time)
  (pinned git dependency) instead of `time`, for RFC 3339 date
  parsing/validation on `Date`-typed fields.
  `rusty-search-tantivy::convert::parse_date` uses
  `rusty_time::DateTime::parse` + `tantivy::DateTime::from_timestamp_nanos`
  in place of `time::OffsetDateTime::parse(&Rfc3339)` +
  `tantivy::DateTime::from_utc` - reconstructing a nanosecond-precision
  timestamp from `rusty_time::DateTime::timestamp()` (whole seconds) plus
  `.time().nanosecond()` (the sub-second remainder), since `rusty_time`
  has no `time::OffsetDateTime`-shaped type to hand to `from_utc`
  directly. `rusty-search-sqlite-fts5::convert::validate_date` is a
  simpler swap - just the parser, no conversion needed. Added a new test
  in each crate exercising a real `Date` field end to end (range query,
  exact-term query, and - for `sqlite-fts5` - a malformed-date rejection
  case), since neither crate's existing test suite touched `Date` fields
  at all.
- **From:** the `sovereignty-loop` audit, completing the `time`/`uuid`
  follow-up now that [`rusty_time#1`](https://github.com/baileyrd/rusty_time/issues/1)
  (RFC 3339 parser) and [`rusty_time#4`](https://github.com/baileyrd/rusty_time/issues/4)
  (standalone-git-dependency resolution) are both merged. Verified
  directly before wiring this in: a standalone probe crate depending on
  `rusty_time = { git = "..." }` builds clean, pulling in
  `rusty_std`/`rusty_libc`/`rusty_win32` transitively (all confirmed
  public, so no repo-visibility CI blocker like `rusty_sqlite`/`rusty_uuid`
  hit earlier).

## Issue #20 — Swap uuid for rusty_uuid across all seven backend crates
**2026-08-12** · [#20](https://github.com/baileyrd/rusty_search/issues/20)

- **Changed:** `rusty-search-algolia`, `-azure-search`, `-elasticsearch`,
  `-meilisearch`, `-solr`, `-sqlite-fts5`, and `-tantivy` all depend on
  [`rusty_uuid`](https://github.com/baileyrd/rusty_uuid) (pinned git
  dependency) instead of `uuid`, and call `rusty_uuid::Uuid::new_v4()` in
  place of `uuid::Uuid::new_v4()` wherever a `Document` without an id is
  indexed. `rusty_uuid` has **zero dependencies of its own** - a complete
  sovereignty win, not a partial one.
- **From:** the `sovereignty-loop` audit (`dependency-audit.md`), following
  up once [`rusty_uuid#1`](https://github.com/baileyrd/rusty_uuid/issues/1)
  (filed against the empty repo) was implemented and merged.
- **Not included:** `time` → `rusty_time`, the other half of this
  follow-up. `rusty_time` still depends on `rusty_std` via a `path`
  dependency, which breaks resolution as a standalone git dependency -
  confirmed by trying it (`error: no matching package named rusty_std
  found`). Filed as [`rusty_time#4`](https://github.com/baileyrd/rusty_time/issues/4);
  revisit once that's fixed.

## Issue #17 — Swap rusqlite for rusty_sqlite in rusty-search-sqlite-fts5
**2026-08-12** · [#17](https://github.com/baileyrd/rusty_search/issues/17)

- **Changed:** `rusty-search-sqlite-fts5` depends on
  [`rusty_sqlite`](https://github.com/baileyrd/rusty_sqlite) (pinned git
  dependency) instead of `rusqlite` directly. `rusty_sqlite` re-exports the
  same `rusqlite = "0.31"` (`bundled`) underneath - this doesn't eliminate
  the external crate, but consolidates it behind a wrapper that applies
  connection pragmas this crate didn't set before (WAL journaling, foreign
  key enforcement, a busy timeout) and offers a typed `Fts5TableBuilder`
  in place of a hand-assembled `CREATE VIRTUAL TABLE ... USING fts5(...)`
  string.
- **From:** the `sovereignty-loop` skill's dependency audit ([PR #16](https://github.com/baileyrd/rusty_search/pull/16),
  `dependency-audit.md`) - the one row classified as a tractable,
  low-risk swap. All 15 `rusty-search-sqlite-fts5` tests pass unmodified.
- **Not included:** the other audit rows (`serde`/`serde_json` →
  `rusty_serde`/`rusty_json`, `reqwest` → `rusty_request`, and the
  immature `rusty_err`/`rusty_time`/`rusty_uuid` candidates) - tracked as
  capability-gap issues on their respective repos instead, since none are
  viable drop-in swaps today.

## Issue #14 — Add a SQLite FTS5 backend, and settle the vector/hybrid-search design question
**2026-08-12** · [#14](https://github.com/baileyrd/rusty_search/issues/14)

- **Added:** `rusty-search-sqlite-fts5`, a new `SearchBackend` crate backed
  by SQLite's FTS5 virtual table module (via `rusqlite`'s bundled build -
  no system SQLite required). Embedded like `rusty-search-tantivy`, but
  every schema field gets a real, typed SQL column in a `content` table,
  so `Query::Term`/`Query::Range`/`Sort::Field` all work natively on any
  field type with no `fast: true` opt-in and no in-memory sort fallback -
  a real capability advantage over `rusty-search-tantivy`, not just
  another adapter with the same shape. Supports at most one `Query::Match`
  clause per search (its `bm25()`-derived score wires to exactly one FTS5
  join); a second is rejected with `SearchError::InvalidQuery`. Unlike
  `rusty-search-meilisearch`/`rusty-search-algolia`, `must_not` wrapping a
  bare `Query::MatchAll`/`Query::Match` needs no special-casing - plain
  SQL `NOT (...)` handles it directly. Wired into the `rusty-search`
  facade behind a new `sqlite-fts5` feature flag and into the
  `pluggable_backends` example (`SqliteFts5Backend::in_memory()`, no
  external service needed, so it always runs).
- **Added:** ADR-0008, settling the issue's second ask - whether `Query`
  grows a vector-similarity variant. Decision: no. `Query`'s nodes are
  boolean predicates; a k-NN similarity search is a continuous ranking
  signal with no natural must/should/must_not membership, and fusing it
  with lexical results is a backend-specific decision `Query`'s existing
  variants don't need to make individually. Instead, `rusty-search-core`
  gained a standalone `VectorQuery { field, vector, k }` type and an
  additive `SearchRequest::vector: Option<VectorQuery>` field (`None` by
  default, run alongside `query` rather than replacing it - matching
  `rusty_knowledge`'s stated need for *hybrid*, not vector-only, search),
  plus `SearchBackend::supports_vector_search() -> bool` (default
  `false`). All eight pre-existing backends now explicitly reject
  `request.vector.is_some()` with `SearchError::InvalidQuery` rather than
  silently ignoring it, matching this workspace's established "fail loud
  on an unsupported query shape" posture (ADR-0003).
- **Not included:** actually wiring `sqlite-vec` into
  `rusty-search-sqlite-fts5` for real hybrid search. That's real,
  separate work (a second native SQLite extension, plus a fusion-strategy
  choice) the issue itself flagged as "not urgent" - scoped out
  deliberately rather than attempted without the same chance to verify it
  against a real, embedded SQLite build that every other claim in this
  change got.
- **Fixed:** a direct, unrelated commit to `main` ("Antigravity Update")
  had added path dependencies (`rusty_regx`, `rusty_wire`, `rusty_tokio`,
  `rusty_std`, `rusty_json`, `rusty_request`) pointing at sibling
  directories that don't exist in this repo and aren't referenced by any
  source file, breaking `cargo build` for the entire workspace. Removed
  as a necessary prerequisite to developing and testing this change.

## PR #13 — Add a "Planned backends" list to README
**2026-07-21** · [#13](https://github.com/baileyrd/rusty_search/pull/13)

- **Added:** a "Planned backends" subsection under README's Status,
  listing candidate engines for future adapter crates, roughly in order
  of fit: Typesense and Quickwit as the strongest next picks (Typesense
  for its Algolia/Meilisearch-shaped REST API, Quickwit for being the
  distributed counterpart to the Tantivy engine `rusty-search-tantivy`
  already embeds), followed by Manticore Search, Redis/RediSearch, SQLite
  FTS5, and a managed enterprise search SaaS (Kendra/Vertex AI Search).
  Vector/hybrid search engines (Qdrant, Weaviate, Pinecone, Milvus) are
  flagged separately as a bigger undertaking, since none fit the current
  `Query` DSL without first deciding whether it grows a
  vector-similarity variant.
- No code changes - this is a planning list, not a commitment or a
  timeline.

## PR #12 — Add a CI workflow
**2026-07-21** · [#12](https://github.com/baileyrd/rusty_search/pull/12)

- **Added:** `.github/workflows/ci-rust.yml`, running `cargo fmt --all --
  check`, `cargo clippy --all-targets --all-features -- -D warnings`, and
  `cargo test --all-features` on every PR and on pushes to `main`. Applied
  via the `repo-config` skill's audit, which flagged this as the one gap
  left in an otherwise-complete governance-file set (10/10): the repo's
  "verify before committing" habit (`fmt`/`clippy`/`test` run by hand
  before every backend PR so far) had nothing enforcing it automatically.
- Known limitation, stated plainly: this is a single-job gate (format +
  lint + test), not a version/OS matrix or a publish pipeline - adequate
  for an internal repo at this stage, not a public-launch CI setup.
- For this check to actually gate merges, it still needs to be added as a
  required status check under branch protection - a manual follow-up,
  not something this PR can configure itself.

## PR #11 — Add an Azure AI Search backend
**2026-07-21** · [#11](https://github.com/baileyrd/rusty_search/pull/11)

- **Added:** `rusty-search-azure-search`, a `SearchBackend` for the hosted
  Azure AI Search service - hand-rolled over `reqwest` (like
  `rusty-search-elasticsearch`/`rusty-search-solr`/`rusty-search-algolia`),
  since no trustworthy async Azure AI Search Rust SDK exists on crates.io
  (see ADR-0007). Wired into the `rusty-search` facade behind a new
  `azure-search` feature, and into the `pluggable_backends` example
  (skipped gracefully without
  `RUSTY_SEARCH_AZURE_SEARCH_ENDPOINT`/`RUSTY_SEARCH_AZURE_SEARCH_API_KEY`
  set).
- **Added:** `Query` translation split across Azure's two independent
  query grammars in one request - a full-Lucene-syntax `search` string
  (`queryType: "full"`, as expressive as Solr's `q`: more than one
  `Query::Match`, `must_not` wrapping `Query::Match`, using the same
  grounding trick ADR-0005 established for Solr) plus a genuinely separate
  OData `$filter` for `Query::Bool::filter` (which rejects `Query::Match`
  - OData has no full-text primitive - but *does* support `must_not`
  wrapping a bare `Query::MatchAll` via OData's real `true`/`false`
  literals, a narrower boundary than Solr's full completeness but broader
  than Meilisearch/Algolia's).
- **Added:** `FieldOptions::fast` now does something in a remote backend
  for the first time - it maps onto Azure's `sortable` attribute, which
  (like a Tantivy fast field) must be declared at index-creation time
  before native `$orderby` sorting works. A `SearchRequest` sorting by a
  non-sortable field falls back to the same `FALLBACK_SORT_CAP`-bounded
  in-memory sort `rusty-search-tantivy`/`rusty-search-algolia` already
  use.
- **Added:** ADR-0007, documenting the hand-rolled-over-SDK choice, the
  two-grammar query design and its exact completeness boundary, the
  `sortable`/fast-field parallel, and why `commit()` is a no-op here for a
  different reason than Meilisearch/Algolia's (writes are synchronous
  with nothing to poll; Azure simply has no refresh/commit concept at
  all).
- Known limitations, stated plainly: the mandatory key field is always
  named `"id"` and Azure's character restrictions on key values aren't
  validated client-side; `Query::Range` is restricted to
  `I64`/`F64`/`Date` fields; no Azure Active Directory/managed-identity
  auth, only `api-key`.
- 41 new unit tests (28 pure translation tests + 13 mocked-HTTP
  integration tests); all passed alongside the existing 150 unit tests +
  3 doctests across the workspace. `cargo clippy` and `cargo fmt --check`
  are both clean.

## PR #10 — Add an Algolia backend
**2026-07-21** · [#10](https://github.com/baileyrd/rusty_search/pull/10)

- **Added:** `rusty-search-algolia`, a `SearchBackend` for the hosted
  Algolia search SaaS - hand-rolled over `reqwest` (like
  `rusty-search-elasticsearch`/`rusty-search-solr`), since no trustworthy
  async Algolia Rust SDK exists on crates.io (see ADR-0006). Wired into
  the `rusty-search` facade behind a new `algolia` feature, and into the
  `pluggable_backends` example (skipped gracefully without
  `RUSTY_SEARCH_ALGOLIA_APP_ID`/`RUSTY_SEARCH_ALGOLIA_API_KEY` set).
- **Added:** `Query` translation into a single free-text `query` string
  (at most one `Query::Match`, restricted via
  `restrictSearchableAttributes` - the same one-full-text-clause ceiling
  as `rusty-search-meilisearch`) plus a single `filters` expression
  string for everything else. Algolia's filter language nests
  `AND`/`OR`/`NOT` arbitrarily in one string like Solr's Lucene syntax,
  but - unlike Solr - has no "match everything" literal to ground a
  negative clause against, so `must_not` wrapping a bare
  `Query::MatchAll`/`Query::Match` is rejected the same way Meilisearch
  rejects it.
- **Added:** ADR-0006, documenting the hand-rolled-over-SDK choice, the
  async task-polling write model (making `commit()` a no-op, the same
  shape ADR-0003 established for Meilisearch), the dual write/read
  hostname design (and the `with_hosts` constructor that makes both
  collapsible for testing), the constant `1.0` relevance score (Algolia
  exposes no portable single score), and the client-side fallback sort
  reused from `rusty-search-tantivy` (Algolia's native answer to custom
  sort is replica indices, out of scope here).
- Known limitations, stated plainly: no native per-query field sort
  (falls back to the same `FALLBACK_SORT_CAP`-bounded in-memory sort as
  `rusty-search-tantivy`); no native relevance score (`Hit::score` is
  always `1.0`, though result *order* still reflects Algolia's actual
  ranking); `Query::Range` restricted to `I64`/`F64` fields; no
  multi-host failover; index-exists semantics are local-registry-only,
  same caveat as every other remote backend here.
- 27 new unit tests (17 pure translation tests + 10 mocked-HTTP
  integration tests); all passed alongside the existing 123 unit tests +
  3 doctests across the workspace. `cargo clippy` and `cargo fmt --check`
  are both clean.

## PR #9 — Add a Solr backend
**2026-07-21** · [#9](https://github.com/baileyrd/rusty_search/pull/9)

- **Added:** `rusty-search-solr`, a `SearchBackend` for a remote Apache
  Solr instance - an independent implementation (hand-rolled `reqwest`,
  like `rusty-search-elasticsearch`), not a wrapper, since Solr's REST API
  isn't wire-compatible with Elasticsearch's the way OpenSearch's is (see
  ADR-0005 for the contrast with ADR-0004's OpenSearch decision). Wired
  into the `rusty-search` facade behind a new `solr` feature, and into the
  `pluggable_backends` example (skipped gracefully without
  `RUSTY_SEARCH_SOLR_URL` set).
- **Added:** `Query` translation into a single Lucene query string (`q`)
  plus separate `fq` filter queries - Solr's own genuinely non-scoring
  filter mechanism. Because Lucene's syntax supports arbitrary boolean
  nesting in one string, this backend can represent the *entire* `Query`
  DSL, including cases `rusty-search-meilisearch` has to reject (more than
  one `Query::Match`, `must_not` wrapping a bare `Query::MatchAll`).
- **Added:** ADR-0005, documenting why this backend is independent rather
  than a wrapper (Solr and Elasticsearch don't share a wire protocol, so
  there's nothing to reuse), the alternatives considered (wrapping ES
  anyway, an SDK-based approach, Solr's newer JSON Request API, SolrCloud's
  Collections API), and the consequences (no code sharing with the ES
  backend despite conceptual similarity; most expressive backend in the
  workspace, not necessarily the most portable one).
- Known limitations, stated plainly: `create_index` only supports
  standalone Solr via the Core Admin API against the `_default`
  configset, not SolrCloud's Collections API; `Query::Match` compiles to a
  quoted phrase query (analyzed, not an OR-of-terms match the way
  Elasticsearch's `match` defaults to); `Query::Range` doesn't support
  `Keyword`/`Text`/`Bool` fields. Response parsing defensively checks for
  an embedded `"error"` object before trusting the HTTP status code,
  since Solr's status-code passthrough has historically been inconsistent
  across deployments - a safe default made without a live server to
  confirm against, tracked honestly as a judgment call.
- 30 new unit tests (20 pure translation tests + 10 mocked-HTTP
  integration tests); all passed alongside the existing 93 unit tests + 3
  doctests across the workspace. `cargo clippy` and `cargo fmt --check`
  are both clean.

## PR #8 — Add an OpenSearch backend
**2026-07-21** · [#8](https://github.com/baileyrd/rusty_search/pull/8)

- **Added:** `rusty-search-opensearch`, a `SearchBackend` for a remote
  OpenSearch cluster. Rather than duplicating
  `rusty-search-elasticsearch`'s request/response translation against an
  effectively identical wire protocol, `OpenSearchBackend` is a thin
  newtype wrapper delegating every method to an inner `ElasticsearchBackend`
  - see ADR-0004 for the full reasoning. Wired into the `rusty-search`
  facade behind a new `opensearch` feature, and into the
  `pluggable_backends` example (skipped gracefully without
  `RUSTY_SEARCH_OS_URL` set).
- **Added:** ADR-0004, documenting why this backend wraps rather than
  reimplements, the alternatives considered (a second independent
  implementation, a type alias, no dedicated crate at all), and the
  consequences of that choice (inherits the Elasticsearch backend's
  limitations wholesale; would need real logic of its own if OpenSearch's
  API ever meaningfully diverges).
- Known limitation, stated plainly: no AWS SigV4 request signing for
  Amazon OpenSearch Service, the most common managed deployment target.
  `OpenSearchBackend::with_client` accepts a pre-configured
  `reqwest::Client` as the interim escape hatch.
- 6 new unit tests, deliberately scoped to proving the delegation itself
  is correct (construction, request round trips, error mapping, basic
  auth) rather than re-covering `rusty-search-elasticsearch`'s own
  query/schema/document translation tests, which apply unchanged since
  the code path is identical. All passed alongside the existing 87 unit
  tests + 3 doctests across the workspace. `cargo clippy` and
  `cargo fmt --check` are both clean.

## PR #7 — Add a Meilisearch backend
**2026-07-21** · [#7](https://github.com/baileyrd/rusty_search/pull/7)

- **Added:** `rusty-search-meilisearch`, a `SearchBackend` implementation
  for a remote Meilisearch instance, built on the official
  `meilisearch-sdk` crate rather than hand-rolled HTTP (a deliberate
  departure from `rusty-search-elasticsearch`'s approach - see ADR-0003).
  Wired into the `rusty-search` facade behind a new `meilisearch` feature,
  and into the `pluggable_backends` example (skipped gracefully without
  `RUSTY_SEARCH_MEILI_URL` set).
- **Added:** ADR-0003, documenting why this backend uses the official SDK
  instead of hand-rolled HTTP, waits on Meilisearch's async task model
  internally (making `commit()` a no-op), and restricts `Query` trees to
  at most one `Query::Match` clause plus a filter-expression translation
  of everything else - Meilisearch's search API has exactly one free-text
  query string, unlike Elasticsearch's composable query DSL.
- Known limitation, stated plainly: a `Query` tree with more than one
  `Query::Match`, or a `must_not` wrapping a bare `Query::MatchAll`/
  `Query::Match`, is rejected with `SearchError::InvalidQuery` rather than
  approximated. `Query::Range` is restricted to `I64`/`F64` fields here
  (Meilisearch filter comparisons don't support date strings the way the
  other backends' range queries do), and `SearchResults::total` reflects
  Meilisearch's `estimatedTotalHits`, not a guaranteed exact count.
- 25 new unit tests (17 pure translation tests + 8 mocked-HTTP integration
  tests covering the task-polling lifecycle); all passed alongside the
  existing 59 unit tests + 3 doctests across the workspace. `cargo clippy`
  and `cargo fmt --check` are both clean.

## PR #6 — Add an Elasticsearch backend
**2026-07-21** · [#6](https://github.com/baileyrd/rusty_search/pull/6)

- **Added:** `rusty-search-elasticsearch`, a `SearchBackend` implementation
  that talks to a remote Elasticsearch/OpenSearch cluster over HTTP via
  `reqwest` (rustls, no OpenSSL dependency). Wired into the `rusty-search`
  facade behind a new `elasticsearch` feature, and into the
  `pluggable_backends` example (skipped gracefully unless
  `RUSTY_SEARCH_ES_URL` is set, since it's the first backend needing a live
  external service).
- **Added:** ADR-0002, documenting the Elasticsearch-specific design
  choices — a local index/field-type registry instead of round-tripping to
  the cluster per query, client-side id generation matching the other
  backends, and `Query::Bool`'s `filter` mapping onto a genuinely
  non-scoring Elasticsearch `filter` context (unlike the Tantivy backend,
  which has to approximate it).
- Known limitation, stated plainly: this backend's local registry only
  knows about indices it created itself - an index created by another
  client against the same cluster won't be visible to it. Test coverage is
  against a mocked HTTP server (`wiremock`), not a live cluster; a
  live-cluster smoke test is a reasonable follow-up, not yet done.
- 27 new unit tests (16 pure translation tests + 11 mocked-HTTP integration
  tests); all passed alongside the existing 32 unit tests + 3 doctests across the workspace.
  `cargo clippy` and `cargo fmt --check` are both clean.

## PR #1 — Build rusty_search: async, pluggable search interface for Rust
**2026-07-21** · [#1](https://github.com/baileyrd/rusty_search/pull/1)

- **Added:** the initial `rusty_search` workspace — `rusty-search-core` (the
  `SearchBackend` trait, `Document`, `Schema`, a composable `Query` DSL,
  `SearchRequest`/`SearchResults`), `rusty-search-memory` (a dependency-free
  in-memory backend), `rusty-search-tantivy` (an embedded
  [Tantivy](https://github.com/quickwit-oss/tantivy) backend, in-memory or
  on-disk), and the `rusty-search` facade crate gating each backend behind a
  feature flag (`memory`, `tantivy`).
- **Added:** repo governance scaffolding — PR/issue templates,
  `CONTRIBUTING.md`, `CODE_OF_CONDUCT.md`, `SECURITY.md`,
  `ARCHITECTURE.md` (boundary table filled in for the real
  core/memory/tantivy/facade split), and an ADR seed.
- Known limitation, stated plainly rather than left implied:
  `rusty-search-tantivy`'s native sort acceleration only covers a single
  `Sort::Field` on an `i64`/`f64` field created with `fast: true`. Sorting
  by a `Keyword`/`Text`/`Bool`/`Date` field, or by more than one key, falls
  back to an in-memory sort over a candidate set capped at
  `FALLBACK_SORT_CAP` (10,000 documents) — correct up to that cap, not
  beyond it.
- Known limitation: `TantivyBackend::on_disk` does not reopen indices that
  already exist on disk from a previous process — `create_index` always
  creates fresh segments and errors if the directory already holds one.
- 32 new unit tests + 3 doctests; all passed. `cargo clippy` and
  `cargo fmt --check` are both clean across the workspace.
