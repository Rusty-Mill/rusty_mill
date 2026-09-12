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
//! **One concrete extractor so far: [`JsonLdExtractor`]** (capability
//! inventory §4.5.5) — enrich-only, parses `application/ld+json` script
//! tags. See its own module doc for why it hand-rolls its narrow
//! HTML-scanning and text-sanitization needs rather than pulling in a
//! general HTML-parsing/sanitizer dependency; most of the other 19
//! built-in extractors will need one or both eventually, a bigger
//! cross-cutting choice deferred until it's actually needed. See
//! `docs/capability-inventory/HISTER-CAPABILITY-INVENTORY.md` §4.3-§4.5
//! for the full list and their semantically-significant default chain
//! order, and `docs/decisions/ADR-0001-bootstrap-scope-and-crate-split.md`
//! §7 for the licensing policy governing how their tests are written (from
//! independently reading Hister's Go source, never copied).

mod jsonld;
mod registry;

pub use jsonld::JsonLdExtractor;
pub use registry::Registry;
pub use rusty_hister_core::{
    Capabilities, Document, DocumentType, ExtractOutcome, Extractor, ExtractorConfig, HisterError,
    Metadata, PreviewOutcome, PreviewResponse,
};
