//! The data-product registry HTTP API -- the Rust port of
//! `meshed.registry` (data products/ports/contracts CRUD, access
//! grants, governance/lineage/metrics endpoints, the monitor topology +
//! SSE event feed, and the transformation-maturity endpoints).
//!
//! See `../../capability-manifest.md` (rows REG-001..139, XFM-001..040)
//! for the capabilities this crate covers; most of the HTTP surface is
//! still open, see the crate's module list below for what's
//! implemented so far.

pub mod models;
pub mod transformation;
