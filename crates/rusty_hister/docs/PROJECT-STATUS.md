# PROJECT-STATUS: rusty_hister

Last updated: 2026-09-12 (bootstrap commit).

## Where this is

**Bootstrap stage.** This commit establishes the crate cluster, the
capability inventory, and the two open decision-requests that block further
work — it contains **no port implementation logic**. Every `rusty-hister-*`
crate compiles as an empty skeleton with a module doc comment pointing back
to the capability inventory and the relevant ADR.

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

## Blocking decisions (open)

| ADR | Subject | Status |
|---|---|---|
| [ADR-0002](decisions/ADR-0002-search-indexing-engine-approach-proposal.md) | Search/indexing engine approach | **Proposed — awaiting sign-off** |
| [ADR-0003](decisions/ADR-0003-js-rendering-crawler-approach-proposal.md) | JS-rendering crawler approach | **Proposed — awaiting sign-off** |

`rusty-hister-indexer`'s query-DSL-to-`Query`-tree compiler and
`rusty-hister-vectorstore`'s storage side cannot start until ADR-0002
resolves. `rusty-hister-crawler`'s `chromedp`- and `bidi`-equivalent
backends cannot start until ADR-0003 resolves. The HTTP-only crawler
backend, the extractor SDK, the model schema, the server route table, and
the MCP tool surface are **not** blocked by either and can proceed in
parallel — see `docs/roadmap/ROADMAP.md`.

## Open items carried from the capability inventory (not blocking, but
unresolved — see `docs/capability-inventory/HISTER-CAPABILITY-INVENTORY.md`
§13 and inline flags)

- Licensing approach for Go test fixtures (ADR-0001) — recommendation
  stated, not yet confirmed by the user.
- `sqlite-vec` C-extension story for the SQLite vectorstore backend — folded
  into ADR-0002 rather than decided separately, since it's downstream of the
  storage-engine choice.
- Legacy pre-GORM `indexer_versions` table read path (capability inventory
  §7.3) — flagged for explicit scope sign-off, not yet resolved either way.
- Document-versioning diff format/algorithm (capability inventory §11) and
  query-alias expansion site (capability inventory §11) — flagged in the
  inventory as needing a follow-up read of the Go source before the
  `rusty-hister-model`/`rusty-hister-indexer` design can fully account for
  them; not yet done.

## Crate status

| Crate | Status |
|---|---|
| `rusty-hister-core` | Skeleton only |
| `rusty-hister-model` | Skeleton only |
| `rusty-hister-extractor` | Skeleton only |
| `rusty-hister-indexer` | Skeleton only — blocked on ADR-0002 |
| `rusty-hister-vectorstore` | Skeleton only — storage side blocked on ADR-0002 |
| `rusty-hister-crawler` | Skeleton only — JS-rendering backends blocked on ADR-0003 |
| `rusty-hister-server` | Skeleton only |
| `rusty-hister-mcp` | Skeleton only |
