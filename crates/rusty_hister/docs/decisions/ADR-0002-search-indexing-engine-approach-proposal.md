# ADR-0002: Search/indexing engine approach — DECISION REQUEST

Status: Proposed — awaiting sign-off
Date: 2026-09-12

## This is a decision-request, not a decision

Per the kickoff brief: Hister's BM25 + custom query language is the single
hardest piece to port, and this ADR exists to get an explicit answer before
`rusty-hister-indexer` or `rusty-hister-vectorstore`'s storage side write any
code — not to record one this session picked unilaterally. Everything below
is framed as options and a recommended next step (a scoping spike), not a
conclusion.

## Context

Hister's Go source (`server/indexer/`, capability inventory
`docs/capability-inventory/HISTER-CAPABILITY-INVENTORY.md` §5) uses `bleve`
for five distinct, separable sub-capabilities:

1. **Full-text indexing + BM25-family scoring.** Table-stakes; multiple Rust
   engines do this.
2. **`IndexAlias`-style multi-index federation** — when `detect_languages`
   is on, each detected language gets its own on-disk index, and search/
   facet queries fan out across all of them transparently. No off-the-shelf
   Rust engine does this; it's fan-out/merge logic to build regardless of
   the underlying engine choice.
3. **The custom query grammar** itself (hand-written lexer: quoting,
   alternation groups, negation including `-*`'s "match nothing"
   special case, wildcards, field filters, per-field kinds, numeric/time
   ranges, alias expansion). This is **not** a bleve feature — it's
   independent Go code that compiles down to bleve query objects. It ports
   cleanly to a hand-written Rust parser regardless of backend, and is
   confirmed to be the *easy* half of this problem, not the hard half.
4. **`url_re:`'s custom-filter-after-retrieval pattern** — bleve has no
   native full-string-regex-on-a-stored-field primitive, so Hister runs a
   literal Go regex as a post-filter over `MatchAllQuery` candidates via
   `NewCustomFilterQueryWithFilter`. Whatever engine is chosen needs an
   equivalent "run an arbitrary predicate after primary retrieval" hook.
5. **Three highlight-rendering styles** (HTML spans for the web UI, ANSI for
   the CLI, a bespoke "tui" style for the Bubble Tea client) driven by
   bleve's `Highlight`/`HighlightWithStyle`. Tantivy's highlighting API is
   less mature than bleve's.

Separately, semantic/vector search (`server/vectorstore/`, capability
inventory §6) needs a vector index — Hister's SQLite backend vendors the
`sqlite-vec` C extension via cgo, hand-patched for musl; its Postgres
backend uses (presumably) pgvector.

The sovereignty audit of `rusty_search` (this workspace's existing
SQLAlchemy-style pluggable search abstraction,
`crates/rusty_search/README.md`) found:

- `rusty-search-tantivy` and `rusty-search-sqlite-fts5` both do **real BM25**
  scoring today (Tantivy's own scorer; SQLite FTS5's native `bm25()`
  function).
