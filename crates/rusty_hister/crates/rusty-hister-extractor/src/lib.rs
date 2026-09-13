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
//! **Three concrete extractors so far:**
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
//!
//! See `docs/capability-inventory/HISTER-CAPABILITY-INVENTORY.md` §4.3-§4.5
//! for the full list of 20 built-in extractors and their semantically-
//! significant default chain order, and `docs/decisions/
//! ADR-0001-bootstrap-scope-and-crate-split.md` §7 for the licensing
//! policy governing how their tests are written (from independently
//! reading Hister's Go source, never copied).

mod embeddedvideo;
mod jsonld;
mod registry;
mod sanitizer;
mod stackexchange;
mod urlutil;

pub use embeddedvideo::EmbeddedVideoExtractor;
pub use jsonld::JsonLdExtractor;
pub use registry::Registry;
pub use rusty_hister_core::{
    Capabilities, Document, DocumentType, ExtractOutcome, Extractor, ExtractorConfig, HisterError,
    Metadata, PreviewOutcome, PreviewResponse,
};
pub use sanitizer::{sanitize_html, sanitize_text, sanitize_trusted_html};
pub use stackexchange::StackExchangeExtractor;
