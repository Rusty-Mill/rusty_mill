//! Shared types, IDs, and error types for the `rusty_hister` crate cluster —
//! a Rust port of [asciimoo/hister](https://github.com/asciimoo/hister).
//!
//! **Bootstrap stage — no implementation yet.** This crate exists so the
//! workspace member list and crate-split decision are recorded and buildable
//! before any port logic lands. See:
//!
//! - `docs/PROJECT-STATUS.md` (this cluster's root) for current status.
//! - `docs/capability-inventory/HISTER-CAPABILITY-INVENTORY.md` for the full
//!   capability manifest this crate's eventual `Document`, `Metadata`, and
//!   error types must satisfy (particularly §4.1's extractor SDK contract and
//!   §7's model layer, which both depend on the shared `Document` shape
//!   defined here).
//! - `docs/decisions/ADR-0001-bootstrap-scope-and-crate-split.md` for why the
//!   cluster is split this way.
