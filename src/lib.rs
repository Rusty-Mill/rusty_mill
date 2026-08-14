//! Pure-Rust, from-scratch SQLite reimplementation aiming for `rusqlite`
//! API parity. See `ARCHITECTURE.md` for the engine/API boundary and
//! `gap-analysis.md` for what's tracked toward that parity target.

mod connection;
mod error;

pub use connection::Connection;
pub use error::{Error, Result};
