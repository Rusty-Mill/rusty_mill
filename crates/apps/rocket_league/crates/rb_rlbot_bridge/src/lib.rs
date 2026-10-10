//! `rb_env` as an RLBot bot: the stage 5 bridge of the native RLBot port (`../../rlbot/PLAN.md`).
//!
//! `rb_env` and everything built on it speak [`rb_domain::PhysicsFrame`] in and
//! [`rb_domain::ControllerInput`] out. This crate is the two conversions that put the game on
//! the same footing, and a [`Policy`] bot on top of `rb_rlbot_client`, so a policy written
//! against a simulation plays in the real game unchanged:
//!
//! - [`observation`]: a `GamePacket` as the `PhysicsFrame` `Env::reset` takes.
//! - [`controller_state`]: a `ControllerInput` as RLBot's `ControllerState`.
//! - [`PolicyBot`] / [`run_policy`]: one policy per car, one input per packet.
//!
//! A policy decides from the frame alone (the team and car index are arguments), so it carries
//! no RLBot types. What it cannot see is what a `PhysicsFrame` does not hold: boost pad timers,
//! the match clock and score.

mod bot;
mod convert;

pub use bot::{run_policy, Policy, PolicyBot};
pub use convert::{controller_state, observation};
