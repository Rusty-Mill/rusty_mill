//! The real game as `rb_rlbot_core`'s match: [`LinkBackend`].
//!
//! Bots connect to `rb_rlbot_core::Core` as they do to RLBot; `Core` asks this backend for
//! packets and hands it their input. The backend reads the game with
//! [`RocketLeagueLink`](crate::RocketLeagueLink), applies RLBotCS's distribution rules
//! ([`BridgeLoop`]), and sends inputs and state setting back. It is **self-paced**: the game
//! decides when a packet exists, so core's pacing does not apply.
//!
//! The cars must already exist in the game: there is no `SpawnBot` yet. A bot's car is the car
//! with its player index, which is spawn order (see `GameFolder`).

use std::io::{self, ErrorKind, Read, Write};

use rb_rlbot_core::{MatchBackend, Poll};
use rb_rlbot_wire::{
    ControllerState, DesiredGameState, DesiredPhysics, FieldInfo, GamePacket, MatchConfiguration,
    PartialVec3, Physics, Rotator, Vec3,
};

use crate::link::{BridgeLoop, GameCommand, GameLink};
use crate::rl_link::{PollClump, RocketLeagueLink};
use crate::tick::Roster;

/// Messages read per [`MatchBackend::poll`] before giving the server a turn, so a flood from the
/// game cannot starve the bots' sockets.
const MAX_CLUMPS_PER_POLL: usize = 256;

/// A game behind a non-blocking stream, serving `rb_rlbot_core`.
pub struct LinkBackend<S> {
    link: RocketLeagueLink<S>,
    bridge: BridgeLoop,
    configuration: MatchConfiguration,
    latest: Option<GamePacket>,
}

impl<S: Read + Write> LinkBackend<S> {
    /// `stream` must be non-blocking (`TcpStream::set_nonblocking(true)`). `configuration` names
    /// the players: car `i` is `player_configurations[i]`.
    pub fn new(stream: S, configuration: MatchConfiguration) -> LinkBackend<S> {
        let mut bridge = BridgeLoop::new();
        bridge.set_roster(Some(Roster::from_configuration(&configuration)));
        LinkBackend {
            link: RocketLeagueLink::new(stream),
            bridge,
            configuration,
            latest: None,
        }
    }

    pub fn link(&self) -> &RocketLeagueLink<S> {
        &self.link
    }
}

/// `want` laid over `current`: only the named fields change.
fn patch(want: &DesiredPhysics, current: Physics) -> Physics {
    let vec = |w: &Option<PartialVec3>, old: Vec3| match w {
        Some(w) => Vec3 {
            x: w.x.unwrap_or(old.x),
            y: w.y.unwrap_or(old.y),
            z: w.z.unwrap_or(old.z),
        },
        None => old,
    };
    Physics {
        location: vec(&want.location, current.location),
        rotation: match &want.rotation {
            Some(r) => Rotator {
                pitch: r.pitch.unwrap_or(current.rotation.pitch),
                yaw: r.yaw.unwrap_or(current.rotation.yaw),
                roll: r.roll.unwrap_or(current.rotation.roll),
            },
            None => current.rotation,
        },
        velocity: vec(&want.velocity, current.velocity),
        angular_velocity: vec(&want.angular_velocity, current.angular_velocity),
    }
}

impl<S: Read + Write> MatchBackend for LinkBackend<S> {
    fn configuration(&self) -> &MatchConfiguration {
        &self.configuration
    }

    fn field_info(&self) -> FieldInfo {
        self.link.folder().field_info()
    }

    /// Ready once a packet has been distributed, so bots never see an empty match.
    fn ready(&self) -> bool {
        self.latest.is_some()
    }

    fn self_paced(&self) -> bool {
        true
    }

    fn step(&mut self) -> io::Result<()> {
        Err(io::Error::new(
            ErrorKind::Unsupported,
            "a game steps itself; core must poll it",
        ))
    }

    fn poll(&mut self) -> io::Result<Poll> {
        // A packet is distributed on its own turn so no bot misses one: stop at the first.
        for _ in 0..MAX_CLUMPS_PER_POLL {
            match self.link.poll_clump()? {
                PollClump::Idle => return Ok(Poll::Idle),
                PollClump::Closed => return Ok(Poll::Closed),
                PollClump::Clump(clump) => {
                    if let Some(packet) = self.bridge.on_clump(clump).distribute {
                        self.latest = Some(packet);
                        return Ok(Poll::Packet);
                    }
                }
            }
        }
        Ok(Poll::Idle)
    }

    fn set_input(&mut self, player_index: u32, state: &ControllerState) -> io::Result<bool> {
        match self.link.send(GameCommand::PlayerInput {
            player_index,
            state: *state,
        }) {
            Ok(()) => Ok(true),
            Err(e) if e.kind() == ErrorKind::InvalidInput => Ok(false),
            Err(e) => Err(e),
        }
    }

    fn apply_desired(&mut self, desired: &DesiredGameState) -> io::Result<bool> {
        if let Some(ball) = desired.ball_states.first() {
            if let Some((_, current)) = self.link.folder().ball() {
                self.link.send(GameCommand::SetBallPhysics {
                    physics: patch(&ball.physics, current),
                })?;
            }
        }
        let mut unknown_car = false;
        for (index, want) in desired.car_states.iter().enumerate() {
            let player_index = index as u32;
            let Some(current) = self.link.folder().car_physics(player_index) else {
                unknown_car = true;
                continue;
            };
            if let Some(physics) = &want.physics {
                self.link.send(GameCommand::SetCarPhysics {
                    player_index,
                    physics: patch(physics, current),
                })?;
            }
            if let Some(boost) = want.boost_amount {
                self.link.send(GameCommand::SetBoost {
                    player_index,
                    amount: boost.clamp(0.0, 100.0) as u8,
                })?;
            }
        }
        Ok(unknown_car)
    }

    fn packet(&self) -> GamePacket {
        self.latest
            .clone()
            .unwrap_or_else(|| self.link.folder().packet())
    }
}
