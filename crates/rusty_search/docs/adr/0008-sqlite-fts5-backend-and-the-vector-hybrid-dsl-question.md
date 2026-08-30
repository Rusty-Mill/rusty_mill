# ADR-0008: SQLite FTS5 backend design, and settling the vector/hybrid `Query` DSL question

Status: Accepted
Date: 2026-08-12

## Context
The README's "Planned backends" list (added in #13) named SQLite FTS5 as
the most direct next embedded backend to add - "genuinely embedded like
`rusty-search-tantivy`, but via SQL virtual tables rather than an
inverted-index library" - and separately flagged vector/hybrid search as a
bigger undertaking blocked on a design question: does `Query` grow a
vector-similarity variant, or does hybrid search need something else
alongside `SearchBackend::search`? [rusty_knowledge#14](https://github.com/baileyrd/rusty_search/issues/14)
tied the two together: `Rusty-Mill/rusty_knowledge` hand-rolls its own
FTS5 queries today and has an unused `sqlite-vec` `vec0` table waiting to
be wired into *hybrid* (lexical + vector, not vector alone) retrieval,
tracked as its own `rusty_knowledge#18`. Both needs are exactly the kind
of backend-specific query-building logic this workspace's pluggable
`SearchBackend` trait exists to absorb, so this ADR covers both: the new
backend crate, and the DSL question its existence forces an answer to.

## Decision

### `rusty-search-sqlite-fts5`
- New crate, `rusqlite` (`bundled` feature - no system SQLite required,
  same zero-external-dependency spirit as `rusty-search-tantivy`) plus
  FTS5 virtual tables. One SQLite connection per index (in-memory via
  `SqliteFts5Backend::in_memory()`, or one `<dir>/<name>.sqlite3` file per
  index via `SqliteFts5Backend::on_disk(dir)`), guarded by a
  `std::sync::Mutex`, mirroring `TantivyBackend`'s constructor shape and
  on-disk-doesn't-reopen limitation exactly.
- Each index is two SQL objects sharing `rowid` values: a `content` table
  with one real, typed column per schema field, and - only when the
  schema has `Text` fields - an `idx_fts` FTS5 table shadowing them.
  Because `content`'s columns are ordinary typed SQL columns,
  `Query::Term`/`Query::Range`/`Sort::Field` all translate to plain SQL on
  *every* field type, with no `fast: true`/fast-field opt-in and no
  `FALLBACK_SORT_CAP`-bounded in-memory sort - a real capability
  advantage over `rusty-search-tantivy` this ADR is glad to record, not
  just a limitation to disclose.
- At most one `Query::Match` clause is supported per search - the same
  restriction ADR-0003 gave `rusty-search-meilisearch`, for a related but
  distinct reason: this backend's `bm25()`-derived score comes from a
  single `LEFT JOIN (SELECT rowid, bm25(idx_fts) ... WHERE idx_fts MATCH ?)`
  subquery computed once per search, and a full-text clause's *filtering*
  position in the `Query` tree is expressed as `scores.rowid IS NOT NULL`
  referencing that same join - both wired to exactly one MATCH parameter.
  Supporting a second, independently-scored `Match` clause would mean a
  second join and a real score-fusion decision (sum? max? something
  weighted?) this ADR isn't trying to settle for lexical-only search.
  A second `Query::Match` anywhere in the tree is rejected with
  `SearchError::InvalidQuery` rather than approximated, consistent with
  this workspace's established preference (ADR-0003) for an honest error
  over a silently wrong answer.
- Unlike `rusty-search-meilisearch`/`rusty-search-algolia`, `must_not`
  wrapping a bare `Query::MatchAll`/`Query::Match` needs no special
  restriction here: it compiles to a plain `NOT (...)` around whatever SQL
  fragment the wrapped query produced (`NOT (1)`, `NOT (scores.rowid IS
  NOT NULL)`), which is always well-formed SQL. Nothing about SQL's
  boolean algebra runs into the gap those two backends' bespoke filter
  languages hit.
- User-supplied `Query::Match` text is translated into an FTS5 `MATCH`
  expression by quoting every whitespace-separated token as its own
  string literal (`"title" : ("foo" "bar")`), never splicing raw input
  into FTS5's own query syntax - closing off `AND`/`OR`/`NOT`/`-`/`*`/
  column-filter injection through search input, the SQL-injection-shaped
  risk this backend's design otherwise invites more directly than any
  remote-HTTP backend in this workspace.
- `commit()` is a no-op, for the same reason as `rusty-search-meilisearch`
  (ADR-0003) once its task-waiting completes: `index_batch`/`delete` each
  already ran inside their own transaction and committed by the time they
  returned, so there's nothing left to flush.

### The vector/hybrid `Query` DSL question
- **`Query` does not grow a vector-similarity variant.** `Query`'s
  existing nodes are boolean predicates - a document matches a
  `term`/`range`/`bool` clause or it doesn't - composed with
  `.and()`/`.or()`/`.not()`. Vector similarity is a continuous ranking
  signal with no natural must/should/must_not membership of its own:
  forcing it into the same tree would conflate "does this document match"
  with "how do we fuse two independently-computed rankings together" (the
  real question hybrid search asks, e.g. via reciprocal rank fusion), and
  that fusion strategy is backend-specific in a way `Query`'s existing
  variants deliberately aren't.
- Instead, `rusty-search-core` gained a small, additive, backward-compatible
  extension: a standalone `VectorQuery { field, vector: Vec<f32>, k: usize }`
  type, and `SearchRequest::vector: Option<VectorQuery>` (attached via a
  new `.vector()` builder method, defaulting to `None`) - run *alongside*,
  not instead of, `SearchRequest::query`. This directly answers
  `rusty_knowledge`'s stated need: hybrid means lexical FTS5 *and* vector
  similarity together, not a vector-only mode.
- `SearchBackend` gained a matching default method,
  `supports_vector_search(&self) -> bool { false }`, so callers can check
  support without a failed round trip. Every one of the eight backends
  that predate this method (`rusty-search-memory`, `-tantivy`,
  `-elasticsearch`, `-meilisearch`, `-solr`, `-algolia`, `-azure-search`;
  `-opensearch` inherits the check by delegating its `search` call to
  `ElasticsearchBackend`) now explicitly rejects `request.vector.is_some()`
  with `SearchError::InvalidQuery` at the top of `search`, rather than
  silently ignoring the field and returning an incomplete lexical-only
  answer with no indication anything was dropped - the same "fail loud"
  posture ADR-0003 established for unsupported query shapes, applied here
  to an unsupported *request* shape instead.
- `rusty-search-sqlite-fts5` - the backend `rusty_knowledge` would
  actually build hybrid retrieval on, via `sqlite-vec`'s `vec0` virtual
  table alongside `idx_fts` - does **not** implement vector search yet.
  It rejects `request.vector` the same as every other backend today.
  Wiring in `sqlite-vec` (a second native SQLite extension, on top of
  FTS5) and picking a fusion strategy is real, separate work this ADR
  scopes out rather than bundles in, tracked as a follow-up rather than
  attempted without a chance to validate it as thoroughly as this ADR's
  FTS5-only claims were (every claim about FTS5 behavior in this document
  was checked against a real, embedded SQLite build - see Consequences).

## Alternatives considered
- **Add `Query::Vector { field, vector, k }` as a new `Query` variant**,
  composable with `.and()`/`.or()`/`.not()` like every other node.
  Rejected: a vector clause inside `must_not` or nested under `should`
  alongside lexical clauses has no well-defined boolean meaning (what
  does "NOT within k-nearest-neighbors" filter to, precisely?), and
  backends would each have to invent their own answer or reject the
  combination outright - reintroducing exactly the kind of
  per-backend-guesswork this DSL exists to avoid, for a case ADR-0001
  never had to consider when it first shaped `Query` around boolean
  composition.
- **A separate `SearchBackend::vector_search()` trait method** instead of
  extending `SearchRequest`. Rejected: `rusty_knowledge`'s concrete need
  is hybrid (both signals combined into one ranked result set), not
  "vector search as an alternative to lexical search" - a second trait
  method would hand callers two separate result sets and the fusion
  problem back, rather than the backend (which knows its own fusion
  strategy) owning it.
- **A `HybridSearchBackend` supertrait/extension trait**, kept entirely
  out of `SearchBackend`/`SearchRequest`. Considered seriously: it would
  avoid touching all eight existing backends. Rejected because it would
  make `Arc<dyn SearchBackend>` unable to express "does this instance
  support vectors" without a downcast, defeating the runtime-swappable
  `Arc<dyn SearchBackend>` pattern ADR-0001 chose specifically so callers
  don't need to know the concrete backend type.
- **Silently ignore `SearchRequest::vector` in backends that don't support
  it**, treating it as a hint rather than a request. Rejected as
  inconsistent with `rusty-search-meilisearch`'s and `rusty-search-algolia`'s
  established precedent (ADR-0003): an unsupported query shape fails
  loudly here, not silently degrades.
- **Implement `sqlite-vec` hybrid search in this same change**, since
  `rusty-search-sqlite-fts5` is the obvious first home for it and the
  scaffolding (`VectorQuery`, `SearchRequest::vector`) is already being
  added. Rejected for this iteration: SQLite is embedded, so (unlike the
  remote backends' mocked-HTTP test suites) every claim this ADR makes
  about FTS5 was verified against a real, bundled SQLite build in this
  repo's own test suite - `sqlite-vec` deserves the same standard, and
  getting a second native extension (with its own build/linking story)
  right alongside a brand-new backend in one change was judged more risk
  than this "not urgent" ask warranted at once.

## Consequences
- Every claim in the `rusty-search-sqlite-fts5` decision above - FTS5
  column-filtered `MATCH` syntax, `bm25()`'s sign convention (more
  negative is better, so this backend negates it to match this
  workspace's higher-is-better convention), external-content-free
  dual-table sync, `must_not(Match)` working without restriction - is
  backed by a real, passing test against SQLite's own FTS5 module (via
  `rusqlite`'s `bundled` feature), not a mocked HTTP contract or an
  unverified read of documentation, unlike every remote backend's ADR in
  this workspace so far.
- `rusty-search-sqlite-fts5` cannot combine full-text relevance from more
  than one field in a single scored query (e.g. "match `title` OR `body`,
  ranked by whichever matched") - a real, disclosed narrower ceiling than
  `rusty-search-tantivy`/`rusty-search-elasticsearch`/`rusty-search-solr`/
  `rusty-search-azure-search`, though no narrower than
  `rusty-search-meilisearch`/`rusty-search-algolia` already are.
- `VectorQuery`/`SearchRequest::vector`/`supports_vector_search()` are
  live, compiled, tested types today, not just a design writeup - but no
  backend in this workspace can act on `SearchRequest::vector` yet. A
  caller has a real capability check (`supports_vector_search()`) and a
  real error (`SearchError::InvalidQuery`) instead of silence, but still
  needs a follow-up backend change (most likely `sqlite-vec` support in
  `rusty-search-sqlite-fts5`) before hybrid search actually works
  anywhere in this workspace.
- `SearchResults`/`Hit` do not yet carry a retrieval-mode indicator (e.g.
  "this response used hybrid vs. lexical-only retrieval",
  `rusty_knowledge`'s own `RM-KNOWLEDGE-MODEL-0005` requirement). Adding
  one speculatively, with no backend yet producing a hybrid result to
  label, was judged premature; it belongs with whichever change first
  implements real hybrid search, not this one.
