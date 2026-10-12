//! A [`GameLink`] for the real game: framed `GamePacket`s in, framed `ControllerPacket`s out.
//!
//! [`GameFolder`] keeps the state the game's events describe, as RLBotCS's `GameState` does,
//! and turns it into the `GamePacket` the bots get. [`RocketLeagueLink`] runs it over any
//! `Read + Write` stream (a `TcpStream` from [`crate::GameListener`] in use, a pipe in tests).
//! The wire layouts are in `rusty_bullet/docs/research/RL-GAME-SOCKET-FORMAT.md`.
//!
//! **Never run against a live game yet.** Everything here is checked against buffers built by
//! the generated code of `Bridge.dll` (see `tests/golden.rs`) and by this crate's own encoder.

use std::collections::BTreeMap;
use std::io::{self, ErrorKind, Read, Write};

use rb_rlbot_wire::frame::{frame, FrameDecoder};
use rb_rlbot_wire::{
    AirState, BallInfo, BoostPad, BoostPadState, ControllerState, FieldInfo, GamePacket, GoalInfo,
    MatchInfo, MatchPhase, Physics, PlayerInfo, TeamInfo, Vec3,
};

use crate::game_wire::{
    decode_game_packet, encode_controller_packet, Command, GameBundle, GameEvent, VehicleInput,
};
use crate::link::{GameCommand, GameLink};
use crate::tick::Clump;

/// Physics frames per second; the game's `delta_physics_frames` counts these.
const PHYSICS_HZ: f32 = 120.0;

/// The most bytes RLBot sends the game between two messages from it (`TcpMessenger.Send`).
pub const SEND_BUDGET: usize = 65_535;

#[derive(Debug, Clone)]
struct Car {
    name: String,
    team: u32,
    physics: Physics,
    state: u8,
    boost: f32,
}

#[derive(Debug, Clone, Copy)]
struct Pad {
    location: Vec3,
    big: bool,
    active: bool,
    timer: f32,
}

/// The match as the game's events describe it.
#[derive(Debug, Clone)]
pub struct GameFolder {
    frame_num: u32,
    phase: MatchPhase,
    overtime: bool,
    time_remaining: f32,
    scoreboard_seconds: Option<i16>,
    scores: [u32; 2],
    map_name: String,
    cars: BTreeMap<u16, Car>,
    /// Which actor is the car of each player index.
    slots: BTreeMap<u32, u16>,
    /// Spawn commands sent and not yet answered: command id to the player index it was for.
    expected: BTreeMap<u16, u32>,
    ball: Option<(u16, Physics)>,
    pads: BTreeMap<u16, Pad>,
    goals: Vec<GoalInfo>,
}

impl Default for GameFolder {
    fn default() -> GameFolder {
        GameFolder {
            frame_num: 0,
            phase: MatchPhase::Inactive,
            overtime: false,
            time_remaining: 0.0,
            scoreboard_seconds: None,
            scores: [0; 2],
            map_name: String::new(),
            cars: BTreeMap::new(),
            slots: BTreeMap::new(),
            expected: BTreeMap::new(),
            ball: None,
            pads: BTreeMap::new(),
            goals: Vec::new(),
        }
    }
}

impl GameFolder {
    pub fn new() -> GameFolder {
        GameFolder::default()
    }

    pub fn map_name(&self) -> &str {
        &self.map_name
    }

    /// The actor id of the car with player index `index`.
    pub fn actor_for_player(&self, index: u32) -> Option<u16> {
        self.slots.get(&index).copied()
    }

    /// Records that spawn command `command_id` is for player `player_index`: the car the game
    /// answers it with takes that index, whatever order cars appear in. (A car with no such
    /// command, such as a human's, takes the lowest index nobody has claimed.)
    pub fn expect_spawn(&mut self, command_id: u16, player_index: u32) {
        self.expected.insert(command_id, player_index);
    }

    /// The lowest player index that is neither taken nor promised to a pending spawn.
    fn free_index(&self) -> u32 {
        (0..)
            .find(|i| !self.slots.contains_key(i) && !self.expected.values().any(|e| e == i))
            .unwrap_or(u32::MAX)
    }

