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
//! **Eight concrete extractors so far:**
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

mod chatgpt;
mod embeddedvideo;
mod github;
mod godoc;
mod hackernews;
mod jsonld;
mod lobsters;
mod registry;
mod sanitizer;
mod stackexchange;
mod textutil;
mod urlutil;

pub use chatgpt::ChatGptExtractor;
pub use embeddedvideo::EmbeddedVideoExtractor;
pub use github::GitHubExtractor;
pub use godoc::GoDocExtractor;
pub use hackernews::HackerNewsExtractor;
pub use jsonld::JsonLdExtractor;
pub use lobsters::LobstersExtractor;
pub use registry::Registry;
pub use rusty_hister_core::{
    Capabilities, Document, DocumentType, ExtractOutcome, Extractor, ExtractorConfig, HisterError,
    Metadata, PreviewOutcome, PreviewResponse,
};
pub use sanitizer::{sanitize_html, sanitize_text, sanitize_trusted_html};
pub use stackexchange::StackExchangeExtractor;
