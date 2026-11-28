//! `lel_treegen`: the seeded synthetic dedup test tree for Let Em Loose card
//! #1319 (design `docs/projects/let-em-loose/1319-synthetic-tree-design.md`,
//! D240–D243, D246).
//!
//! This crate is the pure core: [`plan`] turns `(seed, scale, profile)` into
//! every entry and every expected group with no I/O, and [`manifest`] renders
//! that as canonical JSON Lines whose sha256 names the tree. Writing the tree
//! comes in a later slice.
//!
//! ```
//! use lel_treegen::{Profile, manifest, plan::plan};
//! let profile = Profile::default();
//! let tree = plan(42, 0.0001, &profile)?;
//! let bytes = manifest::render(&tree, &profile)?;
//! assert_eq!(manifest::digest(&bytes).len(), 64);
//! # Ok::<(), lel_treegen::ProfileError>(())
//! ```

pub mod manifest;
pub mod plan;
pub mod profile;
pub mod rng;

pub use profile::{Profile, ProfileError};
