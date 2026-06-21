//! Rocket League replay analyzer.
//!
//! A scoring-agnostic data layer: it decodes a `.replay` and emits a neutral
//! **canonical match model** (uniform per-frame world state plus coalesced,
//! identity-stable per-player tracks). Scoring is a future *consumer* of this
//! model — none of it lives here.
//!
//! Two layers, separated by the [`decode::ReplayParser`] port:
//! - **decode** — adapters (currently [`decode::boxcars_adapter`]) turn raw
//!   bytes into a neutral [`decode::DecodedReplay`] event stream.
//! - **analyze** — pure reconstruction of world state and player tracks into the
//!   [`model::CanonicalMatch`].
//!
//! # Example
//! ```no_run
//! use replay_analyzer::{decode::{ReplayParser, boxcars_adapter::BoxcarsParser}, analyze};
//! let data = std::fs::read("match.replay").unwrap();
//! let decoded = BoxcarsParser::new().parse(&data).unwrap();
//! let canonical = analyze::build_canonical(&decoded, "match");
//! println!("{} coalesced player tracks", canonical.tracks.len());
//! ```
pub mod analyze;
pub mod cars;
pub mod decode;
pub mod field;
pub mod model;

pub use analyze::build_canonical;
pub use model::CanonicalMatch;
