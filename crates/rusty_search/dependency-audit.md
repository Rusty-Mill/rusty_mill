# Dependency sovereignty audit — `rusty_search`

Run via the `sovereignty-loop` skill. Scope: direct, non-dev, non-build
dependencies across the workspace (`cargo metadata --no-deps`). No
already-decided "floor" dependencies are declared in this repo's
ARCHITECTURE.md/ADRs (unlike e.g. `rustils`' RFC v2 naming `libc`/
`windows-sys`), so nothing was excluded on that basis.

Platform search covered every `baileyrd`/`Rusty-Mill` repo whose name
plausibly matched a dependency (`rusty_sqlite`, `rusty_serde`, `rusty_json`,
`rusty_tokio`, `rusty_time`, `rusty_err`, `rusty_error`, `rusty_async`,
`rusty_chrono`, `rusty_uuid`, `rusty_request`, `rusty_http`) — each was
cloned and its actual source read, not just its name matched. Four
name-matched candidates (`rusty_error`, `rusty_async`, `rusty_chrono`,
`rusty_uuid`) turned out to be **empty repositories** (reserved names, no
code yet) and are noted as "no coverage" rather than silently skipped.

| Dependency | Purpose | Classification | Internal candidate | Size | Recommended action | Notes |
| --- | --- | --- | --- | --- | --- | --- |
| `rusqlite` | SQLite + FTS5 bindings, `rusty-search-sqlite-fts5` | partial | `rusty_sqlite` | S | Extend & swap | Re-exports the *same* `rusqlite = "0.31"` (`bundled`) underneath — doesn't eliminate the external crate, but consolidates connection setup behind one wrapper that already applies pragmas this backend currently doesn't (WAL journaling, foreign-key enforcement, busy timeout) and offers a typed `Fts5TableBuilder` instead of hand-rolled `CREATE VIRTUAL TABLE ... USING fts5(...)` strings. Real, tractable, mostly a call-site refactor. |
| `serde` | Derive macros for `Document`/`Schema`/`Query`/... — `rusty-search-core`'s public API | covered | `rusty_serde` | XL | Track as its own initiative, not this loop | From-scratch, zero-crates.io-dependency reimplementation with its **own** `Serialize`/`Deserialize` traits — not source-compatible with real `serde`. Swapping breaks `Document`'s public `fields: serde_json::Map<String, serde_json::Value>` surface for every downstream consumer (the README explicitly tells callers to convert via `serde_json`). This is an ecosystem-wide breaking migration, not an inline dependency swap. |
| `serde_json` | `Document`'s `Value`/`Map`, every backend's wire format | partial | `rusty_json` | L | Bundle with the `serde` row into one larger initiative | More promising than `rusty_serde` for compatibility — it works *over* real `serde::Serialize`/`Deserialize` (so `#[derive(serde::Serialize)]` types still work) — but it still depends on real `serde` itself, so this alone doesn't remove that dependency, and its own `Value` type isn't a drop-in for the exact JSON representation `Document`'s public API already commits to. Same blast radius as the `serde` row; don't split into two uncoordinated swaps. |
| `tokio` | Async runtime; `RwLock`/`Mutex` in 7 backends; `#[tokio::test]` | keep external | `rusty_tokio` (exists, not compatible) | — | Keep | Foundational for a *library* meant to run inside arbitrary consumer applications' own tokio runtimes. `rusty_tokio` is a real, serious alternative runtime (its own scheduler/reactor/timers), but forcing every downstream consumer onto it is a far bigger decision than dependency sovereignty — matches this skill's own stated precedent for `tokio` exactly ("foundational, no sovereignty case strong enough to justify hand-rolling an async runtime"). |
| `async-trait` | Object-safe `async fn` on `SearchBackend` (ADR-0001) | keep external | none (`rusty_async` is an empty repo) | — | Keep | No internal coverage exists. ADR-0001 already chose `async-trait` deliberately as "the ecosystem-standard choice" for this exact problem. |
| `reqwest` | HTTP client — algolia/azure-search/elasticsearch/opensearch/solr backends | partial | `rusty_request` (+ `rusty_http`) | — | Keep for now; revisit if `rusty_request` grows a real-tokio adapter | `rusty_request` is hard-pinned to `rusty_tokio`, not real tokio, with no `tokio` feature flag — unlike `rusty_http`, which already has one (`features = ["tokio"]`, built specifically for `rusty_tail`'s migration). Swapping today would force these backend crates onto a second, incompatible async runtime alongside the real tokio the rest of this crate already requires. Worth a follow-up ask to `rusty_request` once/if it gains the same real-tokio adapter `rusty_http` has. |
| `meilisearch-sdk` | Official Meilisearch client SDK | keep external | none | — | Keep | Hosted third-party SaaS client; no plausible internal equivalent. ADR-0003 already chose the official SDK over hand-rolling for Meilisearch's async task-model correctness. |
| `tantivy` | Embedded full-text search engine `rusty-search-tantivy` wraps | keep external | none | — | Keep | This *is* the product being wrapped — a real inverted-index engine — not a utility dependency to abstract away. Matches the reason this crate (and `rusty-search-sqlite-fts5`) exist in the first place. |
| `thiserror` | Typed `SearchError` enum derive, `rusty-search-core` | keep external | `rusty_err` (no derive macro yet) | — | Keep, revisit later | `rusty_err`'s description promises a derive macro, but its current source (52 lines) only has the `Error`/`Context` traits — no `#[derive(Error)]` equivalent to `SearchError`'s `#[error("...")]`/`#[from]` attributes exists yet. |
| `anyhow` | Catch-all backend error wrapping (`SearchError::Backend`), 8 backend crates | keep external | `rusty_err` (`Context` stringifies, doesn't box/preserve) | — | Keep, revisit later | `rusty_err::Context` converts an error straight to `Result<T, String>`, losing the original error rather than preserving/downcasting it the way `anyhow::Error` does — not a drop-in for how every backend's `backend_err()` helper is used today. |
| `time` | RFC 3339 date parsing/validation, `rusty-search-tantivy` + `rusty-search-sqlite-fts5` | keep external | `rusty_time` (formatter only, no parser) | — | Keep, revisit later | Description promises an ISO-8601 "parser/formatter", but the current source (133 lines) has no `from_str`/parse function at all, and `DateTime::timestamp()` is a stub that always returns `0`. Can't validate/parse the RFC 3339 strings `Query::Range` on `Date` fields depends on. |
| `uuid` | Client-side document id generation, 6 backend crates | keep external | none (`rusty_uuid` is an empty repo) | — | Keep | Name matches, but no source exists yet. |

## Summary

- **1 tractable swap** worth doing now: `rusqlite` → `rusty_sqlite`.
- **2 rows** (`serde`, `serde_json`) are genuinely `covered`/`partial` by
  real internal crates, but at a size (public-API-breaking, ecosystem-wide)
  this loop's normal swap mechanics aren't built for — flagging as a
  separate, deliberately-scoped initiative rather than folding in here.
- **1 row** (`reqwest`) has a real internal candidate blocked on a specific,
  nameable gap (no real-tokio adapter) rather than "no coverage" — worth
  raising with whoever owns `rusty_request`.
- **4 rows** (`thiserror`, `anyhow`, `time`, `uuid`) have name-matching
  internal repos that turned out to be too immature (missing the derive
  macro / boxed-error semantics / parser / any code at all) to be real
  candidates today — logged so a future re-run doesn't have to
  re-investigate from scratch, and doesn't silently drop them either.
- **4 rows** (`tokio`, `async-trait`, `meilisearch-sdk`, `tantivy`) are
  deliberate keeps: foundational, no internal equivalent, or the product
  being wrapped rather than a utility to sovereignize away.

Nothing here proceeds without a pick — which row(s), if any, should move to
step 4?
