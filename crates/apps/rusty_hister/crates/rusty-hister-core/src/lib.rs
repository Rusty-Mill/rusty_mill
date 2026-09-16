//! Shared types, IDs, and error types for the `rusty_hister` crate cluster —
//! a Rust port of [asciimoo/hister](https://github.com/asciimoo/hister).
//!
//! This crate holds the contract other `rusty-hister-*` crates build
//! against: the `Document` working type extractors operate on, the
//! `Extractor` trait itself (capability inventory §4.1), and the shared
//! error type. It does not hold the extractor *registry* or any concrete
//! extractor — those live in `rusty-hister-extractor`, which depends on
//! this crate rather than the other way around.
//!
//! See:
//! - `docs/PROJECT-STATUS.md` (this cluster's root) for current status.
//! - `docs/capability-inventory/HISTER-CAPABILITY-INVENTORY.md` for the
//!   full capability manifest, in particular §4.1 (extractor SDK contract)
//!   and §5.3/§7 (the fields this crate's `Document` type must eventually
//!   round-trip through the query layer and the model layer).
//! - `docs/decisions/ADR-0001-bootstrap-scope-and-crate-split.md` for why
//!   the cluster is split this way.

mod document;
mod error;
mod extractor;

pub use document::{Document, DocumentType, Metadata};
pub use error::HisterError;
pub use extractor::{
    Capabilities, ExtractOutcome, Extractor, ExtractorConfig, PreviewOutcome, PreviewResponse,
};