    /// Folds one message in. Returns whether it announced a newly loaded map.
    ///
    /// Time follows the bundle: the frame count adds `delta_physics_frames`, seconds elapsed is
    /// frames over 120, pad timers run down, and the match clock runs down between whole-second
    /// scoreboard updates while play is active (as RLBotCS's does).
    pub fn apply(&mut self, bundle: &GameBundle) -> bool {
        let mut map_loaded = false;
        let delta = f32::from(bundle.delta_physics_frames) / PHYSICS_HZ;
        self.frame_num += u32::from(bundle.delta_physics_frames);
        let before_scoreboard = self.scoreboard_seconds;
        for event in &bundle.events {
            map_loaded |= self.event(event);
        }
        if bundle.delta_physics_frames != 0 {
            for pad in self.pads.values_mut().filter(|p| !p.active) {
                pad.timer -= delta;
            }
            if self.phase == MatchPhase::Active && before_scoreboard == self.scoreboard_seconds {
                self.time_remaining = (self.time_remaining - delta).max(0.0);
            }
        }
        map_loaded
    }

    fn event(&mut self, event: &GameEvent) -> bool {
        match event {
            GameEvent::GameSpawn { map_name, .. } => {
                // A new map: everything from the last one is gone.
                *self = GameFolder {
                    frame_num: self.frame_num,
                    map_name: map_name.clone(),
                    ..GameFolder::default()
                };
                return true;
            }
            GameEvent::StateChange { phase, is_overtime } => {
                self.phase = *phase;
                self.overtime = *is_overtime;
            }
            GameEvent::TimeChange { display_seconds } => {
                self.scoreboard_seconds = Some(*display_seconds);
                self.time_remaining = f32::from(*display_seconds);
            }
            GameEvent::PickupSpawn { id, location, big } => {
                self.pads.insert(
                    *id,
                    Pad {
                        location: *location,
                        big: *big,
                        active: true,
                        timer: 0.0,
                    },
                );
            }
            GameEvent::PickupState { id, picked_up } => {
                if let Some(pad) = self.pads.get_mut(id) {
                    pad.active = !picked_up;
                    pad.timer = if *picked_up {
                        if pad.big {
                            10.0
                        } else {
                            4.0
                        }
                    } else {
                        0.0
                    };
                }
            }
            GameEvent::PlayerSpawn {
                id,
                name,
                team,
                command_id,
            } => {
                if !self.slots.values().any(|a| a == id) {
                    // Command id 0 is the game's own spawns (a human), never a bot we asked for.
                    let promised = command_id
                        .filter(|c| *c != 0)
                        .and_then(|c| self.expected.remove(&c));
                    let index = promised.unwrap_or_else(|| self.free_index());
                    self.slots.insert(index, *id);
                }
                self.cars.insert(
                    *id,
                    Car {
                        name: name.clone(),
                        team: u32::from(*team),
                        physics: Physics::default(),
                        state: 0,
                        boost: 0.0,
                    },
                );
            }
            GameEvent::PlayerPhysics { id, physics, state } => {
                if let Some(car) = self.cars.get_mut(id) {
                    car.physics = *physics;
                    car.state = *state;
                }
            }
            GameEvent::PlayerBoost { id, amount, .. } => {
                if let Some(car) = self.cars.get_mut(id) {
                    car.boost = f32::from(*amount);
                }
            }
            GameEvent::BallSpawn { id } => self.ball = Some((*id, Physics::default())),
            GameEvent::BallPhysics { id, physics } => {
                if let Some((ball_id, ball)) = &mut self.ball {
                    if ball_id == id {
                        *ball = *physics;
                    }
                }
            }
            GameEvent::TeamScore { team, score } => {
                if let Some(slot) = self.scores.get_mut(usize::from(*team)) {
                    *slot = u32::from(*score);
                }
            }
            GameEvent::ActorDestroyed { id } => {
                if self.cars.remove(id).is_some() {
                    self.slots.retain(|_, actor| actor != id);
                }
                if self.ball.is_some_and(|(ball_id, _)| ball_id == *id) {
                    self.ball = None;
                }
            }
            GameEvent::GoalSpawn {
                team,
                center,
                direction,
                width,
                height,
                ..
            } => self.goals.push(GoalInfo {
                team_num: i32::from(*team),
                location: *center,
                direction: *direction,
                width: *width,
                height: *height,
            }),
            GameEvent::Other(_) => {}
        }
        false
    }

    /// The ball's actor id and physics, once it has spawned.
    pub fn ball(&self) -> Option<(u16, Physics)> {
        self.ball
    }

