//! The unified RLEvalSystem application library.
//!
//! Folds the formerly-disparate executables — analyzer, scoring, skills, value,
//! viewer — into one in-process [`pipeline`], and serves the result through a
//! single-page web [`ui`] over a tiny dependency-free [`server`]. The `rleval`
//! binary is a thin shell over these three modules.

pub mod pipeline;
pub mod server;
pub mod ui;
