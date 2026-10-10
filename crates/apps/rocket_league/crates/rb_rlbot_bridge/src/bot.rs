//! A policy as an RLBot bot.

use rb_domain::{ControllerInput, PhysicsFrame};
use rb_rlbot_client::{run_bots, Agent, BotInit, Connection, Outbox, Result};
use rb_rlbot_wire::{ConnectionSettings, GamePacket, PlayerInput};

use crate::convert::{controller_state, observation};

/// Chooses a car's input from what `rb_env` observes: the same call whether the frame came from
/// a simulation (`Env::step`) or from the game (`observation`). Any `FnMut(&PhysicsFrame, usize)
/// -> ControllerInput` is a policy.
pub trait Policy {
    /// The input for car `car` (its index in `frame.cars`) at this frame.
    fn act(&mut self, frame: &PhysicsFrame, car: usize) -> ControllerInput;
}

impl<F: FnMut(&PhysicsFrame, usize) -> ControllerInput> Policy for F {
    fn act(&mut self, frame: &PhysicsFrame, car: usize) -> ControllerInput {
        self(frame, car)
    }
}

/// One car driven by a [`Policy`].
pub struct PolicyBot<P> {
    policy: P,
    car: u32,
}

impl<P: Policy> PolicyBot<P> {
    /// A bot for the car at `car` in the game's player list.
    pub fn new(policy: P, car: u32) -> PolicyBot<P> {
        PolicyBot { policy, car }
    }

    /// The input to send for `packet`, or `None` if there is nothing to act on: no ball, or the
    /// car is not in the packet.
    pub fn input(&mut self, packet: &GamePacket) -> Option<PlayerInput> {
        let car = self.car as usize;
        let frame = observation(packet)?;
        frame.cars.get(car)?;
        Some(PlayerInput {
            player_index: self.car,
            controller_state: controller_state(&self.policy.act(&frame, car)),
        })
    }
}

impl<P: Policy> Agent for PolicyBot<P> {
    fn tick(&mut self, packet: &GamePacket, out: &mut Outbox) {
        if let Some(input) = self.input(packet) {
            out.push(input);
        }
    }
}

/// Connects as a bot and plays `make(init)` for every car core gives this connection. Returns
/// when core disconnects the client; see [`run_bots`].
pub fn run_policy<P: Policy>(
    connection: &mut Connection,
    settings: ConnectionSettings,
    mut make: impl FnMut(BotInit<'_>) -> P,
) -> Result<()> {
    run_bots(connection, settings, |init, _| {
        let car = init.controllable.index;
        PolicyBot::new(make(init), car)
    })
}
