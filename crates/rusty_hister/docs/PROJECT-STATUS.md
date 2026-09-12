# PROJECT-STATUS: rusty_hister

Last updated: 2026-09-12 (Phase 1: `rusty-hister-model`'s `CrawlJob`
lifecycle query layer).

## Where this is

**Phase 1 in progress.** Bootstrap, capability inventory, and all three
ADRs (crate split/scope, search engine, JS-rendering crawler, and the
licensing policy) are settled. Three crates now have real implementation:

- `rusty-hister-core` — the `Document` working type, the `Extractor` trait
  and its supporting types (capability inventory §4.1), and the shared
  `HisterError` type.
- `rusty-hister-extractor` — `Registry`, the chain-of-responsibility
  mechanism (capability inventory §4.2): ordered registration
  (`register`/`register_before`), two-phase enrich-then-extract execution,
  a separate preview chain with case-insensitive starting-point selection,
  and config merging (`apply_configs`). **No concrete extractors yet** —
  the registry mechanism is generic over any `Extractor` impl; the 20
  built-in extractors (capability inventory §4.3-§4.5) are a separate,
  not-yet-started increment.
- `rusty-hister-model` — the nine `#[derive(Mapped)]` types from
  `automigrate()`'s list (capability inventory §7.2: `User`, `Link`,
  `History`, `HistoryLink`, `CrawlJob`, `CrawlURL`, `WebSession`,
  `DocumentVersion`, `EmbeddingJob`), soft-delete via `rusty_db`'s
  `#[table(soft_delete)]` where Go used `CommonFields`' nullable
  `DeletedAt`, and a fresh-install migration
  (`SQLITE_MIGRATIONS`/`POSTGRES_MIGRATIONS`) that creates all nine tables
  and their indexes via `rusty_db::Migrator`. Hister's own `Database`
  singleton-row schema-version tracker has no Rust equivalent —
  `rusty_db`'s `Migrator` already solves the same problem via its own
  bookkeeping table, so this is a documented substitution, not a dropped
  capability. **The embedding-queue query layer is also ported**:
  `EmbeddingJob::{enqueue, claim_next, complete, retry, fail, release,
  in_progress_exists, delete, reset_in_progress}`, matching
  `embedding.go`'s state machine exactly (the dedup-on-conflict upsert,
  the claim-loop's optimistic-concurrency retry, the dirty-job-retries-
  immediately semantics). Three of the nine (`enqueue`, `retry`,
  `release`) need a `CASE`-based `SET` clause the portable query builder
  can't express (`Update::set` only takes a `Value`, never an `Expr`), so
  those three use raw SQL built dialect-portably via
  `Engine::dialect().placeholder(..)` rather than the builder. **`WebSession`'s
  query layer is ported too**: `create`/`get`/`refresh`/`delete` (Go:
  `CreateWebSession`/`GetWebSession`/`UpdateWebSession`/`DeleteWebSession`
  — `refresh` avoids colliding with `#[derive(Mapped)]`'s own generated
  `update()` instance method). `create` is this crate's first
  database-assigned surrogate key: since `Mapped::insert()` always
  supplies the primary key's current value, `create` drops to a raw
  `INSERT` that omits the `id` column and recovers the generated value
  dialect-appropriately (`RETURNING id` on Postgres, `SELECT
  last_insert_rowid()` on SQLite) — the same recipe every other
  autoincrementing model (`User`, `Link`, `History`, `HistoryLink`,
  `DocumentVersion`) will need for its own `create`. **`DocumentVersion`'s
  query layer is ported too**: `save`/`move_versions`/`count`/`list`/
  `list_until` (Go: `SaveDocumentVersion`/`MoveDocumentVersions`/
  `CountDocumentVersions`/`GetDocumentVersions`/`GetDocumentVersionsUntil`),
  reusing `WebSession::create`'s database-assigned-surrogate-key recipe
  for `save`. The document-versioning diff format/algorithm itself
  (capability inventory §11) stays a separate, not-yet-decided concern —
  this crate only stores whatever diff text the caller already computed.
  **`CrawlJob`'s own lifecycle is ported too**: `generate_id`/`create`/
  `create_with_urls`/`get`/`update_status`/`list`/`delete` (Go:
  `GenerateCrawlJobID`/`CreateCrawlJob`/`CreateNamedCrawlJobWithURLs`/
  `GetCrawlJob`/`UpdateCrawlJobStatus`/`ListCrawlJobs`/`DeleteCrawlJob`).
  `generate_id` uses the existing first-party `rusty_rand` crate for its
  OS-backed CSPRNG bytes rather than adding the external `rand` crate — a
  sovereignty-loop pass found `rusty_rand` already exists in this
  workspace precisely to avoid that. `create_with_urls`'s job-id-collision
  retry loop and its per-URL dedup both need `ON CONFLICT DO NOTHING`
  inside one atomic unit, so it drops to raw SQL inside a `Transaction`
  (`Engine::begin()`/`Transaction::execute`/`commit`) — the same
  raw-SQL-for-conflict-handling pattern as `EmbeddingJob::enqueue`,
  extended here to a multi-statement transaction. `CrawlURL`'s own queue
  mechanics (bulk insert, per-URL status updates, the streaming
  iterators, job stats) are a separate, not-yet-started increment — this
  file's query layer covers `CrawlJob` only. The remaining two domain
  files' query-layer helpers (`history.go`'s search/pin/timeline queries,
  `user.go`'s auth helpers) are still a separate, not-yet-started
  increment.

