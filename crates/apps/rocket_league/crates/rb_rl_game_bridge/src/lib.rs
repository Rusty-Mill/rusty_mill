//! The game-facing half of RLBot core, for rusty_bullet. See
//! `rusty_bullet/docs/research/RLBOT-BRIDGE-DLL.md` for how `Bridge.dll` fits between RLBot
//! and Rocket League and the evidence for each rule below.
//!
//! RLBot core talks to bots over its own socket (that half is `rb_rlbot_wire`,
//! `rb_rlbot_client` and `rb_rlbot_core`) and to the game through `Bridge.dll`. This crate
//! rebuilds the parts of the second that are visible in RLBotCS's open source:
//!
//! - [`launch`]: the arguments that make Rocket League dial out (`-rlbot
//!   RLBot_ControllerURL=127.0.0.1:<port> RLBot_PacketSendRate=240 -nomovie`) and a [`GameLaunch`]
//!   that starts the executable with them.
//! - [`ports`]: which controller port to use (a running game's, else the first free from
//!   23233, skipping 23234) and whether a game is already running in RLBot mode.
//! - [`GameListener`]: the socket the game connects to.
//! - [`tick`] and [`BridgeLoop`]: how each message from the game is folded into time, and
//!   whether its snapshot is handed to the bots.
//! - [`GameLink`]: the seam to the game, with [`SimGameLink`] (the game is `rb_env`) and
//!   [`RocketLeagueLink`] (the real game's socket) as implementations; [`LinkBackend`] serves a
//!   real game to `rb_rlbot_core`.
//! - [`decode_game_packet`] and friends: Rocket League's `-rlbot` socket format, recovered from
//!   `Bridge.dll` under ADR-0083 and documented in `RL-GAME-SOCKET-FORMAT.md`.
//!
//! Run against a live Rocket League (Epic, Freeplay) on 2026-10-11; see the README for what that
//! covered and what it did not.

mod game_wire;
mod launch;
mod link;
mod link_backend;
mod ports;
mod rl_link;
mod sim_link;
mod tick;

pub use game_wire::{
    decode_controller_packet, decode_game_packet, encode_controller_packet, encode_game_packet,
    paint, product, BotSkill, Command, GameBundle, GameEvent, Loadout, Rgba, VehicleInput,
    WireError, LOADOUT_SLOTS,
};
pub use launch::{
    rlbot_args, GameLaunch, DEFAULT_GAME_PORT, IDEAL_GAME_PORT, PACKET_SEND_RATE,
    RLBOT_SOCKETS_PORT,
};
pub use link::{BridgeLoop, GameCommand, GameLink, GameListener, Step};
pub use link_backend::LinkBackend;
pub use ports::{
    any_started_with_rlbot, controller_port, find_usable_game_port, tcp_port_is_free,
    ProcessSource, SystemProcesses,
};
pub use rl_link::{vehicle_input, GameFolder, PollClump, RocketLeagueLink, SEND_BUDGET};
pub use sim_link::SimGameLink;
pub use tick::{
    should_distribute, Clump, Roster, TickTracker, MAX_TICK_SKIP, TIME_ADVANCE_EPSILON,
};
