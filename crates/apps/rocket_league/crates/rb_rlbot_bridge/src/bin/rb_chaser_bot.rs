//! A runnable RLBot v5 bot that plays `rb_env`'s scripted chaser: connects to core, takes every
//! car it is given, and drives each at the ball. Started by RLBot core from `chaser.bot.toml`
//! (core sets `RLBOT_SERVER_PORT` and `RLBOT_AGENT_ID`; `RLBOT_SERVER_ADDR` overrides the host and port), or by hand for a
//! match somebody else started.
//!
//! Usage: `rb_chaser_bot` (no arguments).

use std::error::Error;

use rb_rlbot_bridge::{chaser, run_policy};
use rb_rlbot_client::{Connection, Environment};
use rb_rlbot_wire::ConnectionSettings;

const DEFAULT_AGENT_ID: &str = "rusty_bullet/chaser_bot";

fn main() -> Result<(), Box<dyn Error>> {
    let Environment {
        server_addr,
        agent_id,
    } = Environment::from_env();
    let agent_id = agent_id.unwrap_or_else(|| DEFAULT_AGENT_ID.into());
    let mut connection = Connection::connect(&server_addr)?;
    let settings = ConnectionSettings {
        agent_id: agent_id.clone(),
        wants_ball_predictions: false,
        wants_comms: false,
        close_between_matches: true,
    };
    run_policy(&mut connection, settings, |init| {
        println!(
            "chaser bot `{agent_id}`: car {} on team {}",
            init.controllable.index, init.team
        );
        chaser(init.team as usize)
    })?;
    println!("chaser bot `{agent_id}` exited");
    Ok(())
}