    /// The physics of the car with player index `index`.
    pub fn car_physics(&self, index: u32) -> Option<Physics> {
        let id = self.actor_for_player(index)?;
        self.cars.get(&id).map(|car| car.physics)
    }

    /// The static field as the game announced it: pads by y then x, goals by team.
    pub fn field_info(&self) -> FieldInfo {
        let mut pads: Vec<&Pad> = self.pads.values().collect();
        pads.sort_by(|a, b| {
            a.location
                .y
                .total_cmp(&b.location.y)
                .then(a.location.x.total_cmp(&b.location.x))
        });
        let mut goals = self.goals.clone();
        goals.sort_by_key(|g| g.team_num);
        FieldInfo {
            boost_pads: pads
                .into_iter()
                .map(|p| BoostPad {
                    location: p.location,
                    is_full_boost: p.big,
                })
                .collect(),
            goals,
        }
    }

    /// The snapshot the bots get. Cars are by player index; pads are by y, then x, as in
    /// `RLBotCS/Conversion/GameStateToFlat.cs`.
    pub fn packet(&self) -> GamePacket {
        let players = self
            .slots
            .iter()
            .filter_map(|(index, id)| self.cars.get(id).map(|car| (*index, car)))
            .map(|(index, car)| PlayerInfo {
                physics: car.physics,
                air_state: match car.state {
                    1 => AirState::Jumping,
                    2 => AirState::DoubleJumping,
                    3 => AirState::Dodging,
                    4 => AirState::InAir,
                    _ => AirState::OnGround,
                },
                // The game says only that a car is demolished, not for how long.
                demolished_timeout: if car.state == 5 { 0.0 } else { -1.0 },
                is_bot: true,
                name: car.name.clone(),
                team: car.team,
                boost: car.boost,
                player_id: i32::try_from(index).unwrap_or(i32::MAX),
            })
            .collect();
        let mut pads: Vec<&Pad> = self.pads.values().collect();
        pads.sort_by(|a, b| {
            a.location
                .y
                .total_cmp(&b.location.y)
                .then(a.location.x.total_cmp(&b.location.x))
        });
        GamePacket {
            players,
            boost_pads: pads
                .into_iter()
                .map(|p| BoostPadState {
                    is_active: p.active,
                    timer: p.timer.max(0.0),
                })
                .collect(),
            balls: self
                .ball
                .iter()
                .map(|(_, physics)| BallInfo { physics: *physics })
                .collect(),
            match_info: MatchInfo {
                seconds_elapsed: self.frame_num as f32 / PHYSICS_HZ,
                game_time_remaining: self.time_remaining,
                is_overtime: self.overtime,
                is_unlimited_time: false,
                match_phase: self.phase,
                frame_num: self.frame_num,
            },
            teams: self
                .scores
                .iter()
                .enumerate()
                .map(|(team, score)| TeamInfo {
                    team_index: team as u32,
                    score: *score,
                })
                .collect(),
        }
    }
}

/// A connection to a real game over any byte stream.
pub struct RocketLeagueLink<S> {
    stream: S,
    decoder: FrameDecoder,
    folder: GameFolder,
    next_command_id: u16,
    sent_since_message: usize,
}

impl<S: Read + Write> RocketLeagueLink<S> {
    pub fn new(stream: S) -> RocketLeagueLink<S> {
        RocketLeagueLink {
            stream,
            decoder: FrameDecoder::new(),
            folder: GameFolder::new(),
            // RLBot numbers a command queue from 1.
            next_command_id: 1,
            sent_since_message: 0,
        }
    }

    pub fn folder(&self) -> &GameFolder {
        &self.folder
    }

    fn actor(&self, player_index: u32) -> io::Result<u16> {
        self.folder.actor_for_player(player_index).ok_or_else(|| {
            io::Error::new(ErrorKind::InvalidInput, format!("no car {player_index}"))
        })
    }

    /// The stream, giving up the link.
    pub fn into_inner(self) -> S {
        self.stream
    }
}

fn invalid(e: impl std::error::Error + Send + Sync + 'static) -> io::Error {
    io::Error::new(ErrorKind::InvalidData, e)
}

/// What a non-blocking look at the game's stream found.
#[derive(Debug, Clone, PartialEq)]
pub enum PollClump {
    Clump(Clump),
    /// No complete message yet.
    Idle,
    /// The game closed the socket.
    Closed,
}

