//! A blocking RLBot v5 client on `std::net`.
//!
//! [`Connection`] speaks the framed protocol of [`rb_rlbot_wire`]: send
//! [`InterfaceMessage`]s (start a match, set state, stop), receive
//! [`CoreMessage`]s, with an optional timeout. On top of it, [`run_bots`] and [`run_hivemind`]
//! run the handshake and the packet loop for [`Agent`]s; pings are answered for you.
//!
//! One thread, no async: a bot answers one packet at a time, and the loop is I/O-bound on a
//! single local socket. Stage 3 of the native RLBot port (`../../rlbot/PLAN.md`).

mod agent;
mod connection;
mod env;
mod error;

pub use agent::{run_bots, run_hivemind, Agent, BotInit, Outbox};
pub use connection::{Connection, StartingInfo};
pub use env::Environment;
pub use error::{Error, Result};
pub use rb_rlbot_wire::{CoreMessage, InterfaceMessage};
