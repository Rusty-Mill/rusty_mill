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
- [ ] User sign-off on ADR-0002 and ADR-0003.
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
  so cannot fully land until ADR-0002 resolves and at least a
  non-semantic-search path through the indexer works — the route
  table/tool schemas themselves, however, can be scaffolded against a stub
  indexer in the meantime.

## Phase 3 — Indexer + vectorstore (blocked on ADR-0002)

- Query grammar/lexer (§5.2-§5.4) — can start immediately once ADR-0002
  names a target `Query`-tree shape to compile into, since the grammar
  itself is backend-independent.
- Multi-language federation, `url_re:` custom-filter equivalent, and the
  three highlight styles (§5.1, flagged as the hard part of ADR-0002).
- `rusty-hister-vectorstore`'s embedding pipeline (not blocked — build on
  `rusty_llama`/`rusty_provider` per ADR-0001) and storage side (blocked on
  ADR-0002's `sqlite-vec` sub-decision).

## Phase 4 — JS-rendering crawler backends (blocked on ADR-0003)

- `chromedp`-equivalent backend.
- `bidi`-equivalent backend, or an explicit descope sign-off per ADR-0003.
- Re-enable the Notion extractor, which hard-depends on one of these
  (§4.5.16).

## Later phases (out of v1, tracked for visibility only)

- CLI (cobra-equivalent, ~35 subcommands, `cmd/*.go`).
- TUI (Bubble Tea, `cmd/tui/`).
- qutebrowser companion daemon (`cmd/companion/`).
- Browser extension: stays TypeScript/Svelte; only its API contract is
  tracked here (must not break against `rusty-hister-server`).
