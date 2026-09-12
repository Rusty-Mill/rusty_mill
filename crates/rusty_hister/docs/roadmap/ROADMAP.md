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
- [ ] User confirmation on ADR-0001's licensing recommendation (Go test
      fixtures: rewrite from independent reading, don't copy verbatim).

## Phase 1 — Unblocked foundations (can start once Phase 0's docs land,
independent of ADR-0002/0003)

- `rusty-hister-core`: `Document` type, extractor-SDK contract types
  (capability inventory §4.1), shared error type.
- `rusty-hister-model`: the ten GORM-equivalent models on `rusty_db`
  (capability inventory §7.2), the two-phase pre/post migration mechanism
  (§7.3) including the three concrete migrations, UTC-everywhere timestamp
  discipline (§7.1), and the legacy `indexer_versions` read path (pending
  the open sign-off in PROJECT-STATUS.md).
- `rusty-hister-extractor`: the SDK contract and chain-of-responsibility
  registry (§4.1-§4.2), then the extractors in default-chain order (§4.3),
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
