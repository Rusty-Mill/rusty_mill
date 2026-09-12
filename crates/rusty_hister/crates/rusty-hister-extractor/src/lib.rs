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
//! **No concrete extractors yet.** See
//! `docs/capability-inventory/HISTER-CAPABILITY-INVENTORY.md` §4.3-§4.5
//! for the 20 extractors and their semantically-significant default chain
//! order, and `docs/decisions/ADR-0001-bootstrap-scope-and-crate-split.md`
//! §7 for the licensing policy governing how their tests are written (from
//! independently reading Hister's Go source, never copied).

mod registry;

pub use registry::Registry;
pub use rusty_hister_core::{
    Capabilities, Document, DocumentType, ExtractOutcome, Extractor, ExtractorConfig, HisterError,
    Metadata, PreviewOutcome, PreviewResponse,
};
