//! A simulated RLBot v5 core: the half of RLBot that sits between bots and the game, with
//! `rb_env`'s physics standing in for Rocket League.
//!
//! Real RLBot core has two faces. Towards bots it is a TCP server on port 23234 speaking
//! `rb_rlbot_wire`; towards the game it uses `Bridge.dll` over a second socket to read state
//! and send inputs (see `rusty_bullet/docs/research/RLBOT-BRIDGE-DLL.md`). This crate keeps the bot-facing face
//! byte-for-byte and replaces the game-facing one with [`Simulation`], so any RLBot v5 bot or
//! script, including `rb_tape_bot` and `rb_chaser_bot`, can play a match with no game installed.
//!
//! - [`Simulation`] is the match with no sockets: setup from a `MatchConfiguration`, held inputs,
//!   state setting, and the `GamePacket` / `FieldInfo` core publishes. Deterministic.
//! - [`Core`] is the socket server around it: handshake, input routing by agent id, packet
//!   broadcast, stop. [`Pacing::Lockstep`] waits for every bot to answer each packet, which
//!   makes a run repeatable whatever the bots' speed; [`Pacing::RealTime`] paces at 120 Hz.
//!
//! Not modelled: ball prediction, match comms, rendering, quick chat, loadouts, starting a
//! new match over the socket, and every mutator except match length. A client that asks for
//! them gets nothing (or an [`Event::Ignored`]), never a crash.

mod backend;
mod server;
mod sim;
pub mod validation;

pub use backend::{MatchBackend, Poll};
pub use server::{Core, Event, Pacing, RunEnd};
pub use sim::{bot_match, controllable_team_info, expected_agents, SetupError, Simulation};
