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
- [x] `rusty-hister-model` (`DocumentVersion` query layer): `version.go`'s
      helpers as `DocumentVersion` associated functions —
      `save`/`move_versions`/`count`/`list`/`list_until` (Go:
      `SaveDocumentVersion`/`MoveDocumentVersions`/`CountDocumentVersions`/
      `GetDocumentVersions`/`GetDocumentVersionsUntil`). Done — 6 new unit
      tests (56 total in the crate), clippy/fmt clean. `save` reuses
      `WebSession::create`'s database-assigned-surrogate-key recipe. The
      document-versioning diff format/algorithm (capability inventory
      §11) stays a separate, undecided concern — this layer only stores
      and retrieves whatever diff text the caller already computed.
- [x] `rusty-hister-model` (`CrawlJob` lifecycle query layer): `crawl.go`'s
      job-level helpers as `CrawlJob` associated functions —
      `generate_id`/`create`/`create_with_urls`/`get`/`update_status`/
      `list`/`delete` (Go: `GenerateCrawlJobID`/`CreateCrawlJob`/
      `CreateNamedCrawlJobWithURLs`/`GetCrawlJob`/`UpdateCrawlJobStatus`/
      `ListCrawlJobs`/`DeleteCrawlJob`). Done — 9 new unit tests (65 total
      in the crate), clippy/fmt clean. `generate_id` uses the existing
      first-party `rusty_rand` crate (sovereignty check: already in this
      workspace, so no new external `rand` dependency). `create_with_urls`
      atomically creates a job and its initial URL queue inside a
      `Transaction`, retrying with a `-2`/`-3`/... suffix on id collision —
      both that retry and the per-URL dedup need `ON CONFLICT DO NOTHING`,
      so it drops to raw SQL, same pattern as `EmbeddingJob::enqueue`.
      `CrawlURL`'s own queue mechanics (bulk insert, per-URL status
      updates, the `ForEach*` streaming iterators, job stats) are a
      separate, not-yet-started increment.
- [x] `rusty-hister-model` (`CrawlURL` queue-mechanics query layer):
      `crawl.go`'s remaining URL-level helpers as `CrawlURL` associated
      functions — `insert_if_not_exists`/`bulk_insert`/
      `mark_done_and_enqueue_links`/`insert_done`/`next_pending`/
      `update_status`/`mark_failed`/`reset_in_progress`/`count_by_status`/
      `count`/`list_failed`/`list`/`job_stats` (Go:
      `InsertCrawlURLIfNotExists`/`BulkInsertCrawlURLs`/
      `MarkCrawlURLDoneAndEnqueueLinks`/`InsertDoneCrawlURL`/
      `GetNextPendingCrawlURL`/`UpdateCrawlURLStatus`/`MarkCrawlURLFailed`/
      `ResetInProgressCrawlURLs`/`CountCrawlURLsByStatus`/`CountCrawlURLs`/
      `ForEachFailedCrawlURL(WithMessage)`/`ForEachCrawlURL(ByStatus)`/
      `GetCrawlJobStats`). Done — 16 new unit tests (81 total in the
      crate), clippy/fmt clean. Go's private `insertCrawlURLs` helper
      becomes this file's own private `insert_crawl_urls`, shared by
      `CrawlJob::create_with_urls` and `CrawlURL::bulk_insert`. The two
      `ForEach*` streaming iterators become `list_failed`/`list` returning
      a `Vec<Self>` instead of taking a row-streaming callback — a
      deliberate simplification, not a dropped capability. `crawl.go`'s
      query layer is now fully ported.
- [x] `rusty-hister-model` (`history.go` query layer): `history.go`'s
      search/pin/timeline helpers as `Link`/`History`/`HistoryLink`
      associated functions — `Link::get_or_create`/`History::get_or_create`
      and `HistoryLink::{delete_by_user_and_url,
      delete_by_user_query_and_url, set_pinned, record_selection,
      urls_by_query, latest_items, timestamps, suggest_query}` (Go:
      `GetOrCreateLink`/`GetOrCreateHistory`/`DeleteHistoryURL`/
      `DeleteHistoryItem`/`SetHistoryPinned`/`UpdateHistory`/
      `GetURLsByQuery`/`GetLatestHistoryItems(Filtered(ByDate))`/
      `GetHistoryItemTimestampsFilteredByDate`/`GetQuerySuggestion`). Done
      — 22 new unit tests (106 total in the crate), clippy/fmt clean.
      `latest_items` collapses Go's three `GetLatestHistoryItems*`
      argument-forwarding wrappers into one function taking a
      `HistoryItemsFilter`. Discovered while porting: Hister's
      `CommonFields.DeletedAt` is a plain `*time.Time`, not GORM's
      `gorm.DeletedAt`, so GORM never soft-deletes these tables in Go —
      `delete_by_user_and_url`/`delete_by_user_query_and_url` use a real
      `DELETE` rather than this crate's `#[table(soft_delete)]` column,
      matching Go and avoiding breaking `history_links`' non-partial
      unique index — see PROJECT-STATUS.md's new open item on what this
      means for the schema's other soft-delete columns.
