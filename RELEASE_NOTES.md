# Release Notes

Tracks the **monorepo itself** — crate merges, workspace-wide CI, and
cross-crate changes (like the duplication sweeps below) — not each crate's
own internal changes, which are logged in that crate's own
`crates/<name>/RELEASE_NOTES.md` where one exists (many crates kept theirs
from before the merge; see ADR-0001 for why root and per-crate logs are
separate rather than one superseding the other).

One entry per merged PR against `main`, reverse chronological, each linking
to its PR. Bolded inline category tags (`**Added:**` / `**Changed:**` /
`**Fixed:**`), known limitations stated plainly.

---

## Continue rusty_hister Phase 1: implement rusty-hister-model's WebSession query layer
**2026-09-12** · branch [`claude/hister-phase1-model-websession`](https://github.com/Rusty-Mill/rusty_mill/tree/claude/hister-phase1-model-websession)

Fifth Phase 1 increment (after `rusty-hister-core`, `rusty-hister-extractor`'s
registry, `rusty-hister-model`'s schema, and its embedding-queue query
layer, previous entries below). Scoped to `WebSession`'s query layer
alone — small and fully self-contained, and the first place this crate
needs a database-assigned surrogate key, worth solving and documenting in
isolation before the remaining five model files (which mostly share the
same autoincrementing-`i64`-primary-key shape) need the same recipe.

- **Added:** `WebSession::create`/`get`/`refresh`/`delete` — a port of
  `session.go`'s `CreateWebSession`/`GetWebSession`/`UpdateWebSession`/
  `DeleteWebSession`. `get` returns `Option<WebSession>` rather than a
  `HisterError`/sentinel-error pair for the not-found case — the
  idiomatic Rust equivalent of Go's dedicated `ErrWebSessionNotFound`.
  Named `refresh`, not `update`, since `#[derive(Mapped)]` already
  generates an `update()` instance method (turns a whole struct value
  into an `UPDATE` statement) that a same-named associated function would
  collide with.
- **Added:** `create`'s database-assigned-primary-key recipe —
  `rusty_db::Mapped::insert()` always includes the primary key field's
  current value (there's no "leave this to the database" marker), so it
  can't populate an autoincrementing `i64` column on its own. `create`
  instead issues a raw `INSERT` that omits the `id` column, then recovers
  the generated value dialect-appropriately: `RETURNING id` where
  `Dialect::supports_returning()` is true (Postgres), `SELECT
  last_insert_rowid()` otherwise (SQLite — this crate's dialect model
  reports `false` here even though modern SQLite itself supports
  `RETURNING`). Documented in `session.rs` as the pattern
  `User`/`Link`/`History`/`HistoryLink`/`DocumentVersion` will reuse for
  their own `create`s.
- **Changed:** the dialect-placeholder helper (`Engine::dialect().placeholder(..)`
  rendering) introduced for the embedding queue's raw-SQL functions is
  now a shared `crate::placeholders` in `lib.rs`, since `WebSession::create`
  is a second real call site.
- **Added:** 7 new unit tests (50 total in the crate) — create assigns a
  positive, distinct id per session and the row is retrievable by token
  hash; get returns `None` for an unknown hash; refresh changes
  data/expiry for a known session and reports `false` for an unknown one;
  delete removes a session and no-ops for an unknown one. clippy/fmt
  clean.

## Continue rusty_hister Phase 1: implement rusty-hister-model's embedding-queue query layer
**2026-09-12** · branch [`claude/hister-phase1-model-embedding-queue`](https://github.com/Rusty-Mill/rusty_mill/tree/claude/hister-phase1-model-embedding-queue)

Fourth Phase 1 increment (after `rusty-hister-core`, `rusty-hister-extractor`'s
registry, and `rusty-hister-model`'s schema, previous entries below). Scoped
to `EmbeddingJob`'s query-layer alone — the first of the six model files'
domain operations flagged as a follow-up when the schema increment landed
— since it's fully self-contained (no dependency on any other table) and
exercises the two genuinely hard parts of this crate's remaining work
(a dialect-portable upsert, and an optimistic-concurrency claim loop) in
isolation before touching anything else.

- **Added:** `EmbeddingJob::enqueue`/`claim_next`/`complete`/`retry`/
  `fail`/`release`/`in_progress_exists`/`delete`/`reset_in_progress` — a
  field-for-field port of `embedding.go`'s durable, deduplicated embedding
  work queue: enqueuing a pending job is a no-op, enqueuing an active job
  marks it dirty instead of resetting it, `claim_next` atomically claims
  the oldest available job (retrying its select-then-claim pair when
  another worker races ahead), and `complete`/`fail` return a dirty job to
  pending instead of deleting/failing it.
- **Added:** a small dialect-portable raw-SQL path for the three
  operations (`enqueue`'s `ON CONFLICT ... DO UPDATE SET` upsert,
  `retry`'s and `release`'s `CASE WHEN ... THEN ... ELSE ... END` SET
  clauses) that `rusty_db`'s query builder can't express — `Update::set`
  only ever takes a `Value`, never an `Expr`. Placeholders are rendered
  per-dialect via `Engine::dialect().placeholder(..)` rather than
  hardcoding `?`/`$N`, so the same SQL text works against both SQLite and
  Postgres; the other six functions use the ordinary `Select`/`Update`/
  `Delete` builder.
- **Known limitation:** the Postgres path is untested — only SQLite is
  exercised (no Postgres instance available in this environment). Flagged
  in `crates/rusty_hister/docs/PROJECT-STATUS.md`'s open items, same risk
  profile as the schema increment's untested `POSTGRES_MIGRATIONS`.
- **Added:** 18 new unit tests (43 total in the crate) — enqueue's
  three outcomes (fresh/idempotent/dirty-marking/failed-job-reset),
  claim_next's ordering and availability filtering, complete/fail's
  dirty-job-returns-to-pending branch, retry's immediate-vs-scheduled
  branch, release's never-negative attempt count, and
  in_progress_exists/delete/reset_in_progress. clippy/fmt clean.

## Continue rusty_hister Phase 1: implement rusty-hister-model's schema
**2026-09-12** · branch [`claude/hister-phase1-model`](https://github.com/Rusty-Mill/rusty_mill/tree/claude/hister-phase1-model)

Third Phase 1 increment (after `rusty-hister-core` and
`rusty-hister-extractor`'s registry, previous entries below). Scoped to
`rusty-hister-model`'s **schema** — the nine `#[derive(Mapped)]` types and
the migration that creates them — not the domain/query-layer behavior each
Go model file builds on top of its table, which is a separate follow-up
increment (same schema-mechanism-first split `rusty-hister-extractor`'s
registry used).

- **Added:** `User`, `Link`, `History`, `HistoryLink`, `CrawlJob`,
  `CrawlURL`, `WebSession`, `DocumentVersion`, `EmbeddingJob` — Hister's
  nine `automigrate()`-list models (capability inventory §7.2), ported
  field-for-field from `server/model/*.go`. `CrawlJobStatus`/
  `CrawlUrlStatus`/`EmbeddingJobStatus` are `#[derive(MappedEnum)]` closed
  enums rather than Go's untyped string constants, making each field's
  "only these values are valid" invariant checkable by the type system.
- **Added:** soft-delete on the four models that embedded Go's
  `CommonFields` (`User`, `Link`, `History`, `HistoryLink`) via `rusty_db`'s
  first-party `#[table(soft_delete)]`, rather than hand-rolling
  `CommonFields`' nullable `DeletedAt` convention — an equivalent
  capability via a different, already-available mechanism, not a
  simplification.
- **Added:** `SQLITE_MIGRATIONS`/`POSTGRES_MIGRATIONS` — a fresh-install
  migration (one per backend, since `rusty_db` migrations are plain,
  non-portable SQL) that creates all nine tables and their unique/lookup
  indexes, run through `rusty_db::Migrator`. Hister's own `Database`
  singleton-row schema-version tracker has no Rust equivalent: `Migrator`'s
  own bookkeeping table already solves the same problem, so this is a
  documented substitution, not a dropped capability.
- **Known limitation (by design, flagged for explicit follow-up):** this
  is a fresh-install-only schema. Hister's three historical Go migrations
  (the `history_links.pinned` backfill, the `web_sessions.last_seen_at`
  column drop, the mixed-offset-to-UTC timestamp rewrite) and the legacy
  `indexer_versions` read path are **not** reproduced, since they only
  matter for opening a pre-existing Hister-Go-created database file — a
  new, still-unresolved question (does `rusty_hister` ever need to do
  that at all?) recorded in `crates/rusty_hister/docs/PROJECT-STATUS.md`'s
  open items, alongside the already-flagged `indexer_versions` item it
  subsumes.
- **Added:** 25 unit tests — real round-trips through an in-memory SQLite
  engine (`sqlite::memory:`) for every model, unique-constraint/duplicate-
  rejection checks for every `uniqueIndex` in the Go source
  (`username`, `url`, `(user_id, query)`, `(history_id, link_id)`,
  `(job_id, url)`, `token_hash`), a soft-delete round-trip
  (`Session::delete`/`get`/`load_active`), and migration up/down/status
  checks (including that the schema is actually created and is
  reversible). clippy/fmt clean.

## Continue rusty_hister Phase 1: implement rusty-hister-extractor's registry
**2026-09-12** · branch [`claude/hister-phase1-extractor-registry`](https://github.com/Rusty-Mill/rusty_mill/tree/claude/hister-phase1-extractor-registry)

Second Phase 1 increment (after `rusty-hister-core`, previous entry below).
Scoped to `rusty-hister-extractor`'s `Registry` alone — the
chain-of-responsibility mechanism, not any concrete extractor — since it
only depends on `rusty-hister-core` (already merged) and is a
self-contained, well-specified unit on its own.

- **Added:** `Registry` (capability inventory §4.2): `register`/
  `register_before` with case-insensitive duplicate-name rejection;
  `apply_configs` to merge a pre-parsed name→config map into matching
  extractors (unknown names ignored, matching Hister's own "config for an
  unregistered extractor is a no-op" behavior — parsing an actual config
  *file* into that map is a separate, not-yet-decided concern); `list`/
  `list_enabled`/`list_matching`/`list_matching_preview` introspection.
- **Added:** the two-phase extraction chain — every matching enabled
  enricher runs in chain order first (a `Fallback` is skipped over, only
  `Abort` halts everything), with its enrichment carried forward into the
  next stage; then matching enabled content extractors run in chain order
  until one succeeds or aborts. Verified with a test that actually checks
  the second-phase extractor receives the first phase's enrichment (not
  just that the chain doesn't crash).
- **Added:** the separate preview chain — an optional case-insensitive
  starting-point name skips ahead in chain order without disabling the
  fallback chain after it; a starting point that's unregistered, disabled,
  non-preview-capable, or non-matching is a hard `Abort`, never silently
  ignored.
- **Verified:** 18 unit tests (including every hard-error path and the
  enrichment hand-off), clippy/fmt clean, whole-cluster `cargo check`
  clean, dependency-sovereignty policy clean. `Registry` is deliberately
  not internally synchronized (no mutex) — Hister's Go version guards its
  list because it's shared across concurrent HTTP handlers; that's a
  caller-side concern (e.g. `rusty-hister-server` wrapping it in a
  `Mutex`/`RwLock`), not something to build in speculatively here.
- **Not done here:** no concrete extractors — `rusty-hister-extractor` has
  a working chain mechanism and nothing registered into it yet. That's the
  next increment (capability inventory §4.3-§4.5, in default-chain order).

---

## Start rusty_hister Phase 1: implement rusty-hister-core
**2026-09-12** · branch [`claude/hister-phase1-core`](https://github.com/Rusty-Mill/rusty_mill/tree/claude/hister-phase1-core)

First real implementation in the `rusty_hister` cluster (previously all
eight crates were empty skeletons). Scoped to `rusty-hister-core` alone —
the shared contract every other `rusty-hister-*` crate depends on — rather
than all of Phase 1 (`rusty-hister-model`, `-extractor`, `-crawler`'s `http`
backend) in one PR, matching this project's established pattern of
PR-sized increments.

- **Added:** `Document` — the extractor pipeline's working document type
  (url, title, text, html, favicon, label, document type, language,
  metadata), distinct from `rusty-hister-model`'s persisted `History`/`Link`
  rows. `DocumentType` (`Web`/`Local`/`RemoteFile`) deliberately leaves its
  wire-format integer encoding unassigned — capability inventory review
  only confirmed one value ("2 = remote-file snapshot"), and v1's
  byte-compatibility requirement means guessing the rest would risk a wire
  mismatch against real Hister clients; assign it when `rusty-hister-server`
  needs it and the exact values can be confirmed.
- **Added:** the `Extractor` trait (capability inventory §4.1) — `name`,
  `description`, `capabilities`, `matches`, `extract`, `preview`, `config`,
  `set_config` — plus `Capabilities` (independent enrich/extract/preview
  booleans), `ExtractorConfig` (enabled + options bag, enabled by default),
  `PreviewResponse`, and the tri-state `ExtractOutcome`/`PreviewOutcome`
  enums reproducing Hister's `ExtractorSuccess`/`ExtractorFallback`/
  `ExtractorAbort` chain-of-responsibility pattern as a closed Rust enum
  (no opaque-type/factory-function indirection needed — the enum itself
  makes a fourth state unrepresentable). Synchronous by design: every
  extractor operates on an already-fetched `Document`, no I/O to make
  async worth it. The chain-of-responsibility *registry* (§4.2) is left to
  `rusty-hister-extractor`, which will depend on this trait.
- **Added:** `HisterError`, on `rusty_err` (this workspace's own
  `thiserror`+`anyhow` analog) — `InvalidConfig`, `Extraction`, and a
  `BoxError`-backed catch-all, kept deliberately small pending concrete
  failure modes from later phases rather than speculative variants.
- **Verified:** `cargo test -p rusty-hister-core --all-features` (16/16),
  `clippy --all-targets --all-features -- -D warnings`, and `fmt --check`
  all clean; the other seven `rusty-hister-*` skeleton crates still build
  against the new `rusty-hister-core` API; `check_workspace_deps.py`
  (dependency-sovereignty policy) passes.
- **Not done here:** `rusty-hister-model`, `rusty-hister-extractor`'s
  registry and concrete extractors, and `rusty-hister-crawler`'s `http`
  backend — the rest of Phase 1, tracked in `docs/roadmap/ROADMAP.md` as
  separate follow-up increments.

---

## Confirm rusty_hister's AGPL test-fixture licensing policy
**2026-09-12** · branch [`claude/confirm-hister-licensing`](https://github.com/Rusty-Mill/rusty_mill/tree/claude/confirm-hister-licensing)

Confirms the last open item from `rusty_hister`'s bootstrap (ADR-0001 §7),
on the user's (baileyrd/Nano's) direct instruction. Docs only.

- **Changed:** `crates/rusty_hister/docs/decisions/ADR-0001-…md` §7 →
  confirmed. `rusty_hister` ships under this workspace's standard `MIT OR
  Apache-2.0`; no Hister source file — test files included — is copied
  verbatim into this cluster. Fresh Rust tests are written from
  independently reading and understanding each Go test's behavior instead,
  traceable via the capability inventory's per-extractor test-file
  citations. Binds every future PR touching extractor or query-grammar
  tests, not just this bootstrap.
- **Changed:** `docs/PROJECT-STATUS.md`, `docs/roadmap/ROADMAP.md`, and
  `WORKFLOW.md` updated: Phase 0 is now fully complete (all three
  decision items resolved — crate split/scope, search/crawler approach,
  and now licensing); no items block Phase 1-4 implementation start.
- **Not done here:** no implementation of any kind — this is a licensing
  policy confirmation, not code.

---

## Decide rusty_hister's ADR-0002 and ADR-0003
**2026-09-12** · branch [`claude/decide-hister-adr-0002-0003`](https://github.com/Rusty-Mill/rusty_mill/tree/claude/decide-hister-adr-0002-0003)

Decides the two open decision-requests from `rusty_hister`'s bootstrap
(previous entry below), on the user's (baileyrd/Nano's) direct instruction
("Decide ADR-0002 and ADR-0003 now"), ahead of ADR-0002's own recommended
scoping spike — each ADR records that as an accepted risk, not a silently
skipped step. No implementation code changes; docs only.

- **Changed:** `crates/rusty_hister/docs/decisions/ADR-0002-…md` → Accepted.
  Engine: `rusty_search` + `rusty-search-sqlite-fts5`, chosen over Tantivy
  for single-SQLite-file cohesion with the model DB and with `sqlite-vec`
  (below). Multi-language `IndexAlias` federation, `url_re:` custom
  filtering, and the three highlight styles are decided as
  `rusty-hister-indexer`-layer composition over `rusty-search-core`'s
  `Query` tree, not changes to the shared `rusty_search` crate. BM25 parity
  accepted as result-set, not byte-exact. Semantic-search storage:
  vendored `sqlite-vec` C extension (SQLite path), `pgvector` (Postgres
  path) — no pure-Rust reimplementation for v1.
- **Changed:** `crates/rusty_hister/docs/decisions/ADR-0003-…md` → Accepted.
  CDP crawler backend: `chromiumoxide` as a Tier A adapter dependency (root
  ADR-0002's tiers), since Hister's own `chromedp` backend already wraps an
  external library rather than hand-rolling CDP. WebDriver BiDi: **explicitly
  descoped for v1** (not deferred) — its only advantage over CDP (no
  driver-binary/library dependency) is moot once `chromiumoxide` is already
  accepted; the Notion extractor's JS-rendering requirement is unaffected,
  satisfied by the CDP backend.
- **Changed:** `docs/PROJECT-STATUS.md` and `docs/roadmap/ROADMAP.md`
  updated: Phase 3 (indexer + vectorstore) and Phase 4 (CDP crawler
  backend) are unblocked; Phase 4's BiDi-equivalent line item is marked out
  of v1 scope rather than merely blocked. Crate module doc comments in
  `rusty-hister-{indexer,vectorstore,crawler}` updated to match.
- **Not done here:** no implementation of either decision — the indexer,
  vectorstore storage, or CDP crawler backend. That's Phase 3/4 work,
  tracked but not started.

---

## Bootstrap rusty_hister: a Rust port of asciimoo/hister
**2026-09-12** · branch [`claude/hister-rust-port-quq3ho`](https://github.com/Rusty-Mill/rusty_mill/tree/claude/hister-rust-port-quq3ho)

Bootstraps a Rust port of [asciimoo/hister](https://github.com/asciimoo/hister)
(AGPL-3.0-or-later) as a native crate cluster under `crates/rusty_hister/` —
fresh work written directly in this workspace, not a `git subtree` import of
a pre-existing standalone repo. No port implementation logic lands in this
PR; it establishes the scaffold everything else depends on.

- **Added:** eight new workspace members —
  `rusty-hister-{core,model,extractor,indexer,vectorstore,crawler,server,mcp}`
  — each an empty skeleton crate with a module doc comment pointing back to
  the capability inventory and relevant ADR. All compile clean
  (`cargo check` across the set).
- **Added:** a full `rust-migration`-style capability inventory built from
  reading Hister's actual Go source at commit `49b727f4` (not the kickoff
  brief's own rough orientation notes, which were explicitly unverified) —
  `crates/rusty_hister/docs/capability-inventory/HISTER-CAPABILITY-INVENTORY.md`.
  Covers 39 HTTP routes, 3 MCP tools (with verbatim prompt-injection-defense
  text), ~35 CLI subcommands, 20 extractors, the full query-language
  grammar, the vectorstore/embedding pipeline, 10 DB models, 3 crawler
  backends, and the TUI, each flagged `[TESTED]`/`[UNTESTED]` against
  Hister's own Go test suite.
- **Added:** a sovereignty audit confirming most of what this port needs is
  already covered by existing first-party crates — `rusty_tokio`,
  `rusty_http`/`rusty_request`, `rusty_tls`, `rusty_json`, `rusty_db`,
  `rusty_url`, `rusty_llama`/`rusty_provider`, and notably `rusty_mcp`
  (already a mature MCP server framework this port's tool surface builds on
  directly instead of a fresh JSON-RPC layer). `rusty_search` covers real
  BM25 today but only a structured query-builder DSL, not a text grammar,
  and no vector search yet. Nothing in the workspace touches the Chrome
  DevTools Protocol or WebDriver BiDi.
- **Added:** three ADRs under `crates/rusty_hister/docs/decisions/` —
  ADR-0001 (accepted: native workspace crates, v1 scope is backend-only per
  the kickoff brief, the crate split and its two revisions from the brief's
  starting suggestion, and an open licensing recommendation for Go test
  fixtures), ADR-0002 and ADR-0003 (both **Proposed**, open
  decision-requests per the kickoff brief's explicit instruction — search/
  indexing engine approach and JS-rendering crawler approach, respectively
  — neither decided in this PR).
- **Not done here:** no indexing, crawling, extraction, or server logic.
  `rusty-hister-indexer`, `rusty-hister-vectorstore`'s storage side, and
  `rusty-hister-crawler`'s JS-rendering backends are explicitly blocked on
  ADR-0002/ADR-0003 sign-off; see `crates/rusty_hister/docs/roadmap/
  ROADMAP.md` for what can proceed in parallel.

## Make rusty_json's serde dependency optional
**2026-09-11** · branch [`claude/clever-wright-y6qkyq`](https://github.com/Rusty-Mill/rusty_mill/tree/claude/clever-wright-y6qkyq)

Resumes `repo-inspector-report.md` Section 1 row 7 (hand-rolled JSON
`Value` in `rusty_oauth`/`rusty_request`), left blocked in an earlier pass
on `rusty_json` having a non-optional `serde` dependency — contradicting
the exact "no `serde`" rationale both hand-rolled crates state in their own
doc comments.

- **Added:** a `serde` Cargo feature on `rusty_json`, on by default (zero
  behavior change for its 16 existing workspace dependents — none needed
  any change, spot-checked). With `default-features = false`, the crate
  pulls in no `serde` dependency at all: `Value` parsing
  (`s.parse::<Value>()` / `Value::from_json_str`) and writing
  (`Value::to_json_string`/`Value::to_json_string_pretty`) go through a new
  direct recursive-descent path (`src/value_io.rs`) that reuses the
  existing hand-rolled tokenizer (`src/parser.rs`) and `Formatter` trait
  instead of `serde::Deserializer`/`Serializer`. String-escaping logic
  moved to a shared `src/escape.rs` so the serde-based and serde-free
  writers can't drift apart. Verified standalone: full test suite green in
  both feature configurations (`cargo test -p rusty_json` and
  `--no-default-features --features std`), plus clippy clean in both.
- **Not done here:** `rusty_oauth` and `rusty_request` still hand-roll
  their own `Value` — this PR only removes the prerequisite blocking their
  migration to `rusty_json`, which touches 14 and 4 call sites
  respectively and is left as a separate, deliberately-scoped follow-up.

---

## Migrate rusty_multimodal_db into the monorepo
**2026-09-10** · branch [`claude/loving-bell-kntf9l`](https://github.com/Rusty-Mill/rusty_mill/tree/claude/loving-bell-kntf9l)

`baileyrd/rusty_multimodal_db` — a benchmark harness comparing AoS, SoA,
and UUID-canonical-store record backends, plus the production store,
network server, and schema-driven client built on the winning design —
merged into `crates/rusty_multimodal_db/` via `git subtree`, full history
preserved. A sixth merge outside the `baileyrd/rusty_*` wave numbering
(ADR-0001), same treatment as the `nexus` merge above.

- **Added:** `crates/rusty_multimodal_db` joins this root's member list
  directly (it was already a single, non-nested `Cargo.toml`, unlike
  `nexus`/`rusty_agent_gateway`/`rusty_yirp` — nothing to de-nest).
- **Changed:** its one pinned git dependency on `rusty_tls`
  (`Rusty-Mill/rusty_mill` at a specific commit) retired to a plain path
  dependency on this workspace's own `crates/rusty_tls` — the
  same-workspace-source rule ADR-0002 requires, and the same swap
  `rusty_yirp`'s `sessionmgr-pty` and `nexus-rush` made on their own
  merges.
- **Changed:** its `rusqlite` pin (used only by the optional
  `external-db-bench` benchmark feature) bumped `0.32` → `0.39` to match
  `crates/rusty_inventrory`'s `inventory-core` — `rusqlite` declares
  `links = "sqlite3"`, and Cargo allows only one version of a
  `links`-declaring crate in the whole dependency graph; unifying on the
  higher version is the same fix the `nexus` merge's `sqlx`/`rusqlite`
  collision needed, not a behavior choice of this crate's own.
- **Fixed:** two `clippy::chunks_exact_to_as_chunks` failures
  (`src/durability/mmap_store.rs`, `src/server/pem.rs`) — this workspace's
  clippy version flags `chunks_exact(N)` with a constant `N` in favor of
  `as_chunks::<N>().0`; behavior unchanged, same trailing-partial-chunk
  drop either way. The upstream repo hit the identical failure on its own
  `main` (unrelated to this merge — its clippy toolchain updated
  independently) and carries the same fix.
- **Changed:** `rusty_multimodal_db` added to the `windows-latest`
  `windows-exclude` list alongside `rusty_stream`/`rusty_fedora_agent` —
  its optional `external-db-bench` feature's `duckdb` dependency vendors
  DuckDB's own C++ amalgamation, and this workspace's `--all-features` is
  what first compiles it on `windows-latest`; that native build fails
  under the runner's current MSVC toolchain (a third-party build issue,
  no Rust-side fix available here). The upstream repo's own CI never ran
  a Windows job at all, so this wasn't a regression, just first exposure.
- **Verified:** `cargo tree -p rusty_multimodal_db --all-features`
  resolves to one `rusqlite v0.39.0`; `cargo check -p rusty_multimodal_db
  --features research,server,perf-events` compiles clean, and separately
  `--features research,external-db-bench` compiles clean too (DuckDB's
  bundled from-source build, already a documented one-time cost — see
  this crate's own `docs/decisions/ADR-0015-external-database-benchmark.md`
  — checked on its own given how long that build takes, ~10.5 minutes).

## Migrate nexus into the monorepo
**2026-09-08** · branch [`claude/nexus-rusty-mill-migration-ic3fqa`](https://github.com/Rusty-Mill/rusty_mill/tree/claude/nexus-rusty-mill-migration-ic3fqa)

`baileyrd/nexus` — a 42-crate microkernel note-taking/AI-agent workspace —
merged into `crates/nexus/` via `git subtree`, full history preserved. A
fifth merge outside the `baileyrd/rusty_*` wave numbering (ADR-0001).

- **Added:** all 42 of nexus's workspace crates join this root's member
  list directly; `crates/nexus/shell` (its Tauri desktop shell) is
  `exclude`d the same way as `rusty_key`'s `desktop/src-tauri`.
- **Changed:** nexus's own nested `[workspace]`, `Cargo.lock`, and
  `.cargo/config.toml` (a `target-dir` override that would have
  fragmented this workspace's shared `target/` for anyone building from
  inside `crates/nexus/`) were dropped on merge, the same treatment
  rusty_agent_gateway's and rusty_yirp's own nested workspaces got.
  nexus's `rustils` git pins (`platform`/`platform-linux`) retired to
  this root's existing path dependencies (ADR-0002) — the same swap
  rusty_yirp's `sessionmgr-pty` made.
- **Changed:** `sqlx` bumped `0.8` → `0.9` workspace-wide to resolve a
  hard `libsqlite3-sys` version collision between `sqlx-sqlite` (used by
  `rusty_db`'s SQLite driver and `rusty_acp`'s optional Postgres store)
  and the `rusqlite 0.39` nexus's storage layer needs — both crates
  register `links = "sqlite3"`/pull `libsqlite3-sys`, and Cargo allows
  only one version of a `links`-declaring crate in the whole graph.
  `rusqlite` itself bumped `0.32.1` → `0.39` at the root for the same
  reason; `inventory-core`, `rusty_sqlite`, and `rk-feed` (each
  previously pinned lower for their own documented reasons, all still
  satisfied at `0.39`) now share that one version too. sqlx 0.9's new
  `SqlSafeStr` injection-audit bound needed `sqlx::AssertSqlSafe` wraps
  at every dynamic-SQL call site in `rusty-db-postgres`/`-mysql`/
  `-sqlite` and `rusty_acp`'s postgres store — all of them build SQL
  from a config-supplied table prefix or a caller-supplied connection
  hook, never request data, so each wrap is an audit assertion, not a
  behavior change. This was surfaced to, and approved by, the user
  before making the bump (a toolchain/dependency change, per the
  working agreement) rather than picked unilaterally.
- **Fixed:** two latent nexus-side gaps that its own CI never exercised,
  only surfaced once this workspace's `--all-features` build compiled
  them: `nexus-memory`'s `Memory`/`MemoryType`/`MemoryStatus` never
  derived `TS`/`JsonSchema` despite being embedded in a `ts-export` IPC
  type (`nexus-memory` isn't in nexus's own `check_ipc_drift.sh` build
  list); `nexus-rush`'s job-spawn `Command` literal was missing a
  `detached` field added to this workspace's `rustils` fork after
  nexus's former pinned rev.
- **Verified:** `cargo check --workspace --all-features` across all 230
  workspace members (42 of them new from nexus), `cargo fmt --all -- --check`,
  and `.github/scripts/check_workspace_deps.py` (ADR-0002 dependency-policy
  check) all pass clean.

## Work through the repo-inspector report's duplication and sovereignty rows
**2026-09-05** · branch [`claude/repo-inspector-report-wgoocq`](https://github.com/Rusty-Mill/rusty_mill/tree/claude/repo-inspector-report-wgoocq)

Every row of `repo-inspector-report.md` (regenerated in #152) now has a
recorded disposition in a new **Disposition** section at the top of the
report; the actionable ones landed here.

- **Added:** `crates/rusty_rand` — one dependency-free OS CSPRNG
  (`/dev/urandom` cached handle / `BCryptGenRandom`) replacing three
  identical copies in `rusty_oauth`, `rusty_uuid`, and `sessionmgr-proc`
  (the third was not in the report; it was indexed under `os_random`).
  Public APIs of all three consumers are unchanged.
- **Added:** `rusty_simd::f32_to_f16`; `rusty_llama` and `rusty_whisper`
  re-export both f16 directions from `rusty_simd` and delete their own.
  The two deleted `f32_to_f16` copies disagreed (half-up rounding vs.
  ties-to-even; one mapped NaN to infinity) — both were test-fixture-only,
  so no shipped code path changes.
- **Added:** `rusty_wiremock::canned` behind a `std` feature — the
  sequential canned-response mock server that `rusty_proxmox`,
  `rusty_opnsense`, `rusty_fedora`, and `rusty_homelab_mcp` each carried
  an identical copy of; all four now dev-depend on it. `rusty_wiremock`
  was the home each copy's own doc comment named while calling it a stub.
- **Changed:** `rusty_base64` decodes strictly (misplaced/excess padding
  and non-4-aligned padded input are errors; `DecodeError` carries
  positions) so `sessionmgr-protocol` could adopt it without giving up its
  "reject, never guess" rule. With that, the last three hand-rolled base64
  copies and the last four external `base64` users are gone: no workspace
  manifest declares external `base64` any more.
- **Changed:** `adk-core` → `rusty_uuid`; `rk-feed` → `rusty_url`
  (direct edge only — `reqwest` keeps external `url` transitively).
- **Not done, with reasons recorded in the report:** row 7 (JSON `Value`)
  is blocked because `rusty_json` has a non-optional `serde` dependency,
  which is the one thing `rusty_oauth`/`rusty_request` hand-roll to avoid
  — a serde-free `rusty_json` surface is the prerequisite. `rusty-acp`/
  `rusty-db-core` keep external `uuid` (serde + `sqlx` type mapping on the
  `Uuid` type); `sessionmgr-proc` keeps `libc` (same Track P
  dual-backend decision as #120); `rustls` in `agentgateway-tls`/
  `rp-router` and `rusqlite` in four crates were checked for fit against
  `rusty_tls`/`rusty_sqlite` and there is none today; `toml` needs a serde
  `Deserializer` and `[[array-of-tables]]` in `rusty_codec` first. Row 2
  (retry) turned out narrower than reported: `rusty_request` and
  `rusty_acp` already delegate backoff to `rusty_retry`.
- **Verified:** `cargo test` on every touched crate plus every
  `rusty_base64::DecodeError` consumer (`rusty_a2a`, `rusty-acp`,
  `rusty-mcp`) and `sessionmgr-daemon` (the one test that drives a real
  `claude` binary was skipped as environment-dependent); `cargo fmt --all
  -- --check`; `cargo clippy --all-targets --all-features -- -D warnings`
  on all touched crates; `.github/scripts/check_workspace_deps.py` clean.

---

## Add a Fedora/systemd module to rusty_homelab_mcp
**2026-09-04** · branch [`claude/fedora-systemd-module-hb7vxv`](https://github.com/Rusty-Mill/rusty_mill/tree/claude/fedora-systemd-module-hb7vxv)

- **Added:** `crates/rusty_fedora_agent` — an unprivileged local agent for
  a Fedora Server host with no REST management API of its own (e.g.
  baileyai). Exposes `fedora_system_status`/`fedora_list_services`/
  `fedora_service_control`/`fedora_read_journal`/`fedora_dnf_list_updates`/
  `fedora_dnf_install`/`fedora_dnf_remove`/`fedora_task_status`/
  `fedora_read_config`/`fedora_write_config`'s ten operations over a small
  synchronous HTTP API (`tiny_http`), built on `rustils`'
  `platform`/`platform-linux` `Spawner`/`Command` for `systemctl`/
  `journalctl`/`dnf`, with `SystemController`/`PackageController` domain
  ports so tool-handling logic can be tested against `platform-mock`'s
  scripted spawner without a real Fedora box.
- **Added:** `crates/rusty_fedora` — async typed client for that agent's
  HTTP API, matching `rusty_opnsense`/`rusty_proxmox`'s shape exactly
  (built on `rusty_request`, passthrough JSON, no MCP dependency).
- **Added:** `rusty_homelab_mcp` gained a `fedora` module: the ten tools
  above, following the existing OPNsense/Proxmox discovery-then-mutate
  pattern and `$defs` oneOf-enum style (`ServiceActionArg`, `UnitTypeArg`,
  `PriorityArg`) exactly. `HomelabServer::new` now takes a third,
  independent, optional `FedoraAgentClient`.
- **Not a new repo:** the handoff brief for this task assumed
  `rusty_homelab_mcp`/`rustils` were still separate GitHub repos (as they
  were before this monorepo's consolidation) and proposed a new
  standalone `rusty_fedora_agent` repo. Verified against the actual repo
  before writing anything and built it as a workspace crate instead —
  `ARCHITECTURE.md` documents this monorepo deliberately consolidating
  ~90+ formerly-independent repos for one CI/one place to de-duplicate,
  and a new standalone repo would fight that policy on day one.
- **Privilege scoping is deliberately not applied automatically:**
  `crates/rusty_fedora_agent/deploy/` ships a systemd unit, a polkit rule
  (unit allowlist), a sudoers `NOPASSWD` entry (`dnf install`/`remove`
  only, package-name scoping enforced inside the agent before `dnf` is
  ever invoked), and an allowlist config — all reviewable templates a
  human applies to the target host by hand, with an **empty** allowlist
  by default (nothing permitted until deliberately added). This session
  has no access to the real target host (baileyai), so the "real
  happy-path run against baileyai" a full rollout calls for is left to
  whoever applies `deploy/`.
- **Verified:** `cargo check --workspace` and `cargo test -p
  rusty_fedora_agent -p rusty_fedora -p rusty_homelab_mcp` (allowlist
  rejection tests, scripted-spawner happy-path tests, and mock-HTTP MCP
  tool-dispatch tests).

---

## Fix `rusty_tokio`'s Windows reactor orphaning a socket on a failed AFD re-arm
**2026-09-03** · branch [`claude/rusty-meshed-crate-migration-zy7k1n`](https://github.com/Rusty-Mill/rusty_mill/tree/claude/rusty-meshed-crate-migration-zy7k1n)

- **Fixed:** `crates/rusty_tokio/src/io/reactor/windows.rs`'s `event_loop`
  discarded the result of resubmitting a socket's one-shot
  `IOCTL_AFD_POLL` after every completion (`let _ =
  self.submit_poll(&state)`, at both call sites). If that resubmission
  itself failed, no further completion would ever arrive for that
  socket, so a `readable()`/`writable()` wait registered on it
  afterward hung forever with nothing left to wake it — observed as
  four unrelated `rusty_tokio`/`rusty_tls` tests intermittently timing
  out at nextest's ~600s slow-timeout on `test (windows-latest)`, then
  passing in milliseconds on the very next retry ([#153](https://github.com/Rusty-Mill/rusty_mill/issues/153)).
- **Why not the earlier readiness-edge fix ([#140](https://github.com/Rusty-Mill/rusty_mill/pull/140)):** all four
  occurrences happened on runs *after* #140 had already merged, and its
  fix targets a different failure shape (a bit cleared out from under a
  fresh edge) than this one (no bit update ever happens again because
  nothing is watching the socket anymore).
- **How:** both re-arm call sites now check `submit_poll`'s result and,
  on failure, mark both directions ready via a new `mark_orphaned`
  helper — the same "surface both directions so the caller's own next
  syscall discovers the truth" pattern `event_loop`'s sibling
  bad-completion-status branch already used for a different failure
  mode, extended to cover this one.
- **Verified:** `cargo check`/`clippy -D warnings` clean on
  `x86_64-pc-windows-gnu` (cross-compiled from this Linux sandbox, which
  cannot execute Windows tests); the full Linux `rusty_tokio` suite
  passes unaffected (the changed code is `#[cfg(windows)]`-only). Real
  verification comes from `windows-latest` CI itself, the same oracle
  #137/#138/#140 relied on.
- **Known limitation — partial fix:** the `windows-latest` CI run on
  this very branch (33784080891) reproduced the identical hang
  signature on `rusty_tls::async_handshake::async_handshake_succeeds_and_round_trips_with_pinned_anchor`
  (TRY 1 timing out at ~600s, TRY 2 passing instantly) with this fix
  already applied. So this change is real and worth keeping — it closes
  a genuine silently-swallowed-error hole — but it does not fully
  resolve #153. #153 stays open, narrowed to the still-unexplained
  remainder.

---

## Extract `rusty_base64`; close issue #119
**2026-09-03** · branch [`docs/rusty-base64-extraction-issue-119`](https://github.com/Rusty-Mill/rusty_mill/tree/docs/rusty-base64-extraction-issue-119)

- **Added:** `rusty_base64` — `rusty_oauth::encoding::base64`'s complete
  surface (encode/decode, standard and URL-safe alphabets) extracted into
  its own crate, per [issue #119](https://github.com/Rusty-Mill/rusty_mill/issues/119).
  `rusty_request`'s own `base64.rs` was ruled out as a base: it's private,
  encode-only, and standard-alphabet-only, and extending it would have
  meant building a second base64 crate when `rusty_oauth`'s already
  covered the need.
- **Changed:** `rusty_oauth` now depends on `rusty_base64` too
  (dogfooding) instead of keeping its own copy — its public
  `encoding::base64::*` path is unchanged, so none of its own call sites
  needed edits. `rusty_acp`, `rusty-mcp`, and `rusty_a2a` swapped their
  external `base64` crate dependency for `rusty_base64` after checking
  each call site's exact API needs (standard vs. URL-safe, padded vs.
  unpadded, encode vs. decode) rather than assuming a blind swap would fit
  — the same per-crate verification issue #119 itself asked for.
- **Fixed:** the extraction rewrites `encode_with`/`decode_with`'s
  chunking from `slice::as_chunks` (`rusty_oauth`'s original) to
  `chunks_exact`/`remainder`. `as_chunks` is not yet stable at
  `rusty_acp`'s own `rust-version = "1.86"` floor — confirmed against a
  real `+1.86` toolchain before merging, since `rusty_acp`'s own CI
  convention runs `cargo +1.86 test` and this would have silently broken
  it. Behaviorally identical (verified via the original RFC 4648 test
  vectors under both a `+1.86` and the workspace's default toolchain).
- Known limitation: `rusty_croc`, `adk-a2a`, `agentgateway-auth`, and
  `agentgateway` still depend on the external `base64` crate. They weren't
  in issue #119's verified evidence (filed before three of them joined the
  workspace) and weren't checked here — left for separate follow-up rather
  than swapped without per-call-site verification.

---

## Fix `sessionmgr-pty`'s intermittent size-reporting flake
**2026-09-03** · branch [`claude/rusty-meshed-crate-migration-zy7k1n`](https://github.com/Rusty-Mill/rusty_mill/tree/claude/rusty-meshed-crate-migration-zy7k1n)

- **Fixed:** `LinuxPty::spawn` (`crates/rustils/crates/platform-linux`) set
  a session's pty window size *after* spawning the hosted child, so the
  child could run (and, in `sessionmgr-pty`'s size-reporting test, read
  its own terminal size via `stty size`) before the parent's
  `TIOCSWINSZ` ioctl took effect — a race that had been intermittently
  failing `sessionmgr-pty::tests::the_terminal_reports_the_size_it_was_given`
  on `main`'s `test (ubuntu-latest)` CI job (silently absorbed by
  nextest's retry budget on most runs, then hard-failing it outright on
  [PR #149](https://github.com/Rusty-Mill/rusty_mill/pull/149)), tracked
  as [#150](https://github.com/Rusty-Mill/rusty_mill/issues/150). Fixed
  by setting the size on the pty master before the child is spawned,
  closing the race. `platform`/`platform-linux`/`platform-windows`/
  `platform-mock`/`platform-bsd`/`platform-parity` bumped `0.27.0` →
  `0.27.1` (patch-level: no public API shape changed).

---

## Dependency sovereignty policy (ADR-0002) and the last workspace-member git pins
**2026-09-03** · branch [`claude/review-recommended-changes-8kepe6`](https://github.com/Rusty-Mill/rusty_mill/tree/claude/review-recommended-changes-8kepe6)

- **Added:** `docs/adr/0002-dependency-sovereignty-policy.md` — a three-tier
  classification (Sovereign / Transitional / Adapter) for how a crate's
  external dependencies relate to the workspace's dependency-minimizing
  purpose, written in response to an external Atlas-alignment review that
  found "no external dependencies" does not describe the monorepo as a
  whole (114 of 199 manifests declare a direct external normal dependency).
  A generated, per-crate ledger cross-referencing manifests to tiers is
  tracked as follow-up, not introduced here.
- **Fixed:** `crates/rusty_term/l13`, `crates/rusty_font`, and
  `crates/rusty_gpu` depended on `rusty_lsp`/`rusty_simd` via a pinned git
  URL even though both are workspace members with their own `crates/<name>`
  directory, letting the git and workspace copies silently diverge —
  contrary to `ATLAS-RWC-0050`. All three now use plain path dependencies.
- **Added:** `.github/scripts/check_workspace_deps.py` (with unit tests) and
  a new `dependency-policy` CI job that fails a PR if any workspace member's
  name resolves from a git source anywhere in the dependency graph —
  confirmed to catch the exact violation above by running it against the
  pre-fix manifests.
- Known limitation: `main` is still unprotected on GitHub (no required
  status checks, no branch protection rule), so this new CI job — like the
  rest of `ci.yml` — is not yet a merge gate. That requires a repository
  admin action outside what this PR's tooling can perform; see the Atlas
  review's `ATLAS-TOOL-0010`/`0011` findings.

## Review policy: author self-review when no independent reviewer is available
**2026-09-03** · branch [`claude/assessment-review-corrections-nac84f`](https://github.com/Rusty-Mill/rusty_mill/tree/claude/assessment-review-corrections-nac84f)

- **Changed:** `CONTRIBUTING.md` said "at least one approval required" while
  every PR merged on 2026-09-02 was authored, self-merged, and unreviewed
  by the same account, which the Atlas evidence review (`docs/atlas/`)
  recorded as an unenforced policy. The policy now matches practice
  honestly: an independent approval when a reviewer is reasonably
  available, otherwise a recorded author self-review against the reviewer
  checklist after CI is green. The PR description must say no independent
  reviewer was available; self-review is never represented as independent
  review (the distinction Atlas `ATLAS-GOV-REVIEW-0061`/`0064` draws).
  Security-sensitive, irreversible, or ecosystem-breaking changes still
  wait for an independent reviewer when one can be found.
- **Changed:** the four PR templates gain a checklist line — "Reviewed:
  independent approval, or self-review recorded in the description" — so
  the record is made on every PR rather than remembered.
- Known limitation: this is documented policy, not enforcement. `main`
  is still unprotected, so nothing stops a merge that skips the record.

## Retire the last `rustils` git pins to path dependencies
**2026-09-02** · branch [`claude/assessment-review-corrections-nac84f`](https://github.com/Rusty-Mill/rusty_mill/tree/claude/assessment-review-corrections-nac84f)

- **Changed:** `rusty_tokio`, `rusty_tls`, and `rustils_async` (root manifest
  plus `platform-async`, `platform-async-mock`, `platform-async-linux`,
  `coreutils-async`) depended on `platform`/`platform-linux`/
  `platform-bsd`/`platform-windows`/`platform-mock` through rev-pinned
  `git` dependencies on `baileyrd/rustils`, left over from before
  `rustils` joined this workspace. All twenty-one declarations are now
  `path` dependencies on `crates/rustils/crates/<name>`, the same
  retirement every other first-party pin got when its crate merged.
- **Changed:** `Cargo.lock` drops thirteen git-sourced `platform*` entries
  (three checkouts: two at 0.27.0, `rusty_tls`'s at 0.22.1). Each crate now
  resolves to one in-tree 0.27.0 instance, so consumers share one
  `platform::error::PlatformError` type instead of one per checkout.
- **Verified:** `cargo check`, `cargo clippy -D warnings`, and `cargo test`
  with `--all-features --all-targets` across the six consumers on Linux;
  the Windows and BSD backends are compiled only on their targets, so the
  Windows leg of CI is the evidence for `platform-windows` and nothing
  here exercises `platform-bsd`.
- Known limitation: `rusty_tls` moves from platform 0.22.1 to 0.27.0 in one
  step. It compiles and its tests pass, which is the check the versioning
  rule asks for, but any behavioural change in those five minor versions
  reaches `rusty_tls` with this merge.

## rusty_meshed: reverse-trace & domain-maturity crate
**2026-09-02** · branch [`claude/rusty-meshed-crate-migration-zy7k1n`](https://github.com/Rusty-Mill/rusty_mill/tree/claude/rusty-meshed-crate-migration-zy7k1n)

- **Added:** `crates/rusty_meshed/crates/rusty-meshed-trace` -- the core of
  the *Reverse-Trace & Domain Maturity* spec (Phase 1): the five-level
  `Maturity` ladder, `Domain`/`Source`/`Outcome`/`Requirement` types, a pure
  `trace()` that classifies every requirement (satisfied / blocked / degraded
  / missing), caps the outcome's fidelity at its weakest required domain and
  returns a worst-first bottleneck list, TOML scenario loading via
  `rusty_codec`'s sovereign parser, JSON round-tripping via `rusty_json`, a
  Markdown "gap summary" export, and one shipped scenario (*Acquisition
  Status Dashboard*, ten domains, four outcomes). Fifteen fixture tests cover
  every verdict and edge class, ordering, what-if, and both file formats.
- **Changed:** the crate is a new workspace member; `rusty_meshed/README.md`
  gains a crate-table row and a section on the new capability.
- Known limitation: the shipped scenario's maturity levels are illustrative
  placeholders (spec open question #3), not an assessment; the renderer
  (Phase 2) lives in the source repo's `data-mesh-monitor`, not here.

## Chore: drop committed Python bytecode, ignore it going forward
**2026-09-02** · branch [`claude/assessment-review-corrections-nac84f`](https://github.com/Rusty-Mill/rusty_mill/tree/claude/assessment-review-corrections-nac84f)

- **Fixed:** the previous PR ran the new `.github/scripts` unit tests
  locally before staging and swept two `__pycache__/*.pyc` files into the
  commit. Removed from the tree; `__pycache__/` and `*.pyc` added to
  `.gitignore` so it cannot recur.

## CI: unit tests for the affected-crates plan step
**2026-09-02** · branch [`claude/assessment-review-corrections-nac84f`](https://github.com/Rusty-Mill/rusty_mill/tree/claude/assessment-review-corrections-nac84f)

- **Added:** `.github/scripts/test_affected_crates.py` plus a `plan-tests`
  CI job. The plan step decides what every other job runs on a PR, and the
  Atlas review flagged that its nested-crate ownership and
  reverse-dependency traversal had no regression tests. Thirteen cases
  cover: a file inside a crate, outside every crate, the manifest itself,
  a nested crate winning over its parent, `crates/foo` not claiming
  `crates/foobar`, direct and transitive dependents, leaf changes not
  pulling in dependencies, non-workspace dependencies ignored, cyclic
  dev-dependency graphs terminating, sorted/deduplicated output, and a
  member missing from the resolve graph.
- **Changed:** `affected_crates.py`'s graph logic moved into
  `affected_packages(metadata, changed_files)` with type hints; the CLI
  contract (metadata path in argv, changed files on stdin, names on
  stdout) is unchanged and was checked against the real workspace metadata
  for four representative inputs.
- Known limitation: the tests use synthetic metadata; the end-to-end check
  that CI actually scopes to the right crates remains PR #68's round-trip
  test.

## Docs: repository-map corrections from the Atlas review
**2026-09-02** · branch [`claude/assessment-review-corrections-nac84f`](https://github.com/Rusty-Mill/rusty_mill/tree/claude/assessment-review-corrections-nac84f)

- **Fixed:** README relationship text that the Atlas review found stale:
  `rusty_simd` was described as the one crate "still outstanding" while
  merged and listed as a member; `rusty_tokio` was described as having no
  in-repo dependents while nineteen workspace packages depend on it by
  `path`; `rustils` was described as outside the monorepo's scope while
  living at `crates/rustils`. The surviving `git` pins on `rustils` in
  `rusty_tokio`, `rustils_async`, and `rusty_tls` are now stated as
  outstanding rather than implied to be by design.
- **Fixed:** `ARCHITECTURE.md` described ATLAS-300 as a seed too draft to
  cite; it is an active volume since Atlas ADR-0006. The section now points
  at `docs/atlas/` for the requirement-by-requirement crosswalk.
- **Fixed:** the Atlas review's `rusty_tokio` dependent count, which was
  taken from a manifest grep that matched a comment in `rusty_proxmox`;
  now taken from `cargo metadata --all-features` at the evidence revision.
- Known limitation: docs only. The `rustils` pin retirement itself is not
  done here.

## Atlas evidence review — revision 2 with corrections
**2026-09-02** · branch [`claude/assessment-review-corrections-nac84f`](https://github.com/Rusty-Mill/rusty_mill/tree/claude/assessment-review-corrections-nac84f)

- **Added:** `docs/atlas/rusty-mill-atlas-evidence-review.md` — the review
  of this monorepo as exercised evidence for the Atlas Engineering
  Standards Library, revised after every claim was verified against
  Rusty Mill `06ca8669`, the live PR/branch state, and Atlas `390d6b0f`.
  Concludes that ATLAS-300's deferred feature-flag trigger fired (PRs #134
  and #136), and lists the governance corrections this repo needs before
  any conformance claim: protect `main`, enforce the documented review
  policy, and fix the stale README/ARCHITECTURE map.
- **Added:** `docs/atlas/rusty-mill-atlas-evidence-review-corrections.md`
  — the must-fix and should-fix items found in the review's first
  revision, each with the evidence that established it.
- Known limitation: the review is an alignment assessment, not a
  certification, and PR #131 (still open) is excluded from its evidence
  revision.

## Fourth-wave merge — `rusty_agent_gateway` (wave complete)
**2026-09-01** · branch [`claude/rusty-repos-migration-iwvuld`](https://github.com/Rusty-Mill/rusty_mill/tree/claude/rusty-repos-migration-iwvuld)

- **Added:** `crates/rusty_agent_gateway` — a Rust implementation of the
  agentgateway data plane, built as a drop-in for its `config.yaml`:
  configuration, listeners, route matching, policies, and the MCP gateway
  (several upstream MCP servers federated behind one endpoint, with
  tool-level filtering and authorization). Nine crates behind one nested
  workspace, merged via `git subtree` with full history. This completes the
  fourth wave and the monorepo consolidation.
- **Changed:** four pins retired, the most of any crate in this series, all
  to merged siblings — `rusty_a2a` (rev `b9778e1`, 11,324/360 behind),
  `rusty-mcp` (tag `v0.4.1`, 1,367/210 behind — the only tag-pinned
  dependency in the series), `rusty_tls` (rev `7ac6956e`, 109/27) and
  `rusty_tokio` (rev `6d3bb05a`, 3,158/587). The last two had to move
  together by construction, the same `AsyncRead`/`AsyncWrite` trait-identity
  constraint `rusty_request`'s retirement documented.
- **Changed:** this root's `rusty_a2a` and `rusty-mcp` entries now carry
  `default-features = false`, because a member inheriting a workspace
  dependency may not set it when the root does not — and the gateway's
  crates set it deliberately. Verified to be a no-op for their other
  consumers (`adk-a2a`, `rp-mcp`, `rp-server`): both crates' `default`
  feature is empty.
- **Fixed/Changed:** `[workspace.package]` collided on five fields, so its
  crates carry literal `[package]` fields; its `[workspace.lints]` is
  stricter than this root's (`unsafe_code = "forbid"`, `missing_docs`,
  `clippy::todo`, `clippy::unwrap_used`) and is written literally into each
  crate rather than silently downgraded — same call as `rusty_key`'s. Root
  `clap` gained `env`.
- **Known limitation (pre-existing, unchanged):** `hyper`'s `http2` feature
  is load-bearing for the shipped `agentgateway` binary (its TLS listener
  advertises `h2` over ALPN) but Cargo's feature unification means the test
  binary has it regardless — so it must be verified against a built binary
  with `curl`, not by `cargo test`, exactly as before the merge.
- 73 tests pass. No lint or format fixes were needed.

## Fourth-wave merge — `rusty_yirp`
**2026-09-01** · branch [`claude/rusty-repos-migration-iwvuld`](https://github.com/Rusty-Mill/rusty_mill/tree/claude/rusty-repos-migration-iwvuld)

- **Added:** `crates/rusty_yirp` — sessionmgr, a Windows-native session
  manager for AI coding-agent CLIs (Claude Code, Codex, Gemini CLI): each
  session optionally in its own git worktree, a TUI grid dashboard, and
  sessions that survive the manager closing. Eight `sessionmgr-*` crates
  plus a Tauri 2 desktop shell behind one nested workspace, merged via
  `git subtree` with full history.
- **Changed:** four pins retired — `rusty_tokio` (rev `6e6f1847`, from its
  own `[workspace.dependencies]`) and `sessionmgr-pty`'s `platform`,
  `platform-linux`, `platform-windows` (`rustils` rev `ce9259d4`) — all now
  this root's path entries. `sessionmgr-pty`'s manifest warned that a
  second, differing `rustils` pin would build two non-interoperating copies
  of the platform layer; this wave merged a third consumer
  (`rusty_tailscale`, at a different rev again), and a `path` dependency
  settles that by construction.
- **Changed:** `[workspace.package]` collided on `rust-version` and
  `license`, so its crates carry literal `[package]` fields.
- **Known limitation:** `sessionmgr-daemon`'s
  `a_fresh_claude_session_reaches_needs_input_on_its_own` drives a real
  `claude` session and skips when `claude` is not on `PATH` — the state of
  a CI runner. On a machine with the CLI installed but no way to complete
  its interactive trust prompt, the guard passes and the test times out.
  Same class as `mill-term`'s known environment-dependent failure. With
  `claude` off `PATH` the suite is 130/130.
- All eight non-Tauri crates cross-compile for `x86_64-pc-windows-gnu`. No
  lint or format fixes were needed.

## Fourth-wave merge — `rusty_provider`
**2026-09-01** · branch [`claude/rusty-repos-migration-iwvuld`](https://github.com/Rusty-Mill/rusty_mill/tree/claude/rusty-repos-migration-iwvuld)

- **Added:** `crates/rusty_provider` — an AI provider router: one
  OpenAI-compatible HTTP API in front of OpenAI, Anthropic, Gemini, Groq,
  Together AI and Fireworks, with config-driven fallback chains, budgets,
  metrics, an MCP surface and a CLI. Six crates behind one nested
  workspace, merged via `git subtree` with full history.
- **Changed:** its branch-tracking `rusty-mcp` git dependency retired to a
  `path` dependency on the merged sibling. It had resolved to `ee6c7637` —
  six commits behind the commit this workspace imported, plus two since.
  Verified by running the group's full suite (905 tests) against the swap.
- **Changed:** `[workspace.package]` collided on `license`, so its crates
  carry literal `[package]` fields. Root `reqwest` gained `stream` (SSE
  deltas from upstream providers) and root `tokio` gained `full`, declared
  at the root because Cargo unifies features across the graph either way.
- 905 tests pass. No lint or format fixes were needed — `rusty_provider`
  is the first crate group in this wave to arrive already clean under this
  workspace's `-D warnings` gate and `cargo fmt`.

## Fourth-wave merge — `rusty_adk`
**2026-09-01** · branch [`claude/rusty-repos-migration-iwvuld`](https://github.com/Rusty-Mill/rusty_mill/tree/claude/rusty-repos-migration-iwvuld)

- **Added:** `crates/rusty_adk` — a Rust port of the Agent Development Kit
  (ADK) 2.0 architecture: the same data model, graph execution engine, and
  tool/callback contracts, plus MCP and A2A bridges. Eleven library crates
  and three runnable examples behind one nested workspace, merged via
  `git subtree` with full history.
- **Changed:** `adk-a2a`'s `rusty_a2a` dependency retired from a
  *branch-tracking* git dependency (no `rev`, unlike every other pin in
  this series) to a `path` dependency on the merged sibling. What it had
  actually resolved to was 42 commits behind the commit this workspace
  imported — 9,729 insertions across 66 files — plus three commits since,
  one of which changed `require_auth`'s error type. Verified by running
  `adk-a2a`'s own suite against the swap (13 unit, 8 end-to-end, 10
  remote-transport tests), not by reading the diff.
- **Changed:** `[workspace.package]` collided on `rust-version`, `license`
  and `repository`, so its crates carry literal `[package]` fields. Root
  `tokio` gained `io-std`, `uuid` gained `serde`, and `serde_json` gained
  `float_roundtrip` (which `rusty_adk`'s SQLite session store needs for
  exact f64 round-trips, and which Cargo unifies globally anyway, so it is
  declared where it is visible). `thiserror` and `schemars` stay literal on
  the `adk-*` crates — `"2"` and `"0.8"` against this root's `"1"` and
  `rusty_key`'s `"1.0.4"`.
- **Fixed:** `adk-sessions`' optional `rusqlite = "0.37"` moved to this
  root's `"0.32.1"` — the same `libsqlite3-sys` `links` conflict
  `inventory-core` hit, since Cargo's uniqueness check counts optional
  dependencies it never activates.
- 278 tests pass, 2 ignored. No lint or format fixes were needed.

## Fourth-wave merge — `rusty_tailscale`
**2026-09-01** · branch [`claude/rusty-repos-migration-iwvuld`](https://github.com/Rusty-Mill/rusty_mill/tree/claude/rusty-repos-migration-iwvuld)

- **Added:** `crates/rusty_tailscale` — a sovereign pure-Rust Tailscale
  client: ts2021 control plane, WireGuard data plane, DERP/STUN/disco NAT
  traversal, a userspace smoltcp stack, a daemon and a CLI. Fifteen `ts-*`
  crates plus `xtask` behind one nested workspace (whose `members` was a
  `crates/*` glob, expanded to literal entries here), merged via
  `git subtree` with full history.
- **Changed:** `[workspace.package]` collided on `version`, `edition`
  (2024 — the first edition-2024 crates here) and `repository`, so its
  crates carry literal `[package]` fields; only its dependencies were
  hoisted, with `rusty_http`/`rusty_crypto_key` re-pathed to this root's
  `crates/` layout.
- **Changed:** `ts-magicsock` and `ts-tun`'s pinned `rustils` git
  dependencies (`platform`, `platform-linux`, rev `b8bf992f`) retired to
  this root's path entries, same as `rusty_rdp`'s.
- **Fixed:** two pre-existing breaks in `rusty_tailscale`'s own `main` —
  it has no CI of its own and does not compile on Linux. `ts-magicsock`
  called three `platform::net::UdpSocket` trait methods without the trait
  in scope (true at the pinned rev too, so not drift the pin was hiding),
  and `ts-cli`'s `localapi::Error` declared a `Status(StatusCode)` variant
  nothing constructs while `request()` constructed a nonexistent
  `Api { status, body }`. Both confirmed against the standalone repo first.
- **Fixed:** four more `generic-array` 0.14.9 deprecations (`ts-control`'s
  Noise handshake and frame codec, `ts-disco`, `ts-derp`), plus a
  `cargo fmt --all` pass.
- 93 tests pass. All sixteen crates also cross-compile for
  `x86_64-pc-windows-gnu` despite the Linux-first design, so — unlike
  `rusty_stream` — no `windows-exclude` was needed.

## Fourth-wave merge — `rusty_llama`
**2026-09-01** · branch [`claude/rusty-repos-migration-iwvuld`](https://github.com/Rusty-Mill/rusty_mill/tree/claude/rusty-repos-migration-iwvuld)

- **Added:** `crates/rusty_llama` — a from-scratch Llama/GGUF inference
  engine: CPU SIMD kernels, optional `wgpu` and CUDA backends, an
  OpenAI-compatible server, and GGUF-embedded Jinja chat templating. Merged
  via `git subtree` with full history.
- No dependency swaps: its `rusty_simd`/`rusty_std` path dependencies
  already pointed at siblings under `crates/`.
- **Fixed:** two `unnecessary_cast` lints in `backend/cuda.rs`'s test
  fixtures. They only appear with the `cuda` feature on, which this
  workspace's `--all-features` clippy gate does and the crate's own CI
  never did.
- **Fixed:** `render_jinja_threads_context_variables` asserted a bool
  interpolates as `true`. `minijinja` 2.22 deliberately changed
  none/bool rendering to `None`/`True`/`False` for Jinja2 compatibility;
  the standalone lockfile pinned 2.21, this workspace resolves 2.24. Since
  the code path exists to render templates authored for Python Jinja2, the
  new rendering is the correct one — assertion updated, reason recorded
  inline. Nothing else in the crate interpolates a bare boolean.
- **Changed:** reformatted with `cargo fmt --all` (not fmt-clean under this
  workspace's settings).
- 248 tests pass; 49 stay ignored because they need real model weights on
  disk, by the crate's own design.

## Fourth-wave merge — `rusty_key`
**2026-09-01** · branch [`claude/rusty-repos-migration-iwvuld`](https://github.com/Rusty-Mill/rusty_mill/tree/claude/rusty-repos-migration-iwvuld)

- **Added:** `crates/rusty_key` — Rusty Keys, an AI-native application
  skeleton where the model's agent loop is the kernel and the application
  is the harness around it (constrain / feed / observe / compose). Eight
  crates behind one nested workspace, merged via `git subtree` with full
  history.
- **Changed:** its `[workspace.package]` collided on `rust-version` and
  `license`, so its crates carry literal `[package]` fields and only its
  dependencies were hoisted (`aisdk`, `schemars`, `toml`, and the `rk-*`
  path entries).
- **Fixed:** `rusty_key`'s `[workspace.lints]` (`unsafe_code = "forbid"`)
  is strictly stronger than this root's, which is `rustils`'
  (`unsafe_code = "warn"`). Leaving `[lints] workspace = true` in place
  would have silently downgraded all eight crates, so each carries a
  literal `[lints.rust] unsafe_code = "forbid"`; `rustils`' crates keep
  inheriting the root table unchanged.
- **Changed:** `crates/rusty_key` reformatted with `cargo fmt --all` — it
  was not fmt-clean under this workspace's settings, same as
  `rusty_ansder`/`rusty_boot` when they merged. No behavior change.
- **Known limitation:** its Tauri desktop shell
  (`crates/rusty_key/desktop/src-tauri`) stays a standalone workspace and
  is excluded here, exactly as its own repo had it — the opposite call from
  `inventory-tauri`, and deliberately so, since each is upstream's own.
  Verified it still builds across the boundary post-merge.
- **Known limitation:** two more duplicate-major pairs now resolve —
  `rmcp` 0.9.1 alongside 3.1.4, and `axum` 0.7.9 alongside 0.8.9. Cargo
  keeps them as unrelated crates and no type crosses between the groups.
- No dependency swaps. Full suite (194 tests) passes unmodified.

## Fourth-wave merge — `rusty_skillopt`
**2026-09-01** · branch [`claude/rusty-repos-migration-iwvuld`](https://github.com/Rusty-Mill/rusty_mill/tree/claude/rusty-repos-migration-iwvuld)

- **Added:** `crates/rusty_skillopt` — a from-scratch Rust take on
  Microsoft's SkillOpt: optimize a skill markdown document as the trainable
  state of a frozen LLM agent, with epochs, batches and a validation gate,
  entirely in text space. Four crates behind one nested workspace, merged
  via `git subtree` with full history.
- **Changed:** its `[workspace.package]` collided on `license`, so its four
  crates carry literal `[package]` fields (the `rusty_db` treatment) and
  only its dependencies were hoisted. Root `tokio` widened to the union of
  what `rusty_search`, `rusty_db` and `rusty_skillopt` need; root `chrono`
  gained `serde`. `thiserror` stays literal on `skillopt-core`/
  `skillopt-model`, same `"2"`-vs-`"1"` reason as before.
- **Known limitation:** the workspace now resolves two `reqwest` majors —
  0.13.4 for `rusty_acp`/`rusty_mcp` and 0.12.28 for `skillopt-model`.
  Cargo treats them as unrelated crates so they coexist cleanly and no type
  crosses between the two groups; bumping `skillopt-model` would be an API
  change outside this merge's scope. A build-size cost, not a correctness
  one.
- No dependency swaps: nothing in `rusty_skillopt` depended on a sibling in
  this workspace. Full suite (68 tests, 2 environment-gated ignores) passes
  unmodified.

## Fourth-wave merge — `rusty_inventrory`
**2026-09-01** · branch [`claude/rusty-repos-migration-iwvuld`](https://github.com/Rusty-Mill/rusty_mill/tree/claude/rusty-repos-migration-iwvuld)

- **Added:** `crates/rusty_inventrory` — a local-first encrypted index over
  the conversation history Claude Code, Codex, Cursor, Zed, Kiro and
  Antigravity write to disk, plus its `inv` CLI and a Tauri menu-bar shell.
  Three crates behind one nested workspace, merged via `git subtree` with
  full history.
- **Changed:** its `[workspace.package]` collided with this root's on
  `rust-version`, `license` and `repository`, so its three crates carry
  literal `[package]` fields (the `rusty_db` treatment); only its
  dependencies were hoisted. `thiserror` stays literal on `inventory-core`
  for the same `"2"`-vs-`"1"` reason as `rusty_test`'s `contract`.
- **Changed:** CI's Linux leg installs `libwebkit2gtk-4.1-dev`,
  `libgtk-3-dev`, `libayatana-appindicator3-dev`, `librsvg2-dev` (the Tauri
  shell) and `libdbus-1-dev` (`inventory-core`'s Secret Service keyring),
  matching what `rusty_inventrory`'s own CI installed. Windows and macOS
  use OS-native APIs for both.
- **Fixed:** `inventory-core`'s `rusqlite = "0.37"` needs
  `libsqlite3-sys ^0.35`, which cannot coexist with `sqlx-sqlite`'s
  `^0.30.1` — `libsqlite3-sys` sets `links = "sqlite3"`, so exactly one
  version may exist per graph. Moving *up* would mean `sqlx 0.9` across
  `rusty_db` and `rusty-search-sqlite-fts5`, so `inventory-core` came down
  to `rusqlite = "0.32.1"`, unifying with `rusty_sqlite`. Verified by
  running its suite, not by reading changelogs: 79 tests pass unmodified.
- **Fixed:** three deprecated `GenericArray::from_slice` calls in `db.rs`'s
  sealed-index code — same `generic-array` 0.14.9 cause as `rusty_croc`'s,
  same behavior-preserving rewrite.
- No dependency swaps: nothing in `rusty_inventrory` depended on a sibling
  in this workspace.

## Fourth-wave merge — `rusty_test`
**2026-09-01** · branch [`claude/rusty-repos-migration-iwvuld`](https://github.com/Rusty-Mill/rusty_mill/tree/claude/rusty-repos-migration-iwvuld)

- **Added:** `crates/rusty_test` — the `portable-runtime-contract` spike:
  one execution contract (`contract`), a per-host adapter (`compat`), a
  verification layer (`conformance`), and three reference tools
  (`stat-tool`, `proc-runner`, `pty-shell`). Merged via `git subtree` with
  full history; its nested `[workspace]` table removed and its six crates
  added to this root's `members`.
- **Changed:** its `[workspace.package]` didn't collide with this root's
  (same edition, same license), so its crates keep inheriting via
  `field.workspace = true` — the `rusty_search` treatment, not
  `rusty_db`'s. Only `publish = false` was new here. `thiserror` was
  deliberately left un-hoisted: `rusty_test` wanted `"2.0"`, this root
  pins `"1"` for `rusty_db`/`rustils`, so `contract` keeps a literal
  `thiserror = "2"` instead of forcing a major bump on unrelated crates.
- **Fixed:** `conformance`'s `tests/layering.rs` reads the workspace
  manifest to enforce the layer model, resolving it two directories above
  its own crate and requiring a declared layer for every member found.
  Post-merge that is this monorepo's root — four levels up, ~100 members —
  so three of its four tests panicked. Repointed and filtered through a
  `GROUP_PREFIX` constant; the check's logic is otherwise untouched and all
  four tests pass, alongside the group's other 27.
- No dependency swaps: nothing in `rusty_test` depended on a sibling in
  this workspace.

## Fourth-wave merge — `rusty_croc`
**2026-09-01** · branch [`claude/rusty-repos-migration-iwvuld`](https://github.com/Rusty-Mill/rusty_mill/tree/claude/rusty-repos-migration-iwvuld)

- **Added:** `crates/rusty_croc` — a Rust port of
  [croc](https://github.com/schollz/croc), wire-compatible with stock croc
  v10 (PAKE code phrases, relay, local-network hand-off, resume). Merged
  via `git subtree` with its full commit history, as with every prior
  crate import.
- **Fixed:** four `GenericArray::from_slice` calls in `crypt.rs` (AES-256-GCM
  and XChaCha20-Poly1305 nonces). The standalone repo's lockfile pinned
  `generic-array` 0.14.7; this workspace resolves 0.14.9, which deprecates
  the crate wholesale, so `-D warnings` turned them into errors. Rewritten
  to the `From<&[T]> for &GenericArray` conversion `from_slice` delegates
  to — no behavior change, 49 tests pass unmodified.
- No dependency swaps: `rusty_croc` depends only on crates.io crates, not
  on any sibling in this workspace. Its nightly-only `fuzz/` harness keeps
  its own `[workspace]` table and is excluded from this one, same as
  `rusty_tls/fuzz` and `rusty_lsp/fuzz`.

## PR #65 — Deduplicate `rusty_rdp`'s byte cursor and split `rusty_ansder`'s two crates
**2026-09-01** · [#65](https://github.com/Rusty-Mill/rusty_mill/pull/65)

- **Fixed:** `rusty_rdp`'s hand-rolled byte Reader/Writer duplicated
  `rusty_wire`'s (a dependency `rusty_rdp` already declared but never
  used) — now re-exports `rusty_wire`'s cursor types.
- **Changed:** `rusty_ansder` bundled two unrelated libraries (an ASN.1 DER
  codec and a sovereign RAG/Q&A engine). Split the RAG engine into a new
  `rusty_rag` crate; `rusty_ansder` now holds just the DER codec.
- Also investigated and deferred (different-scoped tools sharing a name,
  not true duplication): `rusty_term` vs. `rusty_ansi`; `rusty_ansder`'s DER
  codec vs. `rusty_tls`'s hand-rolled DER; `rusty_http::Url` vs.
  `rusty_url::Url`.

## PR #10 — Collapse workspace duplication: to_wide, read_lines, SHA-1, IFS splitting, glob, raw-mode
**2026-08-27** · [#10](https://github.com/Rusty-Mill/rusty_mill/pull/10)

- **Fixed:** six of eight findings from a five-sweep duplication review
  (issues #1–#8) — `rusty_win32`'s 7x-duplicated `to_wide()` hoisted;
  `rsed`/`rawk`'s shared stdin-reading extracted to `read_lines()`;
  `rusty_git`/`rusty_term`'s independent SHA-1 implementations merged into
  a new `rusty_sha1` crate; `rush`'s two independent IFS-splitting
  implementations merged into `ifs_run_end()`; `rush`'s backtracking glob
  matcher now tries `rusty_regx::Glob` first; a duplicated Windows
  raw-mode flag transformation (`rusty_term`/`rusty_lines`) hoisted into
  `rusty_win32::console::raw_mode_core()`.
- **Known limitation:** two findings closed `no action` — Unix termios
  save/restore (`rusty_term`/`rusty_lines`) is the same shape by
  deliberate, different policy; a `no_std` rounding workaround in
  `rusty_font` (`round_nonneg` vs. `round_f32`) likewise.
- Filed #9 for the remaining gap (`rusty_regx::Glob` needs embedded `!(p)`
  negation support before rush's fallback matcher can be fully deleted) —
  a capability gap, not duplication; still open as of this writing.

## Earlier crate-import history

Every `Import <crate> into crates/<crate>` merge and the CI-scoping work
(`Speed up CI: affected-crate filtering, rust-cache, nextest, parallel
clippy`) predate this file. See `git log --oneline --merges` for the full
list — not backfilled entry-by-entry here since each import is already its
own reviewable commit with a descriptive message, and there are dozens of
them (see the crate table in `README.md` for the two-wave merge history).
