//! A [`GameLink`] for the real game: framed `GamePacket`s in, framed `ControllerPacket`s out.
//!
//! [`GameFolder`] keeps the state the game's events describe, as RLBotCS's `GameState` does,
//! and turns it into the `GamePacket` the bots get. [`RocketLeagueLink`] runs it over any
//! `Read + Write` stream (a `TcpStream` from [`crate::GameListener`] in use, a pipe in tests).
//! The wire layouts are in `rusty_bullet/docs/research/RL-GAME-SOCKET-FORMAT.md`.
//!
//! Run against a live Rocket League on 2026-10-11 (see the README and `tests/live_capture.rs`),
//! and checked against buffers built by the generated code of `Bridge.dll`
//! (`tests/golden.rs`) and by this crate's own encoder.

use std::collections::{BTreeMap, BTreeSet};
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
/// Goals kept per map.
const MAX_GOALS: usize = 16;

/// The most bytes RLBot sends the game between two messages from it (`TcpMessenger.Send`).
pub const SEND_BUDGET: usize = 65_535;

#[derive(Debug, Clone)]
struct Car {
    name: String,
    /// Spawned by a command (RLBot's or anyone's), not a human, whose spawn carries id 0.
    is_bot: bool,
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
    identities: Vec<rb_rlbot_wire::PlayerConfiguration>,
    reserved: BTreeSet<u32>,
    slot_count: u32,
    mapping_generation: u64,
    /// Spawn commands sent and not yet answered: command id to the player index it was for.
    expected: BTreeMap<u16, u32>,
    /// Player indices whose car is the answer to a spawn command of ours (not a human's, or one
    /// that was already in the game).
    claimed: BTreeSet<u32>,
    ball: Option<(u16, Physics)>,
    pads: BTreeMap<u16, Pad>,
    goals: Vec<GoalInfo>,
    /// The match has no time limit. The game's scoreboard clock then counts up (Freeplay, seen
    /// live: 1, 2, 3 ...) instead of down, so it is not a time remaining.
    unlimited: bool,
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
            identities: Vec::new(),
            reserved: BTreeSet::new(),
            slot_count: 0,
            mapping_generation: 0,
            expected: BTreeMap::new(),
            claimed: BTreeSet::new(),
            ball: None,
            pads: BTreeMap::new(),
            goals: Vec::new(),
            unlimited: false,
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

    /// Declares the match untimed (from the match configuration, as RLBot does): the packet then
    /// says so and counts upward from the scoreboard clock. Survives a
    /// new map.
    pub fn set_unlimited_time(&mut self, on: bool) {
        self.unlimited = on;
    }

    /// Configured identities and optional reservations for bots the backend will spawn.
    pub fn configure_players(
        &mut self,
        players: &[rb_rlbot_wire::PlayerConfiguration],
        spawning: bool,
    ) {
        self.identities = players.to_vec();
        self.reserved = players
            .iter()
            .enumerate()
            .filter(|(_, p)| spawning && !matches!(p.variety, rb_rlbot_wire::PlayerClass::Human))
            .map(|(i, _)| i as u32)
            .collect();
    }

    /// Changes whenever actor-to-slot mappings change, including map resets.
    pub fn mapping_generation(&self) -> u64 {
        self.mapping_generation
    }

    /// The actor id of the car with player index `index`.
    pub fn actor_for_player(&self, index: u32) -> Option<u16> {
        self.slots.get(&index).copied()
    }

    /// Whether the car at `index` is the answer to a spawn command of ours. A car that was in the
    /// game already (a human's) is at an index too, but is not claimed.
    pub fn is_claimed(&self, index: u32) -> bool {
        self.claimed.contains(&index)
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
            .find(|i| {
                !self.slots.contains_key(i)
                    && !self.reserved.contains(i)
                    && !self.expected.values().any(|e| e == i)
            })
            .unwrap_or(u32::MAX)
    }

