# PROJECT-STATUS: rusty_hister

Last updated: 2026-09-13 (Phase 1: `rusty-hister-extractor`'s ninth
concrete extractor, Basic — the universal last-resort fallback (capability
inventory §4.4), always matches, strips markup with no block-aware
separators at all. Mastodon/Bluesky/Twitter flagged as blocked on a
`rusty-hister-core` capability gap — Go's per-page multi-document
extraction — rather than silently ported without it).

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
  and config merging (`apply_configs`). **Nine concrete extractors so far:**
  `JsonLdExtractor` (capability inventory §4.5.5) — enrich-only, parses
  `application/ld+json` script tags into normalized schema.org metadata
  (`type`/`headline`), flattening `@graph`/array wrappers and deep-
  sanitizing every non-`@`-prefixed string field. Chosen as the first of
  the 20 built-in extractors specifically because it could reuse this
  cluster's existing `rusty_json` dependency for JSON parsing and needed
  only a small, purpose-built HTML-scanning/text-sanitizing helper rather
  than a general HTML-parsing or sanitizer library, deferring that bigger
  dependency decision. **`EmbeddedVideoExtractor`** (capability inventory
  §4.5.3) — enrich-only, scans `<video>`/`<source>`/`<iframe>`/`<embed>`/
  `<object>` elements for embedded video URLs, applying a known-host
  prefix allowlist to `iframe`/`embed`/`object` (not `video`/`source`,
  which are trusted as first-party content, matching Go). The first
  extractor to use `scraper` (CSS-selector HTML parsing) and `ammonia`
  (HTML sanitizing), added directly to this crate's dependencies — see
  the now-resolved open item below for the sovereignty-loop pass and
  false start (an unrelated private repo, `rusty_dbs`) that preceded
  this. **`StackExchangeExtractor`** (capability inventory §4.5.7) —
  extract *and* preview: pulls the question and every already-rendered
  answer from a Stack Exchange network question page (Stack Overflow,
  Server Fault, Super User, Ask Ubuntu, `*.stackexchange.com`, and a few
  more). The first extractor with real rendered-HTML preview output
  (rather than an enrich-only fallback or a plain fallback response), so
  it's also the first to need two new shared support modules other
  preview-capable extractors will reuse: `sanitizer` (a Rust port of
  `server/sanitizer/sanitizer.go` on `ammonia`, replacing Go's
  `bluemonday`, including a hand-rolled reproduction of its SVG
  attribute-value allow-list via `ammonia`'s `attribute_filter` callback
  rather than adding a `regex` dependency) and `urlutil` (a port of
  `server/extractor/urlutil/urlutil.go`'s relative-to-absolute URL
  rewriting, done as a whole-document pass via `scraper`'s tree-mutation
  API rather than Go's per-`goquery.Selection` scoping — documented in
  the module itself as harmless for every current caller, since each one
  serializes only the specific subtree it cares about afterward).
  **`GoDocExtractor`** (capability inventory §4.5.8) — preview-only,
  renders pkg.go.dev's `div.Documentation-content` element with its
  `href`/`src` attributes resolved to absolute URLs. Go finds that element
  with a hand-rolled tokenizer that reconstructs HTML byte-by-byte while
  tracking tag-nesting depth (`golang.org/x/net/html` has no CSS-selector
  API); since `scraper` already builds a full DOM, the Rust port collapses
  to a single class selector plus `ElementRef::html()` to serialize the
  matched subtree. When no such element is present, Go's tokenizer loop
  reaches end-of-input having never entered the "in article" state and
  returns an empty string with a `nil` error — i.e. `Preview` *succeeds*
  with empty content rather than falling back — reproduced faithfully
  rather than "corrected" to a `Fallback`.
  **`LobstersExtractor`** (capability inventory §4.5.10) — extract *and*
  preview, pulling the submission metadata, story body, and full
  recursively-nested comment tree from a lobste.rs story page. The
  comment tree is genuinely recursive (a `li.comments_subtree` nests
  `ol.comments > li.comments_subtree` arbitrarily deep), walked with the
  same shape as Go's own recursive helpers but via `scraper`'s
  `ElementRef::child_elements()` (direct children only, to avoid
  double-visiting deeper subtrees a broader descendant selector would
  catch). Reuses `StackExchangeExtractor`'s small selector/text/escaping
  helpers (promoted to `pub(crate)`) rather than a third copy of each.
  Preserves a real Go asymmetry rather than "fixing" it: comment
  author/score/timestamp are interpolated into the accumulated HTML
  unescaped (unlike the story header/byline, which does escape) — with
  no observable effect either way, since the whole accumulated string
  still passes through `sanitizer::sanitize_html` before being returned.
  **`HackerNewsExtractor`** (capability inventory §4.5.11) — extract
  *and* preview for news.ycombinator.com item pages. Unlike Lobsters,
  comments here are a *flat* table with each row's depth carried by an
  `indent` attribute on its leading `td.ind` cell, reconstructed into
  nested `<ul>`/`<li>` lists by tracking that number across the row
  sequence (a small state machine, not recursion). The first extractor
  to need a new shared `textutil` module — a Rust port of
  `server/extractor/textutil/textutil.go`, which flattens an HTML
  subtree to plain text while turning block-element boundaries and
  `<br>` into line breaks (unlike a bare text-node concatenation, which
  runs multi-paragraph comment bodies together). Go itself shares
  `textutil` across `hackernews`/`discourse`/`reddit`, so this port is
  already positioned for reuse when those extractors land. `textutil`'s
  recursive tree walk names `ego_tree::NodeRef` directly, so `ego-tree`
  (already pinned transitively by `scraper` at the same version) is now
  also a direct dependency of this crate.
  **`GitHubExtractor`** (capability inventory §4.5.9) — extract *and*
  preview for repository overview, issue, issue-list, and pull-request
  pages. Go matches these four URL shapes with independent regexes;
  since a `regex` dependency would only ever be exercised for these
  fairly mechanical path-shape checks, this port hand-rolls the same
  checks as small string/character-class predicates on the URL with its
  `https://github.com/` prefix and `<owner>/<repo>` segment stripped.
  Two Go regex quirks are reproduced exactly rather than "corrected"
  (documented in the module itself): the issue-URL pattern allows only a
  *single* non-slash character after a `#` fragment marker (almost
  certainly meant to be `[^/]*`), while the pull-request pattern allows
  a full non-slash run. The README HTML comes from an embedded
  `<script type="application/json">` payload, parsed with `rusty_json`
  (already this crate's dependency for `Metadata`) rather than adding
  `serde_json`. `Preview` doesn't re-sanitize its whole accumulated
  buffer the way the extractors above do — only the embedded README
  HTML passes through `sanitizer::sanitize_html`, matching a real Go
  asymmetry (the surrounding metadata card is built from HTML-escaped
  plain strings, already safe without a second pass).
  **`ChatGptExtractor`** (capability inventory §4.5.18) — extract *and*
  preview for chatgpt.com conversation URLs (authenticated, public-shared,
  and custom-GPT). `scraper::ElementRef` is read-only, so Go's
  clone-then-remove content-cleaning pattern has no direct equivalent;
  this port instead copies only the kept nodes into a fresh
  `ego_tree`-backed fragment (`Html::new_fragment()` + `NodeMut::append()`)
  rather than mutating the parsed document. Go's own conversation-text
  writer is a superset of `textutil` (it also handles list bullets and
  table-cell separators) and isn't built on `textutil` either, so this
  port mirrors that with its own `ConversationTextWriter` rather than
  generalizing `textutil` speculatively for a shape only this one
  extractor needs — reusing only its final `normalize_text` whitespace
  pass. The first extractor to report `ExtractOutcome`/
  `PreviewOutcome::Abort` (a matched conversation URL with no visible
  turns) rather than `Fallback`, matching Go's own `AbortExtraction`
  since that's a dead end for the whole chain, not a case for the next
  extractor to try.
  **`BasicExtractor`** (capability inventory §4.4) — extract *and*
  preview, the universal last-resort fallback: strips markup from any
  HTML document and keeps whatever plain text and `<title>` remain.
  `matches` always returns `true`; this only works because a real chain
  places it last. Go walks the raw byte stream with its own HTML
  tokenizer, tracking "inside `<body>`"/"inside `<script>`/`<style>`/
  `<noscript>`" by hand; this port instead selects the parsed `<body>`
  element via `scraper` and walks its subtree — simpler, but not quite
  equivalent for a fragment with no `<body>` tag at all (Go would find no
  text there; `html5ever` always synthesizes one), a difference documented
  in the module rather than reproduced, since it has no practical effect
  on real crawled pages. Text nodes are concatenated with no separators at
  all — deliberately cruder than `textutil`'s block-aware flattening,
  matching Go's own token-by-token concatenation exactly. `Preview`
  doesn't derive anything from `document.html`; like Go, it just
  HTML-escapes whatever `document.text` already holds, succeeding with
  empty content when there is none rather than falling back (the same
  "succeeds empty" quirk `GoDocExtractor` preserves).
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
  extended here to a multi-statement transaction. **`CrawlURL`'s own queue
  mechanics are ported too**: `insert_if_not_exists`/`bulk_insert`/
  `mark_done_and_enqueue_links`/`insert_done`/`next_pending`/
  `update_status`/`mark_failed`/`reset_in_progress`/`count_by_status`/
  `count`/`list_failed`/`list`/`job_stats` (Go: `InsertCrawlURLIfNotExists`/
  `BulkInsertCrawlURLs`/`MarkCrawlURLDoneAndEnqueueLinks`/
  `InsertDoneCrawlURL`/`GetNextPendingCrawlURL`/`UpdateCrawlURLStatus`/
  `MarkCrawlURLFailed`/`ResetInProgressCrawlURLs`/
  `CountCrawlURLsByStatus`/`CountCrawlURLs`/
  `ForEachFailedCrawlURL(WithMessage)`/`ForEachCrawlURL(ByStatus)`/
  `GetCrawlJobStats`). Go's private `insertCrawlURLs` helper — shared by
  `CreateNamedCrawlJobWithURLs` and `BulkInsertCrawlURLs` — becomes this
  file's own private `insert_crawl_urls`, reused the same way by
  `CrawlJob::create_with_urls` and `CrawlURL::bulk_insert`. The two
  `ForEach*` streaming iterators become `list_failed`/`list` returning a
  `Vec<Self>` instead of taking a row-streaming callback — a deliberate
  simplification, not a dropped capability: every row Go's callback would
  see is still reachable, just batched. `crawl.go`'s full query layer is
  now ported. **`history.go`'s query layer is ported too**:
  `Link::get_or_create`/`History::get_or_create` (Go: `GetOrCreateLink`/
  `GetOrCreateHistory`) and `HistoryLink::{delete_by_user_and_url,
  delete_by_user_query_and_url, set_pinned, record_selection,
  urls_by_query, latest_items, timestamps, suggest_query}` (Go:
  `DeleteHistoryURL`/`DeleteHistoryItem`/`SetHistoryPinned`/
  `UpdateHistory`/`GetURLsByQuery`/`GetLatestHistoryItems(Filtered(ByDate))`/
  `GetHistoryItemTimestampsFilteredByDate`/`GetQuerySuggestion`).
  `latest_items` collapses Go's three `GetLatestHistoryItems*` wrappers
  (pure argument-forwarding with different defaults, not different
  behaviors) into one function taking a `HistoryItemsFilter`. A notable
  discovery while porting this file: Hister's `CommonFields.DeletedAt` is
  a plain `*time.Time`, not GORM's own `gorm.DeletedAt` sentinel type, so
  GORM never actually soft-deletes `History`/`Link`/`HistoryLink` rows —
  `DeleteHistoryURL`/`DeleteHistoryItem` are genuine hard deletes in Go,
  so `delete_by_user_and_url`/`delete_by_user_query_and_url` reproduce
  that with a real `DELETE`, not a soft-delete `UPDATE` (which would also
  have broken re-recording history for the same URL, since
  `history_links`' unique index isn't scoped to active rows) — see this
  update's new open item below for what this means for the rest of the
  schema's `#[table(soft_delete)]` columns. **`user.go`'s query layer is
  ported too — the last of `rusty-hister-model`'s six Go model files,
  completing this crate's query layer**: `User::{create, create_oauth,
  delete_by_username, authenticate, get_by_token, regenerate_token,
  get_by_username, get_by_id, regenerate_token_by_username, rename,
  set_password, get_by_oauth_id, toggle_admin, rules_json,
  set_rules_json}` (Go: `CreateUser`/`CreateOAuthUser`/`DeleteUser`/
  `AuthenticateUser`/`GetUserByToken`/`RegenerateToken`/`GetUser`/
  `GetUserByID`/`RegenerateTokenByUsername`/`UpdateUsername`/
  `UpdatePassword`/`GetUserByOAuthID`/`ToggleAdmin`/`GetUserRules`/
  `SaveUserRules`). A sovereignty-loop pass found no first-party `rusty_*`
  crate for password hashing, so `create`/`set_password` hash with
  **Argon2id, not Go's bcrypt** — `argon2` is already a workspace
  dependency (`rusty_croc`'s PAKE handshake), so this reuses it rather
  than adding a second password-hashing crate; salt bytes come from
  `rusty_rand`, not argon2's own optional `rand` feature. This is a
  deliberate algorithm change flagged for explicit sign-off (see the new
  open item below), not a capability drop: every password this crate ever
  hashes is freshly created here, under the current fresh-install-only
  working assumption. `authenticate` collapses Go's
  `ErrUserNotFound`/`ErrInvalidPassword` into one `None` case — verified
  against the only real caller (`server/endpoints.go`'s `serveLogin`),
  which already treats both identically. `Go`'s `ParseRules`/
  `config.Rules` (a compiled-regex config-rules engine) isn't ported;
  `rules_json`/`set_rules_json` read/write the stored JSON blob as-is,
  the same scope boundary `CrawlJob::validator_rules: Json` already
  draws.

All three have unit tests, `clippy`, and `fmt` clean. `rusty-hister-model`'s
query layer is now fully ported; `rusty-hister-extractor` has its first
concrete extractor (JSON-LD, 19 to go) — the rest of the built-in
extractors and `rusty-hister-crawler`'s `http` backend are the rest of
Phase 1 per `docs/roadmap/ROADMAP.md`.

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
- **HTML-parsing and HTML-sanitizing library choice for the extractors**
  — resolved 2026-09-13, per the user: add `scraper` (CSS-selector HTML
  parsing, built on `html5ever`) and `ammonia` (HTML sanitizing, also
  `html5ever`-based) directly as `rusty-hister-extractor` dependencies.
  A sovereignty-loop pass first checked `baileyrd/rusty_dbs` (a private,
  `UNLICENSED` personal repo, at the user's suggestion) — it turned out
  to be unrelated (a subprocess/editor-integration utility crate,
  `dbs-connector-support`, that had `scraper`/`ammonia` added bare with
  no wrapper API) and a poor fit regardless (no license grant compatible
  with this workspace's `MIT OR Apache-2.0`, and `rusty_mill`'s own
  convention is a full `git subtree` merge for a first-party sibling, not
  a permanent pinned dependency on an external repo). `EmbeddedVideoExtractor`
  (capability inventory §4.5.3) is the first extractor built on the new
  dependencies. Both are common, well-maintained crates.io crates sharing
  the same underlying `html5ever` parser, so only one HTML-parsing engine
  enters the dependency tree even though two crates were added.

## Open items carried from the capability inventory (not blocking, but
unresolved — see `docs/capability-inventory/HISTER-CAPABILITY-INVENTORY.md`
§13 and inline flags)

- `sqlite-vec` C-extension story for the SQLite vectorstore backend — folded
  into ADR-0002 rather than decided separately, since it's downstream of the
  storage-engine choice.
- **Per-page multi-document extraction** (Go: `Document.ExtraDocuments`,
  `Document.SkipIndexing`) — discovered while scoping the Mastodon
  extractor (capability inventory §4.5.13, 2026-09-13). Mastodon,
  Bluesky, and Twitter (§4.5.13-15) don't extract their own page as one
  document; they decompose a timeline/thread page into *one indexed
  document per toot/post/tweet already rendered on it*, via
  `d.ExtraDocuments = append(...)`, while setting `d.SkipIndexing = true`
  on the container page itself. `rusty-hister-core`'s `Document`/
  `ExtractOutcome` have no equivalent yet — `ExtractOutcome::Extracted`
  carries exactly one `Document`. This blocks all three social-media
  extractors until deliberately designed (does `ExtractOutcome` grow an
  `ExtractedMany(Vec<Document>)` variant, or does `Document` grow its own
  `extra_documents`/`skip_indexing` fields the way Go's does, and how
  does `Registry`/the eventual indexer consume either shape?) — not
  silently dropped or worked around with a lossy single-document
  approximation.
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
- **`rusty-hister-model`'s query layer is now fully ported** (all six Go
  model files' domain operations — the embedding-queue state machine,
  `WebSession`, `DocumentVersion`, `crawl.go`, `history.go`, and now
  `user.go` — see the crate status table below). Nothing left open here;
  kept as a resolved-item record of the split `rusty-hister-extractor`
  used (mechanism before concrete extractors), now finished on this
  side.
- **Password hashing: Argon2id, not Go's bcrypt** — a sovereignty-loop
  pass while porting `user.go` (2026-09-12) found no first-party
  `rusty_*` crate for password hashing, so `User::create`/`set_password`
  use `argon2` (already a workspace dependency via `rusty_croc`) instead
  of adding a second password-hashing crate or reimplementing bcrypt.
  Every password this crate ever hashes is freshly created here (the
  fresh-install-only working assumption above), so there's no existing
  bcrypt hash to stay compatible with today — but if the "open a
  pre-existing Hister-Go-created database" question above is ever
  resolved toward "yes," a bcrypt-hashed row from a real Hister install
  would fail to verify against this crate's Argon2id-only `verify_password`,
  since the two hash formats aren't interchangeable. Flagged for explicit
  sign-off rather than decided unilaterally, same reasoning as the
  soft-delete item below.
- **Hister's `CommonFields.DeletedAt` is not GORM's `gorm.DeletedAt`
  sentinel type** — it's a plain, GORM-invisible `*time.Time` — discovered
  while porting `history.go`'s `DeleteHistoryURL`/`DeleteHistoryItem`
  (2026-09-12): GORM never treats any Hister model as soft-delete-enabled,
  so every `DB.Delete(...)` call in the Go codebase is a genuine hard
  delete, and the `deleted_at` column is otherwise inert. This crate's
  schema increment gave `History`/`Link`/`HistoryLink`/`User`/
  `DocumentVersion` a `#[table(soft_delete)] deleted: bool` column
  (mirroring `CommonFields`' *shape*, before this nuance was known) that
  Go's own equivalent operations never actually set — `history.go`'s
  query layer only relies on it for read-side filtering
  (`Mapped::not_deleted_filter()`, a no-op today since nothing sets it),
  and its two delete functions use a real `DELETE` instead.
  `User::delete_by_username` (`user.go` increment, same day) follows the
  same pattern for the same reason. Whether the soft-delete columns on
  the other four affected models should be removed entirely (closer to
  Go's real behavior) or kept as a deliberate, documented Rust-side
  improvement is not yet decided — flagged for explicit sign-off rather
  than resolved unilaterally, since it's a schema change on already-merged
  tables.
- Postgres path for `rusty-hister-model`'s raw-SQL query-layer functions
  (`EmbeddingJob::enqueue`/`retry`/`release`, `WebSession::create`,
  `DocumentVersion::save`, `CrawlJob::create_with_urls`,
  `CrawlURL::{bulk_insert, mark_done_and_enqueue_links, insert_done}`,
  `User::{create, create_oauth}`) is untested —
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
| `rusty-hister-model` | **Query layer complete** — schema: nine `#[derive(Mapped)]` types (`User`, `Link`, `History`, `HistoryLink`, `CrawlJob`, `CrawlURL`, `WebSession`, `DocumentVersion`, `EmbeddingJob`), soft-delete via `rusty_db`'s `#[table(soft_delete)]`, fresh-install SQLite+Postgres migrations via `rusty_db::Migrator`. Query layer: `EmbeddingJob`'s embedding-queue state machine (9 functions), `WebSession`'s lookup/expiry helpers (4 functions), `DocumentVersion`'s save/move/count/list helpers (5 functions), all of `crawl.go` (`CrawlJob`'s lifecycle, 7 functions, plus `CrawlURL`'s queue mechanics, 12 functions), all of `history.go` (`Link`/`History::get_or_create`, plus `HistoryLink`'s 8 query functions), and all of `user.go` (15 functions: account CRUD, Argon2id password hashing/verification, token issuance, admin toggling, raw rules-JSON get/set) — **all six Go model files' query layers are now ported**. 131 unit tests (real SQLite round-trips, unique-constraint/duplicate-rejection checks, migration up/down/status, the embedding queue's dedup/claim/retry/dirty-job semantics, `WebSession`'s create/get/refresh/delete round trips, `DocumentVersion`'s save/list/count/move/list_until behavior, `CrawlJob`/`CrawlURL`'s full lifecycle and queue-mechanics behavior, `history.go`'s get-or-create/pin/record-selection/delete/ranking/pagination/filtering/suggestion behavior, and `user.go`'s create/authenticate/delete/token/rename/password/oauth/admin/rules behavior), clippy/fmt clean. |
| `rusty-hister-extractor` | **In progress** — `Registry` (chain-of-responsibility: ordered registration, two-phase enrich/extract, preview-chain starting points, config merging), plus `JsonLdExtractor` (§4.5.5), `EmbeddedVideoExtractor` (§4.5.3), `StackExchangeExtractor` (§4.5.7), `GoDocExtractor` (§4.5.8), `LobstersExtractor` (§4.5.10), `HackerNewsExtractor` (§4.5.11), `GitHubExtractor` (§4.5.9), `ChatGptExtractor` (§4.5.18), and `BasicExtractor` (§4.4) as concrete extractors, and shared support modules (`sanitizer`, `urlutil`, `textutil`) other extractors reuse. 121 unit tests, clippy/fmt clean. `scraper`/`ammonia` now available for the remaining 11 built-in extractors that need real HTML parsing/sanitizing (Mastodon/Bluesky/Twitter additionally blocked on a core capability gap — see Open items). |
| `rusty-hister-indexer` | Skeleton only — unblocked by ADR-0002, not yet started |
| `rusty-hister-vectorstore` | Skeleton only — unblocked by ADR-0002, not yet started |
| `rusty-hister-crawler` | Skeleton only — CDP backend unblocked by ADR-0003, not yet started; BiDi backend out of v1 scope |
| `rusty-hister-server` | Skeleton only |
| `rusty-hister-mcp` | Skeleton only |
