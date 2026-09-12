# rusty_hister

A Rust port of [asciimoo/hister](https://github.com/asciimoo/hister)
(AGPL-3.0-or-later) — a personal full-text + semantic search engine over
browsing history and local files — built natively as a crate cluster inside
the RustyMill Cargo workspace (not imported via `git subtree`, since this is
fresh work, not a merge of a pre-existing standalone repo).

**v1 scope is backend only**: an HTTP/JSON API and MCP JSON-RPC surface that
stays wire-compatible with Hister's existing SvelteKit `webui`, browser
extension, and qutebrowser companion — none of which move. The CLI, TUI, and
companion daemon are later phases. See `docs/PROJECT-STATUS.md` and
`docs/roadmap/ROADMAP.md`.

**No implementation code exists yet.** This is the bootstrap/scaffold
commit: the capability inventory, the crate split, and two open
decision-requests that block indexer and crawler work. See
`docs/decisions/` before writing indexing or crawling logic.

## Crates

| Crate | Path | Purpose |
|---|---|---|
| [`rusty-hister-core`](crates/rusty-hister-core) | `crates/rusty_hister/crates/rusty-hister-core` | Shared types, IDs, error types, and the `Document`/extractor-SDK data model — **implemented** (Phase 1) |
| [`rusty-hister-model`](crates/rusty-hister-model) | `crates/rusty_hister/crates/rusty-hister-model` | Persisted schema on `rusty_db` (dual SQLite/Postgres) — history, links, users, crawl jobs, embedding queue, sessions — **implemented** (Phase 1: schema, plus the embedding-queue, `WebSession`, `DocumentVersion`, `crawl.go`, and `history.go` query layers; only `user.go`'s query helpers not yet started) |
| [`rusty-hister-extractor`](crates/rusty-hister-extractor) | `crates/rusty_hister/crates/rusty-hister-extractor` | Extractor chain-of-responsibility registry — **implemented** (Phase 1); the 20 built-in per-site/format content extractors themselves are not yet started |
| [`rusty-hister-indexer`](crates/rusty-hister-indexer) | `crates/rusty_hister/crates/rusty-hister-indexer` | Query language + full-text indexing on `rusty_search` + `rusty-search-sqlite-fts5` (ADR-0002) |
| [`rusty-hister-vectorstore`](crates/rusty-hister-vectorstore) | `crates/rusty_hister/crates/rusty-hister-vectorstore` | Embedding pipeline and vector storage for semantic search |
| [`rusty-hister-crawler`](crates/rusty-hister-crawler) | `crates/rusty_hister/crates/rusty-hister-crawler` | HTTP and CDP (`chromiumoxide`) crawler backends; WebDriver BiDi descoped for v1 (ADR-0003) |
| [`rusty-hister-server`](crates/rusty-hister-server) | `crates/rusty_hister/crates/rusty-hister-server` | HTTP/JSON API + WebSocket search protocol — the v1 backend surface |
| [`rusty-hister-mcp`](crates/rusty-hister-mcp) | `crates/rusty_hister/crates/rusty-hister-mcp` | MCP JSON-RPC tool surface (search, get_preview, get_history) on `rusty_mcp` |

A `rusty_hister` binary crate (CLI) is deferred to a later phase per v1's
backend-only scope (see ADR-0001) — v1 ships as libraries plus whatever thin
`main.rs` `rusty-hister-server` needs to run standalone; a full cobra-style
CLI is not part of this bootstrap.

## Governance

- `AGENTS.md` — project shape and canonical commands.
- `WORKFLOW.md` — PR/CI/merge conventions for this cluster.
- `docs/PROJECT-STATUS.md` — current status, updated per roadmap-unit change.
- `docs/roadmap/ROADMAP.md` — delivery sequencing.
- `docs/decisions/` — ADRs, numbered from this cluster's own `0001` (per the
  root workspace's `docs/adr/0001-consolidate-crates-into-workspace.md`
  remit: workspace-wide decisions live at the workspace root; decisions
  internal to this cluster live here).
- `docs/capability-inventory/HISTER-CAPABILITY-INVENTORY.md` — the full,
  rust-migration-style capability manifest built from reading Hister's
  actual Go source (commit `49b727f4`). Every row defaults to REQUIRED scope;
  nothing moves to out-of-scope without an explicit, written,
  user-attributed sign-off recorded in an ADR.
