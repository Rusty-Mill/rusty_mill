//! The unified RLEvalSystem application library.
//!
//! Folds the formerly-disparate executables — analyzer, scoring, skills, value,
//! viewer — into one in-process [`pipeline`], and serves the result through a
//! single-page web [`ui`] over a tiny dependency-free [`server`]. The `rleval`
//! binary is a thin shell over these three modules.

pub mod admin;
pub mod auth;
pub mod authn;
pub mod gzip;
pub mod history;
pub mod jobs;
#[cfg(feature = "oidc")]
pub mod oidc;
#[cfg(feature = "oidc")]
pub mod oidc_transport;
pub mod panels;
pub mod pipeline;
pub mod progress;
pub mod server;
pub mod sha256;
pub mod store;
#[cfg(feature = "mmdb")]
pub mod store_mmdb;
pub mod teams;
pub mod ui;
pub mod uploads;