All three have unit tests, `clippy`, and `fmt` clean. Still not started:
the concrete extractors, the rest of the model crate's query-layer
helpers, and `rusty-hister-crawler`'s `http` backend (the rest of Phase 1
per `docs/roadmap/ROADMAP.md`).

## v1 scope (per the kickoff brief, recorded here as the sign-off of record
for this scope reduction — see `docs/decisions/
ADR-0001-bootstrap-scope-and-crate-split.md` §"v1 scope decision")

**In scope for v1**: an HTTP/JSON API (`rusty-hister-server`) and MCP
JSON-RPC surface (`rusty-hister-mcp`) that stay wire-compatible with
Hister's existing SvelteKit `webui`, browser extension, and qutebrowser
companion. Supporting crates: `rusty-hister-core`, `rusty-hister-model`,
`rusty-hister-extractor`, `rusty-hister-indexer`, `rusty-hister-vectorstore`,
`rusty-hister-crawler`.

**Explicitly deferred past v1** (not silently dropped — tracked as later
phases in `docs/roadmap/ROADMAP.md`): the CLI (cobra-equivalent, ~35
subcommands), the TUI (Bubble Tea, `cmd/tui`), the qutebrowser companion
daemon, and the browser extension. The extension and companion stay
TypeScript/Svelte and Manifest V3 respectively — "port to Rust" doesn't
apply to them; what's tracked is that `rusty-hister-server`'s HTTP contract
must stay byte-compatible with what they already call.

## Decided (no longer blocking)

| ADR | Subject | Status | Decision |
|---|---|---|---|
| [ADR-0002](decisions/ADR-0002-search-indexing-engine-approach-proposal.md) | Search/indexing engine approach | **Accepted** | `rusty_search` + `rusty-search-sqlite-fts5`; multi-language federation, `url_re:` filtering, and highlight rendering built as hister-layer composition, not backend changes; `sqlite-vec` (C extension, vendored) for SQLite semantic search, `pgvector` for Postgres |
| [ADR-0003](decisions/ADR-0003-js-rendering-crawler-approach-proposal.md) | JS-rendering crawler approach | **Accepted** | `chromiumoxide` (Tier A adapter dependency) for the CDP path; WebDriver BiDi explicitly **descoped** for v1 |

Both were decided directly by the user (baileyrd/Nano) on 2026-09-12,
rather than after the scoping spike ADR-0002 recommended — see each ADR's
own "Accepted risk (spike not run)" note for what that trades away.
`rusty-hister-indexer`'s query-DSL-to-`Query`-tree compiler,
`rusty-hister-vectorstore`'s storage side, and `rusty-hister-crawler`'s
`chromedp`-equivalent (CDP) backend are now unblocked to start (roadmap
Phases 3-4); none of that implementation has started yet as of this update.
The `bidi`-equivalent backend is not merely unblocked-but-pending — it is
now **out of v1 scope**, per ADR-0003's decision.

## Resolved

- **Licensing approach for Go test fixtures** (ADR-0001 §7) — confirmed by
  the user 2026-09-12: `rusty_hister` ships under this workspace's standard
  `MIT OR Apache-2.0`; no Hister source file, test files included, is
  copied verbatim into this cluster. Fresh Rust tests are derived from
  independently reading and understanding each Go test's behavior instead.
  This binds every extractor and query-grammar test written from here on.

## Open items carried from the capability inventory (not blocking, but
unresolved — see `docs/capability-inventory/HISTER-CAPABILITY-INVENTORY.md`
§13 and inline flags)

- `sqlite-vec` C-extension story for the SQLite vectorstore backend — folded
  into ADR-0002 rather than decided separately, since it's downstream of the
  storage-engine choice.
- **Whether `rusty_hister` needs to open a pre-existing Hister-Go-created
  database file at all**, versus targeting fresh installs only. This is a
  broader question than the two items below — it decides whether either of
  them needs any Rust-side handling in the first place. Discovered while
  designing `rusty-hister-model`'s schema (2026-09-12): that crate's
  migration currently targets the fresh-install (post-all-historical-
  migrations) schema shape only, on the working assumption that this
  question resolves toward "no" or toward a separate, explicit import/
  upgrade tool rather than `rusty_hister` opening a Go-created file
  in-place. Not yet ratified either way.
