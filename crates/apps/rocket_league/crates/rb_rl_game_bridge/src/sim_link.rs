//! A game that is `rb_env`: the [`GameLink`] used wherever Rocket League is not.

use std::io;

use rb_rlbot_core::Simulation;

use crate::link::{GameCommand, GameLink};
use crate::tick::Clump;

/// Steps a [`Simulation`] once per message and reports it, like a game that renders as fast as
/// it is read. The first message announces the map load, as a real game's does.
pub struct SimGameLink {
    sim: Simulation,
    messages: u64,
    limit: Option<u64>,
}

impl SimGameLink {
    /// A game that never ends.
    pub fn new(sim: Simulation) -> SimGameLink {
        SimGameLink {
            sim,
            messages: 0,
            limit: None,
        }
    }

    /// A game that goes away after `messages` messages.
    pub fn ending_after(sim: Simulation, messages: u64) -> SimGameLink {
        SimGameLink {
            limit: Some(messages),
            ..SimGameLink::new(sim)
        }
    }

    pub fn simulation(&self) -> &Simulation {
        &self.sim
    }
}

impl GameLink for SimGameLink {
    fn next_clump(&mut self) -> io::Result<Option<Clump>> {
        if self.limit.is_some_and(|limit| self.messages >= limit) {
            return Ok(None);
        }
        let map_just_loaded = self.messages == 0;
        if !map_just_loaded {
            self.sim.step();
        }
        self.messages += 1;
        Ok(Some(Clump {
            packet: self.sim.packet(),
            map_just_loaded,
        }))
    }

    fn send(&mut self, command: GameCommand) -> io::Result<()> {
        match command {
            GameCommand::PlayerInput {
                player_index,
                state,
            } => {
                if self.sim.set_input(player_index, &state) {
                    Ok(())
                } else {
                    Err(io::Error::new(
                        io::ErrorKind::InvalidInput,
                        format!("no car {player_index}"),
                    ))
                }
            }
            // These have nothing to act on in the simulation (state setting goes through
            // `Simulation::apply_desired`); refusing says so instead of pretending they ran.
            other => Err(io::Error::new(
                io::ErrorKind::Unsupported,
                format!("the simulated game cannot take {other:?}"),
            )),
        }
    }
}