- [x] `rusty-hister-model` (`user.go` query layer): `user.go`'s auth/token
      helpers as `User` associated functions — `create`/`create_oauth`/
      `delete_by_username`/`authenticate`/`get_by_token`/
      `regenerate_token`/`get_by_username`/`get_by_id`/
      `regenerate_token_by_username`/`rename`/`set_password`/
      `get_by_oauth_id`/`toggle_admin`/`rules_json`/`set_rules_json` (Go:
      `CreateUser`/`CreateOAuthUser`/`DeleteUser`/`AuthenticateUser`/
      `GetUserByToken`/`RegenerateToken`/`GetUser`/`GetUserByID`/
      `RegenerateTokenByUsername`/`UpdateUsername`/`UpdatePassword`/
      `GetUserByOAuthID`/`ToggleAdmin`/`GetUserRules`/`SaveUserRules`).
      Done — 25 new unit tests (131 total in the crate), clippy/fmt
      clean. This is the last of the six Go model files —
      `rusty-hister-model`'s query layer is now fully ported. A
      sovereignty-loop pass found no first-party `rusty_*` crate for
      password hashing, so `create`/`set_password` use Argon2id (`argon2`,
      already a workspace dependency via `rusty_croc`) rather than Go's
      bcrypt or a new dependency — flagged in PROJECT-STATUS.md for
      explicit sign-off. `authenticate` collapses Go's two distinct
      not-found/wrong-password errors into one `None` case, verified
      against the only real caller treating them identically.
      `GetUserRules`/`SaveUserRules`'s `config.Rules` parsing isn't
      ported (no such engine exists in this codebase yet); `rules_json`/
      `set_rules_json` stay at the raw-JSON level, same scope boundary
      `CrawlJob::validator_rules: Json` already draws.
- [x] `rusty-hister-extractor`: the chain-of-responsibility registry
      (§4.2) — `Registry::register`/`register_before` (case-insensitive
      duplicate rejection), the two-phase enrich-then-extract chain,
      the separate preview chain with starting-point selection, and
      `apply_configs`. Done — 18 unit tests, clippy/fmt clean. (The SDK
      contract itself, §4.1, landed with `rusty-hister-core`.)
- [x] `rusty-hister-extractor` (`JsonLdExtractor`, §4.5.5): the first of
      the 20 built-in extractors — enrich-only, parses
      `application/ld+json` script tags into normalized schema.org
      metadata (`type`/`headline`), flattening `@graph`/array wrappers
      and deep-sanitizing every non-`@`-prefixed string field. Done — 11
      new unit tests (29 total in the crate), clippy/fmt clean. Chosen
      first specifically because it could reuse this cluster's existing
      `rusty_json` dependency and needed only a small, purpose-built
      HTML-scanning/text-sanitizing helper (not a general HTML-parsing or
      sanitizer library), deferring the bigger, cross-cutting HTML-
      parsing/sanitizer dependency decision most of the remaining 19
      extractors need.
