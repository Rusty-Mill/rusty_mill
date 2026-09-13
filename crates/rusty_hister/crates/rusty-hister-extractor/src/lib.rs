//! Extractor SDK and the built-in per-site/format content extractors — a
//! Rust port of `server/extractor/`.
//!
//! This crate owns the extractor *chain*: [`Registry`], the
//! chain-of-responsibility mechanism capability inventory §4.2 describes
//! (ordered registration, two-phase enrich-then-extract execution, a
//! separate preview chain with starting-point selection). The `Extractor`
//! trait itself and its supporting types (`Document`, `Capabilities`,
//! `ExtractOutcome`, ...) live in `rusty-hister-core`, re-exported here for
//! convenience — see that crate before this one for the contract every
//! concrete extractor implements.
//!
//! **Twelve concrete extractors so far:**
//!
//! - [`JsonLdExtractor`] (capability inventory §4.5.5) — enrich-only,
//!   parses `application/ld+json` script tags. Hand-rolls its own narrow
//!   HTML-scanning and text-sanitization needs (see its own module doc);
//!   landed before this crate had a general HTML-parsing dependency.
//! - [`EmbeddedVideoExtractor`] (capability inventory §4.5.3) —
//!   enrich-only, scans `<video>`/`<source>`/`<iframe>`/`<embed>`/
//!   `<object>` elements for embedded video URLs. The first extractor to
//!   use `scraper` (CSS-selector HTML parsing) and `ammonia` (HTML
//!   sanitizing) — both added to this crate's dependencies after a
//!   sovereignty-loop pass found no first-party `rusty_*` crate for
//!   either (see `docs/PROJECT-STATUS.md`'s resolved open item).
//! - [`StackExchangeExtractor`] (capability inventory §4.5.7) — extract
//!   *and* preview, the first extractor with real rendered-HTML preview
//!   output. Built on two new shared support modules other preview-capable
//!   extractors will reuse: `sanitizer` (a Rust port of
//!   `server/sanitizer/sanitizer.go`, `ammonia`-based) and `urlutil` (a
//!   port of `server/extractor/urlutil/urlutil.go`, relative-to-absolute
//!   URL rewriting).
//! - [`GoDocExtractor`] (capability inventory §4.5.8) — preview-only,
//!   renders pkg.go.dev's `div.Documentation-content` element with its
//!   links resolved to absolute URLs. Go finds that element with a
//!   hand-rolled tokenizer/depth-tracker; the Rust port collapses to a
//!   single CSS class selector since `scraper` already builds a full DOM.
//! - [`LobstersExtractor`] (capability inventory §4.5.10) — extract *and*
//!   preview, pulling the submission metadata, story body, and full
//!   recursively-nested comment tree from a lobste.rs story page. Reuses
//!   `StackExchangeExtractor`'s small selector/text/escaping helpers
//!   (now `pub(crate)` in that module) rather than duplicating them a
//!   third time.
//! - [`HackerNewsExtractor`] (capability inventory §4.5.11) — extract
//!   *and* preview for news.ycombinator.com item pages. Unlike Lobsters,
//!   comments here are a *flat* table with depth carried by an `indent`
//!   attribute, reconstructed into nested lists by tracking that number
//!   across the row sequence rather than recursing. The first extractor
//!   to need `textutil` (a new shared, block-aware HTML-to-text
//!   flattener — a Rust port of `server/extractor/textutil/textutil.go`,
//!   which Go itself shares across `hackernews`/`discourse`/`reddit`).
//! - [`GitHubExtractor`] (capability inventory §4.5.9) — extract *and*
//!   preview for repository overview, issue, issue-list, and pull-request
//!   pages. Hand-rolls Go's four regex-based URL-shape checks as small
//!   string predicates rather than adding a `regex` dependency for what
//!   are fairly mechanical path-shape checks (see the module doc for the
//!   two Go regex quirks reproduced faithfully). `Preview` doesn't
//!   re-sanitize its whole accumulated buffer the way the extractors
//!   above do — only the embedded README HTML passes through
//!   `sanitizer::sanitize_html`, matching a real Go asymmetry.
//! - [`ChatGptExtractor`] (capability inventory §4.5.18) — extract *and*
//!   preview for chatgpt.com conversation URLs (authenticated, shared, and
//!   custom-GPT). `scraper::ElementRef` is read-only, so Go's
//!   clone-then-remove content-cleaning pattern has no direct equivalent;
//!   this port instead copies only the kept nodes into a fresh
//!   `ego_tree`-backed fragment (`Html::new_fragment()` +
//!   `NodeMut::append()`). Go's own conversation text writer is a
//!   superset of `textutil` (it also handles list bullets and table-cell
//!   separators) and isn't built on it, so this port mirrors that with
//!   its own `ConversationTextWriter` rather than generalizing `textutil`
//!   speculatively — reusing only its final `normalize_text` whitespace
//!   pass. The first extractor to use `ExtractOutcome`/`PreviewOutcome`'s
//!   `Abort` variant: a matched conversation URL with no visible turns is
//!   a dead end for the whole chain, not a "try the next extractor" case.
//! - [`BasicExtractor`] (capability inventory §4.4) — extract *and*
//!   preview, the universal last-resort fallback: strips markup from any
//!   HTML document and keeps whatever plain text and `<title>` remain.
//!   `matches` always returns `true`; this only works because a real
//!   chain places it last, after everything more specific has already had
//!   its chance. Deliberately cruder than `textutil`'s block-aware
//!   flattening — text nodes are concatenated with no separators at all,
//!   matching Go's own token-by-token concatenation exactly.
//! - [`WikipediaExtractor`] (capability inventory §4.5.12) — extract *and*
//!   preview for `*.wikipedia.org/wiki/...` article pages: article text,
//!   infobox key/value pairs, and wikitables for extraction; a richly
//!   styled preview (inline styles standing in for Wikipedia's own,
//!   sanitizer-stripped CSS classes) for rendering. The largest port so
//!   far — Go's `goquery` mutates its parse tree in place (`.Remove()`,
//!   `.SetAttr()`, `.ReplaceWithHtml()`), which `scraper::ElementRef` has
//!   no equivalent for; this port instead mutates the same `ego_tree` by
//!   `NodeId` (attribute changes via `Tree::get_mut`, removals via
//!   `NodeMut::detach`), always collecting the `NodeId`s a selector pass
//!   needs into an owned `Vec` before mutating (see the module's own doc).
//!   One cosmetic-only Go behavior isn't reproduced: wrapping a wikitable
//!   in a horizontally-scrolling `<div>`, which has no cheap `NodeId`-based
//!   equivalent and isn't covered by Go's own tests.
//! - [`RedditExtractor`] (capability inventory §4.5.6) — extract *and*
//!   preview for Reddit post pages. Reddit has shipped at least three
//!   different markups for the same post over the years — modern
//!   `shreddit-*` web components, the legacy `old.reddit.com` DOM, and a
//!   `schema.org` JSON-LD block many pages embed regardless of which HTML
//!   renders — so, like Go, this port copes with an ordered list of
//!   CSS-selector candidates (first non-empty/first-match wins) rather
//!   than branching on "which Reddit era is this" up front. The crate's
//!   third real `textutil` caller. Reuses two tricks already established
//!   by earlier extractors for `scraper::ElementRef`'s read-only API: a
//!   post/comment body's URL rewriting re-parses that subtree's own HTML
//!   as a standalone fragment (`WikipediaExtractor::extract`'s clone
//!   trick), and reading a comment's own text when it has no dedicated
//!   body element copies only the kept nodes into a fresh `ego_tree`
//!   fragment (`ChatGptExtractor`'s content-cleaning approach) so a
//!   nested reply's text isn't double-counted into its parent's.
//! - [`YtdlpExtractor`] (capability inventory §4.5.17) — extract *and*
//!   preview for video-hosting pages (YouTube, Vimeo, and others), by
//!   shelling out to the external `yt-dlp` binary rather than parsing
//!   `document.html` at all — the only extractor in this crate that works
//!   entirely from `document.url`. **Disabled by default**, matching Go:
//!   this extractor is useless without `yt-dlp` installed, so opting a
//!   chain into it is a deliberate administrative choice, not automatic.
//!   Three deliberate simplifications from the Go original, documented in
//!   the module doc rather than worked around: no thumbnail download (no
//!   general-purpose HTTP client to reuse — `thumbnail_url` metadata
//!   holds the original URL instead), no per-instance job-slot
//!   concurrency limit or cancellation (no other extractor models
//!   either), and preview renders HTML directly rather than Go's
//!   structured JSON handed to a frontend template (`PreviewResponse` has
//!   no template-hint field). The first extractor to use `rusty_json`'s
//!   `serde` feature (`#[derive(serde::Deserialize)]` on `VideoInfo` and
//!   friends) rather than walking `rusty_json::Value` by hand, since
//!   `yt-dlp --dump-json`'s output is a fixed, known shape.
//!
//! Mastodon/Bluesky/Twitter (capability inventory §4.5.13-15) are not yet
//! portable: their real behavior decomposes one timeline/thread page into
//! *multiple* indexed documents (Go's `Document.ExtraDocuments`/
//! `SkipIndexing`), a capability `rusty-hister-core`'s `Document`/
//! `ExtractOutcome` don't model yet. Flagged in `docs/PROJECT-STATUS.md`
//! as an open item rather than silently dropped or worked around.
//!
//! See `docs/capability-inventory/HISTER-CAPABILITY-INVENTORY.md` §4.3-§4.5
//! for the full list of 20 built-in extractors and their semantically-
//! significant default chain order, and `docs/decisions/
//! ADR-0001-bootstrap-scope-and-crate-split.md` §7 for the licensing
//! policy governing how their tests are written (from independently
//! reading Hister's Go source, never copied).

mod basic;
mod chatgpt;
mod embeddedvideo;
mod github;
mod godoc;
mod hackernews;
mod jsonld;
mod lobsters;
mod reddit;
mod registry;
mod sanitizer;
mod stackexchange;
mod textutil;
mod urlutil;
mod wikipedia;
mod ytdlp;

pub use basic::BasicExtractor;
pub use chatgpt::ChatGptExtractor;
pub use embeddedvideo::EmbeddedVideoExtractor;
pub use github::GitHubExtractor;
pub use godoc::GoDocExtractor;
pub use hackernews::HackerNewsExtractor;
pub use jsonld::JsonLdExtractor;
pub use lobsters::LobstersExtractor;
pub use reddit::RedditExtractor;
pub use registry::Registry;
pub use rusty_hister_core::{
    Capabilities, Document, DocumentType, ExtractOutcome, Extractor, ExtractorConfig, HisterError,
    Metadata, PreviewOutcome, PreviewResponse,
};
pub use sanitizer::{sanitize_html, sanitize_text, sanitize_trusted_html};
pub use stackexchange::StackExchangeExtractor;
pub use wikipedia::WikipediaExtractor;
pub use ytdlp::YtdlpExtractor;
