# ROADMAP: rusty_hister

This roadmap covers **v1 (backend only)** per `docs/PROJECT-STATUS.md`.
Later phases (CLI, TUI, companion) are listed at the end for visibility, not
because they're scheduled.

## Phase 0 — Bootstrap (this commit)

- [x] Confirm repo-config's standard doc set is already applied at the
      RustyMill root.
- [x] Capability inventory built from Hister's actual Go source
      (`docs/capability-inventory/HISTER-CAPABILITY-INVENTORY.md`).
- [x] Sovereignty audit of candidate `rusty_*` crates.
- [x] Crate cluster scaffolded and registered in the root workspace.
- [x] ADR-0001 (bootstrap scope and crate split) written.
- [x] ADR-0002 (search engine) and ADR-0003 (JS-rendering crawler) written
      as open decision-requests.
- [x] User decision on ADR-0002 (`rusty_search` + `rusty-search-sqlite-fts5`,
      federation/`url_re:`/highlighting as hister-layer composition,
      `sqlite-vec`/`pgvector` for semantic search) and ADR-0003
      (`chromiumoxide` for CDP, WebDriver BiDi descoped for v1) — decided
      2026-09-12, ahead of ADR-0002's recommended scoping spike (see each
      ADR's "Accepted risk" note).
- [x] User confirmation on ADR-0001's licensing recommendation (Go test
      fixtures: rewrite from independent reading, don't copy verbatim) —
      confirmed 2026-09-12.

**Phase 0 is complete.** Everything blocking Phase 1-4 implementation is
resolved; nothing here has started that implementation yet.

## Phase 1 — Unblocked foundations (can start once Phase 0's docs land,
independent of ADR-0002/0003)

- [x] `rusty-hister-core`: `Document` type, extractor-SDK contract types
      (capability inventory §4.1: `Extractor` trait, `Capabilities`,
      `ExtractorConfig`, `ExtractOutcome`/`PreviewOutcome`,
      `PreviewResponse`), shared `HisterError` type. Done — 16 unit tests,
      clippy/fmt clean. The extractor *registry* (chain-of-responsibility,
      §4.2) is `rusty-hister-extractor`'s job, not this crate's.
- [x] `rusty-hister-model` (schema): the nine `#[derive(Mapped)]` models on
      `rusty_db` (capability inventory §7.2 — `Database`'s singleton-row
      version tracker is replaced by `rusty_db::Migrator`'s own bookkeeping,
      not ported separately), soft-delete via `#[table(soft_delete)]`, and
      a fresh-install migration (SQLite + Postgres) via `rusty_db::Migrator`.
      Done — 25 unit tests, clippy/fmt clean. Deliberately **schema only**:
      Hister's three historical migrations (§7.3) and the legacy
      `indexer_versions` read path are not reproduced, since they only
      matter for opening a pre-existing Hister-Go-created database file —
      a still-open, broader question, see PROJECT-STATUS.md's open items.
- [x] `rusty-hister-model` (embedding-queue query layer): `embedding.go`'s
      state machine as `EmbeddingJob` associated functions —
      `enqueue`/`claim_next`/`complete`/`retry`/`fail`/`release`/
      `in_progress_exists`/`delete`/`reset_in_progress`. Done — 18 new unit
      tests (43 total in the crate), clippy/fmt clean. `enqueue`/`retry`/
      `release` need a `CASE`-based `SET` the portable query builder can't
      express, so those three use raw SQL rendered dialect-portably via
      `Engine::dialect().placeholder(..)`; untested against real Postgres
      (no instance available), see PROJECT-STATUS.md's open items.
- [x] `rusty-hister-model` (`WebSession` query layer): `session.go`'s
      lookup/expiry helpers as `WebSession` associated functions —
      `create`/`get`/`refresh`/`delete` (`refresh`, not `update`, to avoid
      colliding with `#[derive(Mapped)]`'s own generated `update()`
      instance method). Done — 7 new unit tests (50 total in the crate),
      clippy/fmt clean. `create` is this crate's first database-assigned
      surrogate key: `Mapped::insert()` always supplies the primary key's
      current value, so `create` drops to a raw `INSERT` that omits `id`
      and recovers the generated value dialect-appropriately (`RETURNING
      id` on Postgres, `SELECT last_insert_rowid()` on SQLite) — the same
      recipe `User`/`Link`/`History`/`HistoryLink`/`DocumentVersion` will
      need for their own `create`s. The dialect-placeholder helper
      introduced for the embedding queue is now shared (`crate::placeholders`,
      hoisted to `lib.rs` on this second real call site).