    /// Folds one message in. Returns whether it announced a newly loaded map.
    ///
    /// Time follows the bundle: the frame count adds `delta_physics_frames`, seconds elapsed is
    /// frames over 120, pad timers count elapsed time, and the regulation clock runs down between whole-second
    /// scoreboard updates while play is active (as RLBotCS's does).
    pub fn apply(&mut self, bundle: &GameBundle) -> bool {
        let mut map_loaded = false;
        let delta = f32::from(bundle.delta_physics_frames) / PHYSICS_HZ;
        // The wire's frame counter is a `u32`; it wraps after about 414 days at 120 Hz.
        self.frame_num = self
            .frame_num
            .wrapping_add(u32::from(bundle.delta_physics_frames));
        let before_scoreboard = self.scoreboard_seconds;
        for event in &bundle.events {
            map_loaded |= self.event(event);
        }
        if bundle.delta_physics_frames != 0 {
            for pad in self.pads.values_mut().filter(|p| !p.active) {
                pad.timer += delta;
            }
            if self.phase == MatchPhase::Active
                && before_scoreboard == self.scoreboard_seconds
                && !bundle
                    .events
                    .iter()
                    .any(|e| matches!(e, GameEvent::TimeChange { .. }))
            {
                self.time_remaining = if self.unlimited || self.overtime {
                    self.time_remaining + delta
                } else {
                    (self.time_remaining - delta).max(0.0)
                };
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
                    unlimited: self.unlimited,
                    identities: self.identities.clone(),
                    reserved: self.reserved.clone(),
                    mapping_generation: self.mapping_generation.wrapping_add(1),
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
                    pad.timer = 0.0;
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
                    // A human is in the game before any bot is spawned and took the lowest
                    // index; a bot spawned for that index gets it and the human moves along,
                    // as RLBot's index changes do, rather than vanishing from the packet.
                    if promised.is_some() {
                        self.claimed.insert(index);
                    }
                    if let Some(displaced) = self.slots.insert(index, *id) {
                        let moved_to = self.free_index();
                        self.slots.insert(moved_to, displaced);
                    }
                }
                self.slot_count = self
                    .slot_count
                    .max(self.slots.keys().next_back().map_or(0, |i| i + 1));
                self.mapping_generation = self.mapping_generation.wrapping_add(1);
                self.cars.insert(
                    *id,
                    Car {
                        name: name.clone(),
                        is_bot: command_id.is_some_and(|c| c != 0),
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
                    self.mapping_generation = self.mapping_generation.wrapping_add(1);
                    let gone: Vec<u32> = self
                        .slots
                        .iter()
                        .filter(|(_, actor)| *actor == id)
                        .map(|(index, _)| *index)
                        .collect();
                    for index in gone {
                        self.slots.remove(&index);
                        self.claimed.remove(&index);
                    }
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
            } => {
                // A map has two; the cap only stops a game that never stops announcing them.
                if self.goals.len() < MAX_GOALS {
                    self.goals.push(GoalInfo {
                        team_num: i32::from(*team),
                        location: *center,
                        direction: *direction,
                        width: *width,
                        height: *height,
                    });
                }
            }
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
        let players = (0..self.slot_count)
            .map(|index| {
                let car = self
                    .actor_for_player(index)
                    .and_then(|id| self.cars.get(&id));
                let configured = self.identities.get(index as usize);
                PlayerInfo {
                    physics: car.map_or_else(Physics::default, |c| c.physics),
                    air_state: match car.map(|c| c.state) {
                        Some(1) => AirState::Jumping,
                        Some(2) => AirState::DoubleJumping,
                        Some(3) => AirState::Dodging,
                        Some(4) => AirState::InAir,
                        _ => AirState::OnGround,
                    },
                    demolished_timeout: if car.is_none_or(|c| c.state == 5) {
                        0.0
                    } else {
                        -1.0
                    },
                    is_bot: car.map_or_else(
                        || {
                            configured.is_some_and(|p| {
                                !matches!(p.variety, rb_rlbot_wire::PlayerClass::Human)
                            })
                        },
                        |c| c.is_bot,
                    ),
                    name: car.map_or_else(
                        || {
                            configured.map_or_else(String::new, |p| match &p.variety {
                                rb_rlbot_wire::PlayerClass::Human => "Human".into(),
                                rb_rlbot_wire::PlayerClass::CustomBot(b) => b.name.clone(),
                                rb_rlbot_wire::PlayerClass::PsyonixBot(b) => b.name.clone(),
                            })
                        },
                        |c| c.name.clone(),
                    ),
                    team: car.map_or_else(|| configured.map_or(0, |p| p.team), |c| c.team),
                    boost: car.map_or(0.0, |c| c.boost),
                    player_id: configured.map_or(index as i32, |p| p.player_id),
                }
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
                // Divided in f64 so the only rounding is the final one to the wire's f32.
                seconds_elapsed: (f64::from(self.frame_num) / f64::from(PHYSICS_HZ)) as f32,
                game_time_remaining: self.time_remaining,
                is_overtime: self.overtime,
                is_unlimited_time: self.unlimited,
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
    /// Bytes accepted by [`GameLink::send`] that the socket has not taken yet. A non-blocking
    /// socket can take part of a frame; the rest waits here, ahead of every later frame, so the
    /// game's framing never sees a half frame followed by a new one.
    outbox: Vec<u8>,
}

/// The most unsent bytes held for a game that is not reading; past this, sends are refused with
/// `WouldBlock` until it catches up.
const MAX_OUTBOX: usize = 256 * 1024;

impl<S: Read + Write> RocketLeagueLink<S> {
    pub fn new(stream: S) -> RocketLeagueLink<S> {
        RocketLeagueLink {
            stream,
            decoder: FrameDecoder::new(),
            folder: GameFolder::new(),
            // RLBot numbers a command queue from 1.
            next_command_id: 1,
            sent_since_message: 0,
            outbox: Vec::new(),
        }
    }

    /// Bytes accepted for sending that the socket has not taken yet.
    pub fn pending_bytes(&self) -> usize {
        self.outbox.len()
    }

    /// Writes as much of the outbox as the socket takes now; what it will not take stays queued.
    fn flush_outbox(&mut self) -> io::Result<()> {
        while !self.outbox.is_empty() {
            match self.stream.write(&self.outbox) {
                Ok(0) => {
                    return Err(io::Error::new(
                        ErrorKind::WriteZero,
                        "the game's socket took no bytes",
                    ))
                }
                Ok(n) => {
                    self.outbox.drain(..n);
                }
                Err(e) if e.kind() == ErrorKind::WouldBlock => return Ok(()),
                Err(e) if e.kind() == ErrorKind::Interrupted => {}
                Err(e) => return Err(e),
            }
        }
        Ok(())
    }

    pub fn folder(&self) -> &GameFolder {
        &self.folder
    }

    /// See [`GameFolder::set_unlimited_time`].
    pub fn set_unlimited_time(&mut self, on: bool) {
        self.folder.set_unlimited_time(on);
    }

    /// Sets identity metadata and reservations before reading game events.
    pub fn configure_players(
        &mut self,
        players: &[rb_rlbot_wire::PlayerConfiguration],
        spawning: bool,
    ) {
        self.folder.configure_players(players, spawning);
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
        self.flush_outbox()?;
        let mut chunk = [0u8; 8192];
        loop {
            if let Some(clump) = self.buffered_clump()? {
                return Ok(PollClump::Clump(clump));
            }
            match self.stream.read(&mut chunk) {
                Ok(0) => {
                    self.decoder.finish()?;
                    return Ok(PollClump::Closed);
                }
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
        self.flush_outbox()?;
        let mut chunk = [0u8; 8192];
        loop {
            if let Some(clump) = self.buffered_clump()? {
                return Ok(Some(clump));
            }
            match self.stream.read(&mut chunk) {
                Ok(0) => {
                    self.decoder.finish()?;
                    return Ok(None);
                }
                Ok(n) => self.decoder.push(&chunk[..n]),
                Err(e) if e.kind() == ErrorKind::Interrupted => {}
                Err(e) => return Err(e),
            }
        }
    }

    /// Sends one command as its own `ControllerPacket`. A command that would take this message
    /// past [`SEND_BUDGET`] bytes is refused with `WouldBlock` (RLBotCS drops it with a log
    /// line); retry after the next message from the game. A command that cannot fit even an
    /// empty frame is permanently rejected with `InvalidInput`.
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
        if payload.len() > SEND_BUDGET {
            return Err(io::Error::new(
                ErrorKind::InvalidInput,
                "command exceeds the game frame limit",
            ));
        }
        if payload.len() > SEND_BUDGET.saturating_sub(self.sent_since_message) {
            return Err(io::Error::new(
                ErrorKind::WouldBlock,
                "over the per-frame byte budget; try after the next message",
            ));
        }
        let bytes = frame(&payload).map_err(invalid)?;
        // Anything still queued goes first. If the game is not taking bytes, refuse before
        // queueing more: the command is then not sent, and its id and any spawn promise stay free.
        self.flush_outbox()?;
        if self.outbox.len() + bytes.len() > MAX_OUTBOX {
            return Err(io::Error::new(
                ErrorKind::WouldBlock,
                "the game is not reading; too many bytes are waiting to be sent",
            ));
        }
        self.outbox.extend_from_slice(&bytes);
        self.flush_outbox()?;
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

#[cfg(test)]
#[allow(clippy::unwrap_used)]
mod tests {
    use super::*;

    fn bundle(delta: u8, events: Vec<GameEvent>) -> GameBundle {
        GameBundle {
            app_seconds: 0.0,
            delta_physics_frames: delta,
            events,
        }
    }

    #[test]
    fn the_frame_counter_wraps_instead_of_overflowing() {
        let mut folder = GameFolder::new();
        folder.frame_num = u32::MAX - 2;
        folder.apply(&bundle(5, vec![]));
        assert_eq!(folder.packet().match_info.frame_num, 2);
        folder.apply(&bundle(1, vec![]));
        assert_eq!(folder.packet().match_info.frame_num, 3);
    }

    #[test]
    fn seconds_are_the_nearest_f32_to_frames_over_120_even_late_in_a_long_run() {
        let mut folder = GameFolder::new();
        // 40 hours in, a frame count an f32 cannot hold exactly (above 2^24).
        folder.frame_num = 17_280_001;
        let seconds = folder.packet().match_info.seconds_elapsed;
        assert_eq!(seconds, (17_280_001_f64 / 120.0) as f32);
        // The old f32 division is a different number.
        assert_ne!(seconds, 17_280_001_f32 / 120.0);
    }

    #[test]
    fn a_game_that_keeps_announcing_goals_does_not_grow_the_list_without_bound() {
        let mut folder = GameFolder::new();
        let goal = |id| GameEvent::GoalSpawn {
            id,
            team: (id % 2) as u8,
            center: Vec3 {
                x: 0.0,
                y: 5120.0,
                z: 321.0,
            },
            direction: Vec3 {
                x: 0.0,
                y: -1.0,
                z: 0.0,
            },
            width: 1786.0,
            height: 642.0,
        };
        let events = (0..1000).map(goal).collect();
        folder.apply(&bundle(1, events));
        assert_eq!(folder.field_info().goals.len(), MAX_GOALS);
        // A new map starts the list afresh.
        folder.apply(&bundle(
            1,
            vec![GameEvent::GameSpawn {
                map_name: "next".into(),
                mode: 1,
                team_size: 1,
                gravity: None,
            }],
        ));
        assert!(folder.field_info().goals.is_empty());
    }
}
