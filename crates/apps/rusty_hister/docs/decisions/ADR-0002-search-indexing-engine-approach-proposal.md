# ADR-0002: Search/indexing engine approach

Status: Accepted
Date: 2026-09-12 (decided; superseding the same-day decision-request below)

## Decided, not just requested

This ADR was opened the same day as a decision-request (the "This is a
decision-request, not a decision" framing below is kept as the historical
record of what was asked and why) and decided later the same day on the
user's (baileyrd/Nano's) explicit instruction — "Decide ADR-0002 and
ADR-0003 now" — rather than after the recommended scoping spike. The
Context and Options sections below are unchanged from the request; the
Decision section after them records what was chosen and why, in place of
running the spike first. Nothing here claims spike-validated certainty —
where the spike would have reduced risk, that risk is accepted explicitly
rather than measured, and is called out as such.

## This was a decision-request, not a decision (as originally opened)

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

## Recommended next step, as originally written (superseded — no spike was run)

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

## Decision

**Engine: Option B — `rusty_search` + `rusty-search-sqlite-fts5`**, kept
strictly behind `rusty-search-core`'s `SearchBackend` trait so a Tantivy or
a future Postgres-native full-text backend can be swapped in later without
touching `rusty-hister-indexer`'s call sites — the query-grammar compiler is
written against `rusty-search-core`'s `Query` tree, not against FTS5
directly.

Rationale: Hister's own common deployment is a single-user, single-SQLite-file
tool (Postgres is the multi-user/shared-server path). FTS5 keeps the
full-text index inside that same SQLite file rather than standing up a
second storage engine (a Tantivy index directory) alongside it, and it sits
naturally beside `sqlite-vec` (below) in one file for the SQLite deployment
path — fewer moving parts for the common case, at the cost of FTS5's
somewhat less mature Rust tooling versus Tantivy's, which is accepted.
Tantivy remains available as a documented alternative backend if FTS5 proves
insufficient once real query volume is thrown at it (nothing here forecloses
that swap).

The three sub-capabilities `rusty_search` doesn't provide (items 2, 4, 5
above) are decided as **hister-layer composition, not `rusty-search-core`
changes** — keeping that shared crate's scope untouched by this port:

- **Multi-language federation** (item 2): `rusty-hister-indexer` owns one
  `rusty-search-sqlite-fts5`-backed index per detected language and fans a
  query out across all of them, merging/re-ranking results itself — the
  same shape as bleve's `IndexAlias`, implemented one layer up instead of
  inside the search engine.
- **`url_re:` custom filtering** (item 4): implemented as a post-retrieval
  filter in `rusty-hister-indexer` — a broader candidate query through
  `rusty_search`, then the compiled regex applied against the stored/
  normalized URL field in Rust before final ranking and pagination. Same
  architecture as bleve's `CustomFilterQueryWithFilter`, just not a backend
  feature.
- **Three highlight styles** (item 5): implemented as three renderers in
  `rusty-hister-indexer` over match spans/snippets, independent of how
  mature any given backend's own highlighting API is.

**BM25 parity**: accepted as **result-set parity, not byte-exact score
ordering** — FTS5's `bm25()` will not reproduce bleve's scoring constants,
and that's fine for a personal search tool; ranking *quality* is what
matters, not bit-for-bit reproduction of Go bleve's numbers.

**`sqlite-vec` sub-decision**: vendor the upstream `sqlite-vec` C extension
via a `cc`-crate build (Tier A adapter dependency under root ADR-0002's
tiers — the same precedent `rusty_sqlite`'s bundled SQLite already
established), confined behind a narrow FFI boundary inside
`rusty-hister-vectorstore`, for the SQLite deployment path. The Postgres
deployment path uses `pgvector`, matching Hister's own presumed approach and
Postgres's standard vector-extension story. No pure-Rust vector-index
reimplementation is pursued for v1 — both are proven, narrow, widely-used C
extensions, and reimplementing either from scratch would be exactly the
kind of hand-rolling-for-its-own-sake this workspace's own README
disclaims for anything security- or correctness-sensitive at this scale.

**Accepted risk (spike not run)**: whether `rusty-search-core`'s `Query`
tree actually round-trips every grammar construct cleanly against
`rusty-search-sqlite-fts5` (in particular alternation groups, `-*`, and
numeric/time ranges) is not yet empirically verified. If
`rusty-hister-indexer`'s implementation phase (roadmap Phase 3) hits a
construct the `Query` tree can't express, the fallback is a `rusty-search-core`
API change proposed upstream in that crate (per its own review process),
not a silent reversal of this decision.

This decision does not itself add `rusty-search-sqlite-fts5`,
`sqlite-vec`'s C source, or `pgvector` bindings to any `Cargo.toml` —
those land when `rusty-hister-indexer`/`rusty-hister-vectorstore` actually
start consuming them (roadmap Phase 3), not in this bootstrap-adjacent
decision record.