- [x] **Decided**: HTML-parsing/sanitizing dependency choice for the
      extractor cluster — `scraper` + `ammonia`, added directly to
      `rusty-hister-extractor`. A sovereignty-loop pass first checked
      `baileyrd/rusty_dbs` (at the user's suggestion) but it turned out
      unrelated and a poor fit (private, `UNLICENSED`, no wrapper API) —
      see PROJECT-STATUS.md's Resolved section for the full account.
- [x] `rusty-hister-extractor` (`EmbeddedVideoExtractor`, §4.5.3):
      enrich-only, scans `<video>`/`<source>`/`<iframe>`/`<embed>`/
      `<object>` elements for embedded video URLs, applying a known-host
      prefix allowlist to `iframe`/`embed`/`object` (`video`/`source` are
      trusted as first-party content, matching Go). Done — 9 new unit
      tests (38 total in the crate), clippy/fmt clean. First extractor
      built on the new `scraper`/`ammonia` dependencies (CSS-selector
      queries for the element scan; no sanitization needed here since
      URLs are stored as opaque strings, not rendered).
- [x] `rusty-hister-extractor` (`StackExchangeExtractor`, §4.5.7): extract
      *and* preview for SE-network question pages (Stack Overflow, Server
      Fault, Super User, Ask Ubuntu, `*.stackexchange.com`, and more).
      Done — 31 new unit tests across `stackexchange` (10),
      `sanitizer` (13), and `urlutil` (8) — 69 total in the crate,
      clippy/fmt clean.
      First extractor with real `preview()` output, so it also lands two
      new shared support modules future preview-capable extractors will
      reuse: `sanitizer` (`server/sanitizer/sanitizer.go` ported onto
      `ammonia`) and `urlutil` (`server/extractor/urlutil/urlutil.go`
      ported onto `scraper`'s tree-mutation API) — see PROJECT-STATUS.md
      for both modules' documented approximations relative to the Go
      originals (`bluemonday`'s per-attribute regex validation via a
      hand-rolled `attribute_filter` instead of a new `regex` dependency;
      a whole-document URL-rewrite pass instead of Go's per-selection
      scoping).
- [x] `rusty-hister-extractor` (`GoDocExtractor`, §4.5.8): preview-only,
      renders pkg.go.dev's `div.Documentation-content` element with
      `href`/`src` resolved to absolute URLs. Done — 8 new unit tests (77
      total in the crate), clippy/fmt clean. Go finds the element with a
      hand-rolled tokenizer/depth-tracker (`golang.org/x/net/html` has no
      CSS-selector API); the Rust port collapses to a single class
      selector plus `ElementRef::html()` since `scraper` already builds a
      full DOM. When no matching element exists, Go's `Preview`
      *succeeds* with empty content rather than falling back (its
      tokenizer loop just reaches EOF having never entered the "in
      article" state) — reproduced faithfully rather than "corrected".
- [x] `rusty-hister-extractor` (`LobstersExtractor`, §4.5.10): extract
      *and* preview for lobste.rs story pages — submission metadata,
      story body, and the full recursively-nested comment tree. Done —
      6 new unit tests (83 total in the crate), clippy/fmt clean. The
      comment tree is genuinely recursive (`li.comments_subtree` nests
      `ol.comments > li.comments_subtree` arbitrarily deep); walked with
      `scraper`'s `ElementRef::child_elements()` (direct children only,
      to avoid double-visiting deeper subtrees a broader descendant
      selector would catch), mirroring Go's own recursive helpers.
      Reuses `StackExchangeExtractor`'s selector/text/escaping helpers
      (promoted to `pub(crate)`) rather than a third copy of each.
      Preserves a real Go asymmetry rather than "fixing" it: comment
      author/score/timestamp go into the accumulated HTML unescaped
      (unlike the story header/byline) — no observable effect either
      way, since the whole string is sanitized before being returned.
- [x] `rusty-hister-extractor` (`HackerNewsExtractor`, §4.5.11): extract
      *and* preview for news.ycombinator.com item pages. Done — 8 new
      unit tests (98 total in the crate), clippy/fmt clean. Unlike
      Lobsters, comments here are a *flat* table with each row's depth
      carried by an `indent` attribute, reconstructed into nested
      `<ul>`/`<li>` lists by tracking that number across the row
      sequence (a small state machine, not recursion). The first
      extractor to need a new shared `textutil` module — a port of
      `server/extractor/textutil/textutil.go`, which flattens an HTML
      subtree to plain text turning block-element boundaries and `<br>`
      into line breaks (so multi-paragraph comment bodies don't run
      together, unlike a bare text-node concatenation). Go itself shares
      `textutil` across `hackernews`/`discourse`/`reddit`, so it's
      already positioned for reuse when those land. `ego-tree` (already
      pinned transitively by `scraper`) is now a direct dependency too,
      needed to name `NodeRef` in `textutil`'s recursive tree walk.
- [x] `rusty-hister-extractor` (`GitHubExtractor`, §4.5.9): extract *and*
      preview for repository overview, issue, issue-list, and
      pull-request pages. Done — 8 new unit tests (106 total in the
      crate), clippy/fmt clean. Go matches these four URL shapes with
      independent regexes; hand-rolled here as small string/character-
      class predicates instead of adding a `regex` dependency for what
      are fairly mechanical path-shape checks — two Go regex quirks
      (a single-non-slash-char fragment allowance on issue URLs vs. a
      full run on pull-request URLs) reproduced exactly rather than
      "corrected". The README HTML comes from an embedded
      `<script type="application/json">` payload, parsed with
      `rusty_json` (already this crate's dependency) rather than adding
      `serde_json`. `Preview` doesn't re-sanitize its whole buffer the
      way prior extractors do — only the README HTML passes through
      `sanitizer::sanitize_html`, matching a real Go asymmetry.
- [x] `rusty-hister-extractor` (`ChatGptExtractor`, §4.5.18): extract
      *and* preview for chatgpt.com conversation URLs (authenticated,
      public-shared, and custom-GPT). Done — 7 new unit tests (113 total
      in the crate), clippy/fmt clean. `scraper::ElementRef` is read-only,
      so Go's clone-then-remove content-cleaning pattern has no direct
      equivalent; ported instead by copying only the kept nodes into a
      fresh `ego_tree`-backed fragment (`Html::new_fragment()` +
      `NodeMut::append()`). Go's own conversation-text writer is a
      superset of `textutil` (adds list-bullet and table-cell-separator
      handling) and isn't built on it either, so this port mirrors that
      with its own `ConversationTextWriter` rather than generalizing
      `textutil` speculatively — reusing only its final `normalize_text`
      whitespace pass. The first extractor to report
      `ExtractOutcome`/`PreviewOutcome::Abort` (matched URL, no visible
      turns) rather than `Fallback`, matching Go's own `AbortExtraction`.
- [x] `rusty-hister-extractor` (`BasicExtractor`, §4.4): extract *and*
      preview, the universal last-resort fallback. Done — 8 new unit
      tests (121 total in the crate), clippy/fmt clean. `matches` always
      returns `true`; this only works because a real chain places it
      last, after everything more specific has had its chance. Go walks
      the raw byte stream with its own HTML tokenizer, tracking "inside
      `<body>`"/"inside `<script>`/`<style>`/`<noscript>`" by hand; ported
      here as a `scraper`-based walk of the parsed `<body>` element's
      subtree instead — simpler, but not quite equivalent for a fragment
      with no `<body>` tag at all (documented in the module rather than
      reproduced, since `html5ever` always synthesizes one and this has no
      practical effect on real crawled pages). Text nodes are concatenated
      with no separators at all, deliberately cruder than `textutil`'s
      block-aware flattening — matching Go's own token-by-token
      concatenation exactly.
- [x] `rusty-hister-extractor` (`WikipediaExtractor`, §4.5.12): extract
      *and* preview for `*.wikipedia.org/wiki/...` article pages. Done —
      10 new unit tests (131 total in the crate), clippy/fmt clean. The
      largest port so far. Go's `goquery` mutates its parse tree in place
      (`.Remove()`, `.SetAttr()`, `.ReplaceWithHtml()`), which
      `scraper::ElementRef` has no equivalent for (it's a read-only view);
      ported here as `NodeId`-based mutation of the same `ego_tree`
      instead — attribute changes via `Tree::get_mut`, removals via
      `NodeMut::detach`, an element swap for the `<video>`-to-`<img>`
      poster replacement — always collecting the `NodeId`s a selector pass
      needs into an owned `Vec` before mutating, since an `ElementRef`
      can't stay borrowed across a `get_mut` call. Needed `html5ever` as a
      new direct dependency (already pinned transitively by `scraper` at
      this same version) to construct attribute names/values by hand. One
      Go behavior isn't reproduced: wrapping a wikitable in a
      horizontally-scrolling `<div>` for preview, which has no cheap
      `NodeId`-based equivalent and isn't covered by Go's own tests.
- [x] `rusty-hister-extractor` (`RedditExtractor`, §4.5.6): extract *and*
      preview for Reddit post pages. Done — 8 new unit tests (139 total in
      the crate), clippy/fmt clean. Reddit has shipped at least three
      different markups for the same post over the years — modern
      `shreddit-*` web components, the legacy `old.reddit.com` DOM, and a
      `schema.org` JSON-LD block many pages embed regardless of which HTML
      renders — so, like Go, this port copes with an ordered list of
      CSS-selector candidates (first non-empty/first-match wins) rather
      than branching on "which Reddit era is this" up front. The crate's
      third real `textutil` caller. Reuses two tricks already established
      for `scraper::ElementRef`'s read-only API: a post/comment body's URL
      rewriting re-parses that subtree's own HTML as a standalone
      fragment (`WikipediaExtractor::extract`'s clone trick), and reading
      a comment's own text when it has no dedicated body element copies
      only the kept nodes into a fresh `ego_tree` fragment
      (`ChatGptExtractor`'s content-cleaning approach) so a nested reply's
      text isn't double-counted into its parent's.
- **Blocked, flagged rather than silently ported without it**: Mastodon,
  Bluesky, and Twitter (§4.5.13-15) each decompose one timeline/thread
  page into multiple indexed documents (Go: `Document.ExtraDocuments`/
  `SkipIndexing`), a capability `rusty-hister-core`'s `Document`/
  `ExtractOutcome` don't model yet. See PROJECT-STATUS.md's Open items
  for the design question this needs before any of the three can land.
- `rusty-hister-extractor`: the remaining 9 built-in extractors in
  default-chain order (§4.3) not blocked on the above, starting with the
  ones that have existing Go test coverage and budgeting fresh test
  authorship for the rest. Markdown/Org (§4.5.1-2) instead need a
  markdown/org-mode parser, a separate dependency choice of their own,
  not yet made. Readability (§4.4) needs a similar dependency decision of
  its own (a Rust Readability-algorithm implementation, or a fresh port of
  Go's `go-readability`).
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