- `rusty-hister-model` (remaining query layer): the domain operations the
  other five Go model files build on top of their tables — `history.go`'s
  search/pin/timeline queries, `user.go`'s auth/token helpers,
  `CreateCrawlJob`/`CreateNamedCrawlJobWithURLs`, and
  `SaveDocumentVersion`/`GetDocumentVersionsUntil` — separate increments
  from the embedding queue and `WebSession` above, same split
  `rusty-hister-extractor` used (mechanism before concrete extractors).
- [x] `rusty-hister-extractor`: the chain-of-responsibility registry
      (§4.2) — `Registry::register`/`register_before` (case-insensitive
      duplicate rejection), the two-phase enrich-then-extract chain,
      the separate preview chain with starting-point selection, and
      `apply_configs`. Done — 18 unit tests, clippy/fmt clean. (The SDK
      contract itself, §4.1, landed with `rusty-hister-core`.)
- `rusty-hister-extractor`: the extractors in default-chain order (§4.3),
  starting with the ones that have existing Go test coverage (11 of 20) and
  budgeting fresh test authorship for the other 9.
- `rusty-hister-crawler`: the `http` backend only (§8.1's default backend),
  BFS traversal, validator rules, robots.txt, proxy support, persistent
  crawl jobs (§8.2-§8.6) — all backend-agnostic or `http`-specific, none of
  it blocked on ADR-0003.

## Phase 2 — Server + MCP surface (v1's actual deliverable)

- `rusty-hister-server`: the 39-route HTTP API (§1), session/OAuth/CSRF
  model (§1.x), WebSocket search protocol.
- `rusty-hister-mcp`: the three MCP tools, with the trust-boundary envelope
  and `mcpNormalizeUntrusted` ported byte-for-byte (§2) — this is the
  highest-priority-to-get-right unit in the whole port.
- Both depend on `rusty-hister-indexer` existing enough to serve `search`,
  so cannot fully land until Phase 3 delivers at least a non-semantic-search
  path through the indexer — the route table/tool schemas themselves,
  however, can be scaffolded against a stub indexer in the meantime.

## Phase 3 — Indexer + vectorstore (unblocked by ADR-0002, not yet started)

- Query grammar/lexer (§5.2-§5.4) against `rusty-search-core`'s `Query`
  tree (ADR-0002's decision) — backend-independent, so this can start
  immediately.
- `rusty-search-sqlite-fts5` integration (ADR-0002's chosen backend);
  multi-language federation, `url_re:` custom-filter equivalent, and the
  three highlight styles all built as `rusty-hister-indexer`-layer
  composition per ADR-0002's decision, not `rusty-search-core` changes.
- `rusty-hister-vectorstore`'s embedding pipeline (build on
  `rusty_llama`/`rusty_provider` per ADR-0001) and storage side: `sqlite-vec`
  (vendored C extension) for the SQLite path, `pgvector` for Postgres, per
  ADR-0002's decision.

## Phase 4 — JS-rendering crawler backend (unblocked by ADR-0003, not yet
started)

- `chromedp`-equivalent backend on `chromiumoxide` (ADR-0003's decision).
- `bidi`-equivalent backend: **out of v1 scope** — ADR-0003 explicitly
  descoped it, not merely deferred it pending a sign-off. Revisit only if a
  concrete driver-free-deployment need arises later.
- Re-enable the Notion extractor, which hard-depends on JS rendering
  existing at all (§4.5.16) — satisfied by the CDP backend above; not
  affected by BiDi's descope.

## Later phases (out of v1, tracked for visibility only)

- CLI (cobra-equivalent, ~35 subcommands, `cmd/*.go`).
- TUI (Bubble Tea, `cmd/tui/`).
- qutebrowser companion daemon (`cmd/companion/`).
- Browser extension: stays TypeScript/Svelte; only its API contract is
  tracked here (must not break against `rusty-hister-server`).
