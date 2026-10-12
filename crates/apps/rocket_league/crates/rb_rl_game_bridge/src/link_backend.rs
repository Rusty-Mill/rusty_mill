//! The real game as `rb_rlbot_core`'s match: [`LinkBackend`].
//!
//! Bots connect to `rb_rlbot_core::Core` as they do to RLBot; `Core` asks this backend for
//! packets and hands it their input. The backend reads the game with
//! [`RocketLeagueLink`](crate::RocketLeagueLink), applies RLBotCS's distribution rules
//! ([`BridgeLoop`]), and sends inputs and state setting back. It is **self-paced**: the game
//! decides when a packet exists, so core's pacing does not apply.
//!
//! With [`LinkBackend::with_spawning`] the backend spawns the configuration's bots itself, as
//! RLBot does: once the map has been up for two messages it sends a `SpawnBot` per bot, and the
//! match starts when every car has answered. Without it the cars must already exist in the
//! game. Either way a bot's car is the one with its player index (see `GameFolder`).

use std::io::{self, ErrorKind, Read, Write};

use rb_rlbot_core::{MatchBackend, Poll};
use rb_rlbot_wire::{
    ControllerState, DesiredGameState, DesiredPhysics, FieldInfo, GamePacket, MatchConfiguration,
    MatchLengthMutator, PartialVec3, Physics, PlayerClass, PsyonixSkill, Rotator, Vec3,
};

use crate::game_wire::{BotSkill, Loadout};
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
    spawning: bool,
    /// A map has loaded since the backend started.
    map_seen: bool,
    /// How many of the configuration's players have had their spawn command sent for this map.
    spawn_sent: usize,
}

/// A player's spawn parameters, if it is a bot: name, skill, team. Humans are not spawned.
fn bot_of(player: &rb_rlbot_wire::PlayerConfiguration) -> Option<(String, BotSkill, u8)> {
    let team = u8::try_from(player.team).ok()?;
    match &player.variety {
        PlayerClass::CustomBot(bot) => Some((bot.name.clone(), BotSkill::Custom, team)),
        // RLBot's skill names against the game's (`MatchStarter.cs:459-463`).
        PlayerClass::PsyonixBot(bot) => Some((
            bot.name.clone(),
            match bot.skill {
                PsyonixSkill::Beginner => BotSkill::Intro,
                PsyonixSkill::Rookie => BotSkill::Easy,
                PsyonixSkill::Pro => BotSkill::Medium,
                PsyonixSkill::AllStar => BotSkill::Hard,
            },
            team,
        )),
        PlayerClass::Human => None,
    }
}

impl<S: Read + Write> LinkBackend<S> {
    /// `stream` must be non-blocking (`TcpStream::set_nonblocking(true)`). `configuration` names
    /// the players: car `i` is `player_configurations[i]`.
    pub fn new(stream: S, configuration: MatchConfiguration) -> LinkBackend<S> {
        let mut bridge = BridgeLoop::new();
        bridge.set_roster(Some(Roster::from_configuration(&configuration)));
        let mut link = RocketLeagueLink::new(stream);
        // The scoreboard clock counts up in an untimed match, so only the configuration can say
        // whether it is a time remaining.
        link.set_unlimited_time(
            configuration
                .mutators
                .is_some_and(|m| m.match_length == MatchLengthMutator::Unlimited),
        );
        LinkBackend {
            link,
            bridge,
            configuration,
            latest: None,
            spawning: false,
            map_seen: false,
            spawn_sent: 0,
        }
    }

    /// Whether to spawn the configuration's bots once the map is up (default off). Loadouts are
    /// not modelled in the configuration, so every bot gets [`Loadout::standard`].
    pub fn with_spawning(mut self, on: bool) -> LinkBackend<S> {
        self.spawning = on;
        self
    }

    /// Sends the spawn commands that are due. Called every poll; it resumes where a refused
    /// command (the per-frame byte budget) left off.
    fn spawn_due(&mut self) -> io::Result<()> {
        // Two messages after the map loads, as RLBot waits (`BridgeHandler.cs:175-184`). It
        // also waits for the game to be paused there, which a LAN match never reports.
        if !self.spawning || !self.map_seen || self.bridge.tracker().ticks_since_map_load() < 2 {
            return Ok(());
        }
        while let Some(player) = self
            .configuration
            .player_configurations
            .get(self.spawn_sent)
        {
            let player_index = self.spawn_sent as u32;
            if let Some((name, skill, team)) = bot_of(player) {
                let spawn = GameCommand::SpawnBot {
                    player_index,
                    name,
                    skill,
                    team,
                    loadout: Loadout::standard(team),
                };
                match self.link.send(spawn) {
                    Ok(()) => {}
                    Err(e) if e.kind() == ErrorKind::WouldBlock => return Ok(()),
                    Err(e) => return Err(e),
                }
            }
            self.spawn_sent += 1;
        }
        Ok(())
    }

    /// Whether every bot in the configuration has the car its spawn command made. A car at the
    /// bot's index is not enough: a human who was already in the game may hold it.
    fn bots_spawned(&self) -> bool {
        self.configuration
            .player_configurations
            .iter()
            .enumerate()
            .filter(|(_, p)| bot_of(p).is_some())
            .all(|(i, _)| self.link.folder().is_claimed(i as u32))
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

    /// Ready once a packet has been distributed (so bots never see an empty match) and, when
    /// spawning, every bot's car exists.
    fn ready(&self) -> bool {
        self.latest.is_some() && (!self.spawning || self.bots_spawned())
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
        self.spawn_due()?;
        // A packet is distributed on its own turn so no bot misses one: stop at the first.
        for _ in 0..MAX_CLUMPS_PER_POLL {
            match self.link.poll_clump()? {
                PollClump::Idle => return Ok(Poll::Idle),
                PollClump::Closed => return Ok(Poll::Closed),
                PollClump::Clump(clump) => {
                    let step = self.bridge.on_clump(clump);
                    if step.map_loaded {
                        // A new map has no cars: spawn the bots again.
                        self.map_seen = true;
                        self.spawn_sent = 0;
                    }
                    if let Some(packet) = step.distribute {
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