- `rusty-search-core`'s `Query` type is a **structured Rust builder**
  (`MatchAll`/`Term`/`Match`/`Range`/`Bool{must,should,must_not,filter}`),
  not a text grammar — each backend translates this tree into its own native
  query representation. This shape looks like it could host item 3 above (a
  parser that compiles Hister's grammar into this `Query` tree) without
  needing changes to `rusty-search-core` itself, but that's untested — see
  the recommended spike below.
- `SearchRequest.vector: Option<VectorQuery>` is **designed for but not
  implemented in any backend yet** (per `rusty_search`'s own ADR-0008)
  — `sqlite-fts5` + `sqlite-vec` is named in `rusty_search`'s own docs as
  the natural next step, but doesn't exist today.
- Nothing in `rusty_search` addresses items 2, 4, or 5 above — those would
  be new work regardless of which backend crate is chosen.

## Options

### Option A: `rusty_search` + `rusty-search-tantivy`, extend as needed

Use the existing pluggable abstraction; write the query-grammar-to-`Query`
compiler against `rusty-search-core`'s trait; extend `rusty-search-tantivy`
(or add a sibling backend crate under `rusty_search`) for multi-index
federation, the custom-filter-after-retrieval hook, and vector search.

- **Pro**: reuses a maintained, already-integrated first-party abstraction;
  new capability (vector search, federation) benefits every future
  `rusty_search` consumer, not just Hister; keeps `rusty-hister-indexer`
  thin.
- **Con**: couples this port's timeline to extending a shared crate outside
  this cluster — federation and the custom-filter hook may need
  `rusty-search-core` API changes that need their own review/sign-off in
  that crate, not just this one; BM25 scoring will not be bit-identical to
  bleve's regardless (different scoring constants/normalization) — an
  explicit "scoring parity is best-effort" acceptance is needed either way.

### Option B: `rusty_search` + `rusty-search-sqlite-fts5`

Same shape as Option A, but on the SQLite-FTS5 backend instead of Tantivy —
this keeps everything (full-text index, document store, and eventually
`sqlite-vec`-based vector search) inside one SQLite file, closer to Hister's
own SQLite deployment story, and avoids running two separate storage engines
side by side for the common single-user/single-file deployment case.

- **Pro**: one storage engine to operate for the common case; `sqlite-vec`
  integration (if pursued) sits naturally alongside FTS5 in the same DB
  file, unlike Tantivy which would need a separate on-disk index directory.
- **Con**: FTS5's query capabilities and Rust ecosystem tooling around it
  are less mature than Tantivy's for advanced cases (e.g. custom scoring
  tweaks, faceting depth); federation/multi-language-index story likely
  still hand-rolled either way.

### Option C: A new, dedicated full-text engine purpose-built for this
grammar, outside `rusty_search`

Build directly against Tantivy (or another engine) inside
`rusty-hister-indexer` itself, without going through `rusty_search`'s
abstraction layer, if the grammar's specific needs (multi-index federation,
custom-filter-after-retrieval, three highlight styles) turn out to need
engine internals `rusty_search`'s trait doesn't expose cleanly.

- **Pro**: no cross-cluster coordination; full control over the engine.
- **Con**: violates this workspace's own sovereignty-loop discipline
  (check for an existing crate before hand-rolling) without first trying
  Option A/B and finding a concrete blocker; loses the reuse benefit for
  future `rusty_search` consumers.

## Recommended next step (not a decision)

A **small scoping spike** before committing to any option, per the kickoff
brief's explicit instruction: implement a throwaway prototype that runs
Hister's actual query grammar (using the parser/lexer test cases in
`server/indexer/querybuilder/parser_test.go` and `builder_test.go` as the
oracle, rewritten independently per ADR-0001 §7's licensing note, not
copied) against `rusty-search-tantivy` and/or `rusty-search-sqlite-fts5`,
specifically exercising: alternation groups, `-*` negated-match-all,
`url_re:` filtering, and one multi-field query. That prototype's outcome —
does `rusty-search-core`'s `Query` tree round-trip cleanly, does the
custom-filter hook exist or need adding, how bad is the federation gap in
practice — should inform the actual choice, rather than picking abstractly.

## What this ADR is asking the user to decide

1. Which option (A/B/C) to pursue, or whether to run the scoping spike
   first and let its outcome decide.
2. Whether "BM25 scoring will not be bit-identical to bleve's" is an
   acceptable parity bar (recommended: yes — result *set* parity matters
   far more than exact score-based ordering for a personal search tool).
3. The `sqlite-vec` sub-decision from ADR-0001 §6: load the same C extension
   from Rust (via the `cc` crate, vendoring the same upstream source), use a
   pure-Rust vector index instead, or support only the Postgres/pgvector
   path for semantic search initially and defer SQLite+vectors.
