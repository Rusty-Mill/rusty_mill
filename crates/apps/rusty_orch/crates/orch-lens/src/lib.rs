//! The lens runner's pure core (Let Em Loose #594): critic lenses on
//! different models, working one shared board of claims.
//!
//! No I/O. The `rusty_orch lens` shell reads the board, asks providers
//! through the [`Model`] port and posts what comes back; everything it
//! decides is decided here.
//!
//! - [`alien`]: the #708 alien review lens: trigger, suspects, ranking, probes.
//! - [`hunt`]: one alien hunt over the model port and a probe runner.
//! - [`round`]: one runner round: schedule, call, post, record.
//! - [`call`]: one job over its fallback order, metered before each ask.
//! - [`port`]: the [`Model`] port, one stateless question per call.
//! - [`post`]: the strict reply protocol every adapter applies ([`check`]).
//! - [`config`]: providers, data classes, ceilings and lenses, checked at load.
//! - [`card`] and [`context`]: a fresh, fitted context per job, blind first.
//! - [`class`]: data classes; nothing above a provider's clearance reaches it.
//! - [`tier`]: routine or contested, and the cleared providers in order.
//! - [`meter`]: integer micro-dollars, reserved before every call.
//! - [`breaker`]: pause a task whose spend rises while its claims stay flat.
//! - [`schedule`]: which jobs run now; the blind-first, look-harder,
//!   proposer, rebel and fairness rules as checks.
//! - [`record`]: a lens's track record, one informational line.
//! - [`taint`]: what a lens saw marks what it posts; never lowered.
//! - [`saved`]: the scheduler's memory as plain data, for the store.

#![cfg_attr(
    not(test),
    deny(clippy::unwrap_used, clippy::expect_used, clippy::panic)
)]

pub mod alien;
pub mod breaker;
pub mod call;
pub mod card;
pub mod class;
pub mod config;
pub mod context;
pub mod hunt;
pub mod meter;
pub mod port;
pub mod post;
pub mod record;
pub mod round;
pub mod saved;
pub mod schedule;
pub mod taint;
pub mod tier;

pub use class::DataClass;
pub use config::LensConfig;
pub use port::{Answer, Model, Question, Reply, Shape, Usage};
pub use post::{check, parse, Allowed, Post};