- Legacy pre-GORM `indexer_versions` table read path (capability inventory
  §7.3) — flagged for explicit scope sign-off, not yet resolved either way.
  Subsumed by the item above: only relevant if opening a pre-existing
  Hister database is in scope at all.
- Hister's three historical Go migrations (`history_links.pinned` backfill,
  `web_sessions.last_seen_at` column drop, mixed-offset-to-UTC timestamp
  rewrite — capability inventory §7.3) — not reproduced in
  `rusty-hister-model`'s migration, since they are one-time data-shape
  transitions for a database that predates them, not part of the current
  schema's shape. Same subsumption as the item above: only relevant if
  opening a pre-existing Hister database is in scope.
- Document-versioning diff format/algorithm (capability inventory §11) and
  query-alias expansion site (capability inventory §11) — flagged in the
  inventory as needing a follow-up read of the Go source; still not done.
  `DocumentVersion`'s query layer (now ported, see below) only stores and
  retrieves whatever diff text the caller already computed — it does not
  decide the format, so this item is unaffected by that increment landing.
- `rusty-hister-model`'s remaining query-layer helpers (the domain
  operations each Go model file builds on top of its table — the
  embedding-queue state machine, `WebSession`, `DocumentVersion`, and
  `CrawlJob`'s own lifecycle are now done, see the crate status table
  below — leaving `history.go` search/pin/timeline queries, `user.go`
  auth helpers, and `CrawlURL`'s queue mechanics: bulk insert, per-URL
  status updates, the `ForEach*` streaming iterators, job stats) —
  deliberately deferred past the schema increment, the same split
  `rusty-hister-extractor` used (mechanism before concrete extractors);
  not yet started.
- Postgres path for `rusty-hister-model`'s raw-SQL query-layer functions
  (`EmbeddingJob::enqueue`/`retry`/`release`, `WebSession::create`,
  `DocumentVersion::save`, `CrawlJob::create_with_urls`) is untested —
  only SQLite is exercised in unit tests (no Postgres available in this
  environment). The SQL is written to be dialect-portable (ANSI-standard
  `ON CONFLICT ... DO UPDATE SET`/`ON CONFLICT ... DO NOTHING`/
  `CASE WHEN`/`RETURNING`, placeholders rendered per-dialect via
  `Engine::dialect().placeholder(..)`, and the `RETURNING`-vs-
  `last_insert_rowid()` branch keyed off `Dialect::supports_returning()`)
  but has not been run against a real Postgres instance — same risk
  profile as the untested `POSTGRES_MIGRATIONS` array from the schema
  increment.

## Crate status

| Crate | Status |
|---|---|
| `rusty-hister-core` | **In progress** — `Document`, `Extractor` trait + `Capabilities`/`ExtractorConfig`/`ExtractOutcome`/`PreviewOutcome`/`PreviewResponse`, `HisterError`. 16 unit tests, clippy/fmt clean. `DocumentType`'s wire-format integer encoding deliberately left unassigned (see its doc comment) until `rusty-hister-server` needs it and the real Hister values are confirmed. |
| `rusty-hister-model` | **In progress** — schema: nine `#[derive(Mapped)]` types (`User`, `Link`, `History`, `HistoryLink`, `CrawlJob`, `CrawlURL`, `WebSession`, `DocumentVersion`, `EmbeddingJob`), soft-delete via `rusty_db`'s `#[table(soft_delete)]`, fresh-install SQLite+Postgres migrations via `rusty_db::Migrator`. Query layer: `EmbeddingJob`'s embedding-queue state machine (9 functions), `WebSession`'s lookup/expiry helpers (4 functions), `DocumentVersion`'s save/move/count/list helpers (5 functions), and `CrawlJob`'s own lifecycle (7 functions, including the atomic job-plus-initial-queue creation with id-collision retry) done; `history.go`/`user.go`'s query helpers and `CrawlURL`'s queue mechanics not yet started. 65 unit tests (real SQLite round-trips, unique-constraint/duplicate-rejection checks, migration up/down/status, the embedding queue's dedup/claim/retry/dirty-job semantics, `WebSession`'s create/get/refresh/delete round trips, `DocumentVersion`'s save/list/count/move/list_until behavior, `CrawlJob`'s create/get/update_status/list/delete and the id-collision-retry/empty-URL-list-rejection behavior of `create_with_urls`), clippy/fmt clean. |
| `rusty-hister-extractor` | **In progress** — `Registry` (chain-of-responsibility: ordered registration, two-phase enrich/extract, preview-chain starting points, config merging). 18 unit tests, clippy/fmt clean. No concrete extractors yet. |
| `rusty-hister-indexer` | Skeleton only — unblocked by ADR-0002, not yet started |
| `rusty-hister-vectorstore` | Skeleton only — unblocked by ADR-0002, not yet started |
| `rusty-hister-crawler` | Skeleton only — CDP backend unblocked by ADR-0003, not yet started; BiDi backend out of v1 scope |
| `rusty-hister-server` | Skeleton only |
| `rusty-hister-mcp` | Skeleton only |
