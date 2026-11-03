//! The lens runner's pure core (Let Em Loose #594): critic lenses on
//! different models, working one shared board of claims.
//!
//! No I/O. The `rusty_orch lens` shell reads the board, asks providers
//! through the [`Model`] port and posts what comes back; everything it
//! decides is decided here.
//!
//! - [`port`]: the [`Model`] port, one stateless question per call.
//! - [`post`]: the strict reply protocol every adapter applies ([`parse`]).
//! - [`config`]: providers, data classes, ceilings and lenses, checked at load.
//! - [`card`] and [`context`]: a fresh, fitted context per job, blind first.
//! - [`class`]: data classes; nothing above a provider's clearance reaches it.
//! - [`tier`]: routine or contested, and the cleared providers in order.
//! - [`meter`]: integer micro-dollars, reserved before every call.

#![cfg_attr(
    not(test),
    deny(clippy::unwrap_used, clippy::expect_used, clippy::panic)
)]

pub mod card;
pub mod class;
pub mod config;
pub mod context;
pub mod meter;
pub mod port;
pub mod post;
pub mod tier;

pub use class::DataClass;
pub use config::LensConfig;
pub use port::{Answer, Model, Question, Usage};
pub use post::{parse, Allowed, Post};
