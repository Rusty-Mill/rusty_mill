//! What [`Core`](crate::Core) plays a match on: the thing behind the bots.
//!
//! RLBot core serves bots on one side and drives a game on the other. [`MatchBackend`] is the
//! second side as the server needs it. [`Simulation`] is one (`rb_env`, stepped by core at the
//! pace it is told); a real Rocket League behind `rb_rl_game_bridge` is another (it produces
//! packets when the game does, so core does not pace it).

use std::io;

use rb_rlbot_wire::{
    ControllableTeamInfo, ControllerState, DesiredGameState, FieldInfo, GamePacket,
    MatchConfiguration,
};

use crate::sim::{controllable_team_info, expected_agents, Simulation};

/// What a self-paced backend has for the server right now.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Poll {
    /// A new [`GamePacket`] is ready: [`MatchBackend::packet`] returns it.
    Packet,
    /// Nothing new yet.
    Idle,
    /// The game has gone away; the match is over.
    Closed,
}

/// The match behind a [`Core`](crate::Core).
pub trait MatchBackend {
    /// Who is in the match; the agents bots connect as come from here.
    fn configuration(&self) -> &MatchConfiguration;

    /// The agent ids the match waits for before it starts.
    fn expected_agents(&self) -> Vec<String> {
        expected_agents(self.configuration())
    }

    /// The cars an agent controls (car index = player index).
    fn controllable_team_info(&self, agent_id: &str) -> ControllableTeamInfo {
        controllable_team_info(self.configuration(), agent_id)
    }

    fn field_info(&self) -> FieldInfo;

    /// Whether the backend has a match to show (a game that has connected and sent its first
    /// packet). The server does not start the match before this, and a self-paced backend that
    /// stops being ready after the start (a new map whose cars are not back yet) is not
    /// broadcast to the bots until it is ready again.
    fn ready(&self) -> bool {
        true
    }

    /// `true` if packets come when the backend says ([`MatchBackend::poll`]); `false` if core
    /// decides when to [`MatchBackend::step`] according to its pacing.
    fn self_paced(&self) -> bool {
        false
    }

    /// Whether the match has reached its end and nothing more will happen (a simulated timed
    /// match that has ended). Core stops serving then. A real game never says so: it rolls into
    /// the next match, and only it closing its connection ends the run.
    fn finished(&self) -> bool {
        false
    }

    /// Advances one physics tick. Only called when not [`self_paced`](Self::self_paced).
    fn step(&mut self) -> io::Result<()>;

    /// Looks for the next packet without blocking. Only called when self-paced.
    fn poll(&mut self) -> io::Result<Poll> {
        Ok(Poll::Idle)
    }

    /// Holds `state` as the input of car `player_index`. `Ok(false)` if there is no such car.
    fn set_input(&mut self, player_index: u32, state: &ControllerState) -> io::Result<bool>;

    /// Applies a state-setting message. `Ok(true)` if it named a car that does not exist.
    /// Nonfinite or out-of-range values (see `validation::MAX_LOCATION` and friends) return `InvalidInput` before any state change or command is sent.
    fn apply_desired(&mut self, desired: &DesiredGameState) -> io::Result<bool>;

    /// The match as it stands, in the shape core sends every tick.
    fn packet(&self) -> GamePacket;
}

impl MatchBackend for Simulation {
    fn configuration(&self) -> &MatchConfiguration {
        Simulation::configuration(self)
    }

    fn field_info(&self) -> FieldInfo {
        Simulation::field_info(self)
    }

    fn finished(&self) -> bool {
        Simulation::is_over(self)
    }

    fn step(&mut self) -> io::Result<()> {
        Simulation::step(self);
        Ok(())
    }

    fn set_input(&mut self, player_index: u32, state: &ControllerState) -> io::Result<bool> {
        Ok(Simulation::set_input(self, player_index, state))
    }

    fn apply_desired(&mut self, desired: &DesiredGameState) -> io::Result<bool> {
        Simulation::apply_desired(self, desired)
    }

    fn packet(&self) -> GamePacket {
        Simulation::packet(self)
    }
}
