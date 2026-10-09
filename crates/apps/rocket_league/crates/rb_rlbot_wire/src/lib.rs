//! The RLBot v5 socket protocol, hand-written on `rusty_flatbuffers`.
//!
//! A connection to RLBot core carries framed messages ([`frame`]): clients send
//! [`InterfaceMessage`]s and receive [`CoreMessage`]s. Both directions are encodable and
//! decodable so a stand-in core can be built from the same types.
//!
//! The schema is large (`rlbot_flat` is 46,000 generated lines); this models the part
//! a bot, a script and a match runner use, field by field, and says what it leaves out on
//! each type. Slot layouts live in [`tables`] and are pinned by buffers made by the reference
//! implementation (`tests/golden.rs`). Schema revision pinned: `c38374e`.

mod codec;
mod error;
pub mod frame;
mod message;
mod tables;
mod types;

pub use codec::Struct;
pub use error::Error;
pub use message::{CoreMessage, InterfaceMessage};
pub use tables::{ConnectionSettings, InitComplete, PlayerInput, StopCommand};
pub use types::*;

/// The schema revision the layouts were taken from.
pub const SCHEMA_REV: &str = "c38374e";
