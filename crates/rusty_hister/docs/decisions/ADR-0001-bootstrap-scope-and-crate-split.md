# ADR-0001: Bootstrap this cluster as native workspace crates, v1 scope, and the crate split

Status: Accepted
Date: 2026-09-12

## Remit of this ADR series

This `docs/decisions/` directory (inside `crates/rusty_hister/`) records
decisions internal to this cluster's own design, mirroring the pattern the
root workspace's `docs/adr/0001-consolidate-crates-into-workspace.md`
establishes: workspace-wide decisions live at the workspace root; a crate's
(or cluster's) own decisions live in its own `docs/adr/` or `docs/decisions/`
directory, numbered independently from `0001`.

## Context

The kickoff brief asks for a Rust port of
[asciimoo/hister](https://github.com/asciimoo/hister) (AGPL-3.0-or-later) —
a personal full-text + semantic search engine over browsing history and
local files — as a project living **inside** the RustyMill Cargo workspace,
not a new standalone repo, following the same systems-engineering bootstrap
(`PROJECT-STATUS.md`, a roadmap, ADRs) and `rust-migration`-style
capability-inventory discipline (every observable capability defaults to
REQUIRED; nothing drops silently) used elsewhere in this workspace.

Unlike most multi-crate clusters already in this workspace (`rusty_search`,
`rusty_db`, `rusty_mcp`, `nexus`), this one is **not** a `git subtree` import
of a pre-existing standalone `baileyrd/*` repo — it's fresh work written
directly here. That matters for two mechanical choices below: there is no
prior standalone-repo `Cargo.lock`/`[workspace]` file to preserve (so this
cluster has none, matching `rusty_search`'s post-merge shape rather than
`rusty_mcp`/`rusty_db`'s, which still carry theirs as a historical artifact
of their own former repos), and there is no pre-existing commit history to
bring over via subtree.

## Decision

### 1. Native workspace crates, flat layout

`crates/rusty_hister/crates/rusty-hister-*`, registered directly as members
in the root `Cargo.toml`, no nested `[workspace]` manifest. CI needs no new
wiring: the root `.github/workflows/ci.yml`'s `plan` job already scopes
`fmt`/`clippy`/`test` to whichever workspace crates a PR touches, and these
crates are now in that member list. (The kickoff brief's phrase "wire CI
through `cargo-rail`" does not correspond to anything in this repo — no such
tool exists here; the existing affected-crates-filter CI is the workspace's
actual convention and is what this cluster uses.)

### 2. Capability inventory

Built by cloning `asciimoo/hister` at commit `49b727f4` and reading the
actual Go source (not inferring from the kickoff brief's rough orientation
notes, which were explicitly marked "verify against source, don't trust
this"). Recorded in full at
`docs/capability-inventory/HISTER-CAPABILITY-INVENTORY.md` — 39 HTTP routes,
3 MCP tools, ~35 CLI subcommands, 20 extractors (11 with existing Go test
coverage, 9 without), the full query-language grammar, the vectorstore/
embedding pipeline, 10 DB models with a two-phase migration mechanism, 3
crawler backends (1 untested-in-Go beyond proxy handling), and the TUI. Every
row defaults to REQUIRED scope per `rust-migration` discipline; the two
scope reductions this ADR does make (below) are recorded explicitly, not
inferred.

### 3. v1 scope decision — backend only

Per the kickoff brief's explicit instruction (itself the user-attributed
sign-off record for this scope line, since the brief states it directly
rather than leaving it for inference): v1 targets an HTTP/JSON API and MCP
JSON-RPC surface compatible with Hister's existing SvelteKit `webui`, the
browser extension, and the qutebrowser companion. The CLI (~35 subcommands),
TUI (Bubble Tea), and companion daemon are deferred to later phases — **not
dropped**, tracked in `docs/roadmap/ROADMAP.md`'s "Later phases" section.
The browser extension itself stays TypeScript/Svelte (there is no
Rust-shaped unit to port it into); what's tracked is that
`rusty-hister-server`'s HTTP contract must not break against it, the
companion, or the TUI once any of those are revisited.

### 4. Crate split — revised from the kickoff brief's starting suggestion

The brief's suggested split (`rusty_hister` binary, `-core`, `-indexer`,
`-extractor`, `-crawler`, `-vectorstore`, `-mcp`, `-tui`) is explicitly
marked as a starting point, not a locked decision. Two changes fall directly
out of the capability inventory and the v1-backend-only scope decision:

- **Added `rusty-hister-model`.** The kickoff brief suggested folding the DB
  layer into whichever crate uses it, since `rusty_db` is "likely the right
  home for the model layer instead of a fresh ORM." That's right about the
  ORM (use `rusty_db`, don't build one), but the *schema* — ten models, a
  two-phase pre/post migration mechanism, a UTC-everywhere timestamp
  discipline, and a legacy pre-GORM compatibility read path (capability
  inventory §7) — is substantial enough, and shared across enough other
  crates (server, indexer, vectorstore, crawler all touch persisted state),
  that folding it into `rusty-hister-core` would make core a
  database-dependent crate for what's meant to be lightweight shared types.
  A dedicated `rusty-hister-model` keeps `core` free of the `rusty_db`
  dependency.
- **Added `rusty-hister-server`, dropped the `rusty_hister` CLI binary
  and `rusty-hister-tui` from this bootstrap.** Since v1 is backend-only,
  the primary deliverable is the HTTP/JSON API surface (`server/api.go`'s
  39 routes, session/OAuth/CSRF model, WebSocket search protocol) — this
  is substantial enough (and load-bearing enough, since three separate
  live clients depend on its exact shape per capability inventory §11) to
  warrant its own crate rather than living inside a CLI binary crate. A
  `rusty_hister` CLI binary and a `rusty-hister-tui` crate are deferred to
  the later phases in `docs/roadmap/ROADMAP.md` alongside the CLI/TUI scope
  deferral above, rather than scaffolded now as empty stubs for
  out-of-v1-scope work.

### 5. Sovereignty audit — before any new dependency

Checked every candidate `rusty_*` crate named in the kickoff brief against
its actual current capabilities (not its name) before this bootstrap
assumed any of them. Summary (full detail in the audit agent's findings,
reproduced here since it's the record this ADR relies on):

| Need | Crate(s) | Verdict |
|---|---|---|
| Async runtime | `rusty_tokio` | **Covers** — genuinely tokio-equivalent (scheduler, reactor, timers, full `sync` module, `select!`/`join!`) |
| HTTP client (crawler's non-JS fetch path) | `rusty_http` + `rusty_request` | **Covers** — pooling, cookies, redirects, retries, multipart, streaming, HTTP+HTTPS proxy (HTTP/1.1 only) |
| TLS | `rusty_tls` | **Covers** (client; rustls-backed per root ADR-0002) |
| JSON | `rusty_json` | **Covers** — real serde `Serialize`/`Deserialize` support (use serde derive, not the workspace's own unwired `rusty_json-derive` stub) |
| JSON-RPC / MCP | `rusty_mcp` | **Covers** — mature `#[tool_router]`/`#[tool]` framework on `rmcp` 3.x, Streamable HTTP + stdio transports; build `rusty-hister-mcp` directly on it, do not write a fresh JSON-RPC layer |
| DB/ORM, dual SQLite+Postgres | `rusty_db` | **Covers** — full SQLAlchemy-Core-style query builder, migrations/`automigrate`, both backends via `sqlx` |
| Full-text search / BM25 | `rusty_search` (`rusty-search-tantivy`, `rusty-search-sqlite-fts5`) | **Covers BM25 itself**; **partially covers** the query-DSL ask (structured `Query`-tree builder exists, no text-grammar parser — that's new work, in scope for `rusty-hister-indexer`); vector/hybrid search is designed-for but unimplemented in any backend — see ADR-0002 |
| URL parsing | `rusty_url` | **Covers** — WHATWG-compliant |
| Embeddings (local + cloud) | `rusty_llama`, `rusty_provider` | **Covers** — `rusty_llama` produces real local embedding vectors (`forward_embed`); `rusty_provider` exposes OpenAI-compatible `/v1/embeddings` with fallback chains |
| TUI base | `rusty_term` | **Does not cover** — it's a terminal *emulator* (PTY/ANSI/grid), not a widget/event-loop application framework; moot for v1 since the TUI is deferred |
| Headless-browser/CDP client | *(none)* | **Confirmed gap** — grepped the whole workspace for "chromedp"/"CDP"/"devtools protocol"/WebSocket-client usage; nothing exists. See ADR-0003. |

### 6. Storage note — `sqlite-vec`

Hister's SQLite vectorstore backend vendors the `sqlite-vec` C extension via
cgo, hand-patched for musl compatibility (capability inventory §6.2). This
is a genuine Rust build-system/packaging problem, not a logic-porting one.
Rather than defaulting silently to either "load the same C extension from
Rust" or "replace it," this is folded into ADR-0002 (search/indexing engine)
as a sub-decision, since `rusty_search`'s own roadmap already anticipates a
hybrid/vector search path and the answer likely depends on which backend
ADR-0002 picks.

### 7. Licensing — confirmed 2026-09-12

**Confirmed by the user (baileyrd/Nano), 2026-09-12** — "Confirm the AGPL
test-fixture licensing recommendation from ADR-0001." The recommendation
below is now settled policy for this cluster, not a proposal: `rusty_hister`
ships under this workspace's standard `MIT OR Apache-2.0`, and no Hister
source file — test files included — is copied verbatim into it. This binds
every extractor and query-grammar test written from here on; a PR that
copies Go test file content (rather than independently deriving a Rust test
from reading it) is a licensing regression, not a style nit, and should be
corrected before merge.

Hister is AGPL-3.0-or-later; this workspace's default license is `MIT OR
Apache-2.0` (root `Cargo.toml`'s `[workspace.package]`). An independent,
clean-room reimplementation from behavioral understanding — which is what
the kickoff brief already asks for ("independently reimplement the
behavior; don't carry Go idioms or structure over verbatim") — is not a
derivative work of the Go source under copyright law, since functionality
and behavior are not themselves copyrightable, only the Go source's own
expression is. That supports licensing `rusty_hister` under this
workspace's standard `MIT OR Apache-2.0`, which is what every new crate's
`Cargo.toml` in this bootstrap already declares.

The one place this gets genuinely risky is **Go test fixtures**. The
capability inventory's own instruction (echoing the kickoff brief) was to
treat existing `_test.go` files as parity-test fixtures "don't discard
them" — but a `_test.go` file is itself AGPL-licensed Go source code, and
copying its actual assertions/table-driven test data verbatim into an
MIT/Apache-2.0-licensed crate would be incorporating AGPL-covered
expression, not just using it as a behavioral reference. **Decision**
(confirmed 2026-09-12, per the note above): do not copy any Hister source
file, test files included, into this cluster. Instead, write fresh Rust
tests derived from independently reading and understanding what each Go
test verifies (the capability inventory's per-extractor test-file
citations exist to make this traceable — "this behavior is covered by
`reddit/reddit_test.go`," not "here is a transliterated copy of it").
Where a Go test embeds third-party sample content (a real Reddit page's
HTML, a real GitHub issue page) that content itself isn't Hister's own
creative expression and using an independently-fetched or freshly-authored
equivalent sample avoids the question entirely. This governs every
extractor and query-grammar test written from here on.

## Alternatives considered

**Fold the DB/model layer into `rusty-hister-core`.** Rejected — see crate
split rationale above; it would make `core` a database-dependent crate.

**Scaffold `rusty_hister` (CLI) and `rusty-hister-tui` now as empty stubs,
matching the brief's suggested split literally.** Rejected for this
bootstrap: v1 is backend-only, and stub crates for explicitly-deferred scope
add workspace-member noise without tracking anything `docs/roadmap/
ROADMAP.md`'s "Later phases" section doesn't already track just as well.

**Decide the search-engine and JS-rendering-crawler questions in this ADR
rather than as separate decision-requests.** Rejected per the kickoff
brief's explicit instruction: these are the two flagged genuine gaps and
get their own decision-requests, with a scoping spike before commitment, not
a default baked into the bootstrap ADR.

## Consequences

- ADR-0002 and ADR-0003 (both since **Accepted**, 2026-09-12) unblocked
  `rusty-hister-indexer`, `rusty-hister-vectorstore`'s storage side, and
  `rusty-hister-crawler`'s CDP backend; see those ADRs for what they
  decided. Everything else in `docs/roadmap/ROADMAP.md`'s Phase 1 was never
  blocked on them.
- The licensing decision in §7 (**confirmed** 2026-09-12) means every
  future PR touching extractor or query-grammar tests must be checked for
  "independently written" vs. "copied from Go source" — this is a real
  ongoing review cost, not a one-time decision.
- The crate split adds `rusty-hister-model` and `rusty-hister-server`
  beyond the kickoff brief's starting suggestion; anyone reading the brief
  alongside this ADR should treat this ADR as the current source of truth
  for the split, not the brief.