impl<S: Read + Write> RocketLeagueLink<S> {
    /// The next complete message already buffered, folded in.
    fn buffered_clump(&mut self) -> io::Result<Option<Clump>> {
        let Some(payload) = self.decoder.next_frame() else {
            return Ok(None);
        };
        let bundle = decode_game_packet(&payload).map_err(invalid)?;
        // Any message from the game means a frame was rendered: the byte budget resets.
        self.sent_since_message = 0;
        let map_just_loaded = self.folder.apply(&bundle);
        Ok(Some(Clump {
            packet: self.folder.packet(),
            map_just_loaded,
        }))
    }

    /// Like [`GameLink::next_clump`] for a stream set non-blocking: reads what has arrived and
    /// returns at once when there is no complete message.
    pub fn poll_clump(&mut self) -> io::Result<PollClump> {
        let mut chunk = [0u8; 8192];
        loop {
            if let Some(clump) = self.buffered_clump()? {
                return Ok(PollClump::Clump(clump));
            }
            match self.stream.read(&mut chunk) {
                Ok(0) => return Ok(PollClump::Closed),
                Ok(n) => self.decoder.push(&chunk[..n]),
                Err(e) if e.kind() == ErrorKind::WouldBlock => return Ok(PollClump::Idle),
                Err(e) if e.kind() == ErrorKind::Interrupted => {}
                Err(e) => return Err(e),
            }
        }
    }
}

impl<S: Read + Write> GameLink for RocketLeagueLink<S> {
    fn next_clump(&mut self) -> io::Result<Option<Clump>> {
        let mut chunk = [0u8; 8192];
        loop {
            if let Some(clump) = self.buffered_clump()? {
                return Ok(Some(clump));
            }
            match self.stream.read(&mut chunk) {
                Ok(0) => return Ok(None),
                Ok(n) => self.decoder.push(&chunk[..n]),
                Err(e) if e.kind() == ErrorKind::Interrupted => {}
                Err(e) => return Err(e),
            }
        }
    }

    /// Sends one command as its own `ControllerPacket`. A command that would take this message
    /// past [`SEND_BUDGET`] bytes is refused with `WouldBlock` (RLBotCS drops it with a log
    /// line); retry after the next message from the game.
    fn send(&mut self, command: GameCommand) -> io::Result<()> {
        let mut spawned_for = None;
        let command = match command {
            GameCommand::SpawnBot {
                player_index,
                name,
                skill,
                team,
                loadout,
            } => {
                spawned_for = Some(player_index);
                Command::SpawnBot {
                    name,
                    skill,
                    team,
                    loadout,
                }
            }
            GameCommand::PlayerInput {
                player_index,
                state,
            } => Command::PlayerInput {
                actor_id: self.actor(player_index)?,
                input: VehicleInput::from_controller(&state),
            },
            GameCommand::Console(text) => Command::Exec(text),
            GameCommand::SetCarPhysics {
                player_index,
                physics,
            } => Command::SetPhysics {
                actor_id: self.actor(player_index)?,
                physics,
            },
            GameCommand::SetBallPhysics { physics } => Command::SetPhysics {
                actor_id: self
                    .folder
                    .ball()
                    .map(|(id, _)| id)
                    .ok_or_else(|| io::Error::new(ErrorKind::InvalidInput, "no ball yet"))?,
                physics,
            },
            GameCommand::SetBoost {
                player_index,
                amount,
            } => Command::SetBoost {
                actor_id: self.actor(player_index)?,
                amount,
            },
        };
        let command_id = self.next_command_id;
        let payload = encode_controller_packet(command_id, &[command]).map_err(invalid)?;
        if payload.len() > SEND_BUDGET.saturating_sub(self.sent_since_message) {
            return Err(io::Error::new(
                ErrorKind::WouldBlock,
                "over the per-frame byte budget; try after the next message",
            ));
        }
        let bytes = frame(&payload).map_err(invalid)?;
        self.stream.write_all(&bytes)?;
        self.sent_since_message += payload.len();
        self.next_command_id = self.next_command_id.wrapping_add(1).max(1);
        // Only once it is on the wire: a refused spawn must not leave a promise behind.
        if let Some(player_index) = spawned_for {
            self.folder.expect_spawn(command_id, player_index);
        }
        Ok(())
    }
}

/// A controller state as the game takes it; exposed for callers that build [`Command`]s.
pub fn vehicle_input(state: &ControllerState) -> VehicleInput {
    VehicleInput::from_controller(state)
}
