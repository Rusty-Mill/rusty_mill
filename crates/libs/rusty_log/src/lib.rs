//! The workspace's single door to `tracing`.
//!
//! Workspace crates log through this crate (`rusty_log::info!`,
//! `rusty_log::info_span!`, `rusty_log::Instrument`, ...) instead of depending
//! on `tracing` themselves. Today it is a pass-through: every item below is the
//! `tracing` item, so moving a crate onto it changes no behaviour and no log
//! line. What it buys is one place that owns the third-party dependency, so the
//! dependency can be pinned, audited or swapped without touching every crate.
//! See `docs/research/TRACING-FACADE-PLAN.md`.
//!
//! # Subscribers
//!
//! With the `subscriber` feature, [`tracing_subscriber`] is re-exported whole
//! (`EnvFilter`, `fmt`, `Layer`, `Registry`, `layer::SubscriberExt`, ...). It
//! keeps its own name because `tracing` already has a `subscriber` module of
//! its own, which this crate passes through unchanged.
//!
//! # `#[instrument]`
//!
//! The attribute macro expands to paths that name the `tracing` crate
//! directly, so a crate using it must still depend on `tracing`. Replace such
//! uses with a span and [`Instrument`] when migrating a crate onto this one.

pub use tracing::*;

#[cfg(feature = "subscriber")]
pub use tracing_subscriber;
