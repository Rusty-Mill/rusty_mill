//! Rocket League's `-rlbot` socket payloads, hand-written on `rusty_flatbuffers`.
//!
//! The layouts are in `rusty_bullet/docs/research/RL-GAME-SOCKET-FORMAT.md` (ADR-0083). Each
//! message on the socket is a `u16` big-endian length plus one buffer, framed by
//! `rb_rlbot_wire::frame`; this module is the buffers. The game sends a `GamePacket` (a bundle
//! of events); RLBot sends a `ControllerPacket` (a bundle of commands). Both directions are
//! encodable and decodable so a stand-in game can be built from the same types.
//!
//! Only what a bridge needs is modelled; the rest decodes as [`GameEvent::Other`].

use std::fmt;

use rb_rlbot_wire::{ControllerState, MatchPhase, Physics, Struct, Vec3};
use rusty_flatbuffers::{Builder, Offset, Table, TableBuilder};

/// Why a payload could not be read or built.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum WireError {
    Flat(rusty_flatbuffers::Error),
    /// A field the format always sends is absent.
    Missing(&'static str),
    /// An enum byte outside the schema.
    UnknownEnum {
        name: &'static str,
        value: u8,
    },
    /// A message asks for more than any real one holds (see the `MAX_` limits).
    TooLarge(&'static str),
}

impl fmt::Display for WireError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            WireError::Flat(e) => write!(f, "{e}"),
            WireError::Missing(what) => write!(f, "missing field {what}"),
            WireError::UnknownEnum { name, value } => write!(f, "{value} is not a {name}"),
            WireError::TooLarge(what) => write!(f, "message holds too many {what}"),
        }
    }
}

impl std::error::Error for WireError {}

impl From<rusty_flatbuffers::Error> for WireError {
    fn from(e: rusty_flatbuffers::Error) -> WireError {
        WireError::Flat(e)
    }
}

pub type Result<T> = std::result::Result<T, WireError>;

/// The most events one message may carry. The real opening message of a map holds about 50.
pub const MAX_EVENTS_PER_BUNDLE: usize = 4096;

/// The most string bytes (map and player names) one message may carry in total. A crafted buffer
/// can point many slots at one long string, so this bounds what decoding allocates.
pub const MAX_STRING_BYTES_PER_BUNDLE: usize = 64 * 1024;

// ---- union tags (numbered from 0 in schema order; see the format note) ----

mod game_tag {
    pub const GAME_SPAWN: u8 = 1;
    pub const GAME_STATE_CHANGE: u8 = 2;
    pub const GAME_TIME_CHANGE: u8 = 3;
    pub const GOAL_SPAWN: u8 = 4;
    pub const PICKUP_SPAWN: u8 = 5;
    pub const PICKUP_STATE_CHANGE: u8 = 6;
    pub const PLAYER_SPAWN: u8 = 7;
    pub const PLAYER_PHYSICS_CHANGE: u8 = 8;
    pub const PLAYER_BOOST_CHANGE: u8 = 9;
    pub const BALL_SPAWN: u8 = 10;
    pub const BALL_PHYSICS_CHANGE: u8 = 11;
    pub const TEAM_SCORE_CHANGE: u8 = 13;
    pub const ACTOR_DESTROYED: u8 = 14;
}

mod command_tag {
    pub const SPAWN_BOT: u8 = 1;
    pub const DESTROY_ACTOR: u8 = 2;
    pub const PLAYER_INPUT: u8 = 3;
    pub const EXEC: u8 = 8;
    pub const SET_PHYSICS: u8 = 9;
    pub const SET_BOOST: u8 = 10;
    pub const END_MATCH: u8 = 12;
    pub const SET_PAUSED: u8 = 13;
}

/// What the game tells us, one event at a time.
#[derive(Debug, Clone, PartialEq)]
pub enum GameEvent {
    /// A map has loaded. `mode` is the game's own numbering (Unknown, Soccar, Hoops, ...).
    GameSpawn {
        map_name: String,
        mode: u8,
        team_size: u8,
        gravity: Option<Vec3>,
    },
    StateChange {
        phase: MatchPhase,
        is_overtime: bool,
    },
    /// The scoreboard clock, whole seconds.
    TimeChange {
        display_seconds: i16,
    },
    GoalSpawn {
        id: u16,
        team: u8,
        center: Vec3,
        direction: Vec3,
        width: f32,
        height: f32,
    },
    PickupSpawn {
        id: u16,
        location: Vec3,
        big: bool,
    },
    PickupState {
        id: u16,
        picked_up: bool,
    },
    PlayerSpawn {
        id: u16,
        command_id: Option<u16>,
        name: String,
        team: u8,
    },
    PlayerPhysics {
        id: u16,
        physics: Physics,
        /// The game's `PlayerPhysicsState`: OnGround, Jumped, DoubleJumped, Dodged, InAir,
        /// Demolished.
        state: u8,
    },
    PlayerBoost {
        id: u16,
        is_active: bool,
        amount: u8,
    },
    BallSpawn {
        id: u16,
    },
    BallPhysics {
        id: u16,
        physics: Physics,
    },
    TeamScore {
        team: u8,
        score: u8,
    },
    ActorDestroyed {
        id: u16,
    },
    /// An event this module does not decode, by its union tag.
    Other(u8),
}

/// One message from the game: a bundle of events and how much time they cover.
#[derive(Debug, Clone, PartialEq)]
pub struct GameBundle {
    pub app_seconds: f32,
    /// How many 120 Hz physics frames the bundle spans; 0 for a bundle that did not advance time.
    pub delta_physics_frames: u8,
    pub events: Vec<GameEvent>,
}

/// A bot's stick and buttons as the game takes them (`VehicleInput`).
#[derive(Debug, Clone, Copy, PartialEq, Default)]
pub struct VehicleInput {
    pub throttle: f32,
    pub steer: f32,
    pub pitch: f32,
    pub yaw: f32,
    pub roll: f32,
    pub dodge_forward: f32,
    pub dodge_strafe: f32,
    pub jump: bool,
    pub boost: bool,
    pub handbrake: bool,
    pub use_item: bool,
}

impl VehicleInput {
    /// RLBot's controller state as the game's input. RLBot derives the dodge direction from
    /// the sticks: forward is minus pitch, and strafe is yaw, which is what makes a stall
    /// work (`RLBotCS/Conversion/FlatToModel.cs:19-22`).
    pub fn from_controller(state: &ControllerState) -> VehicleInput {
        VehicleInput {
            throttle: state.throttle,
            steer: state.steer,
            pitch: state.pitch,
            yaw: state.yaw,
            roll: state.roll,
            dodge_forward: -state.pitch,
            dodge_strafe: state.yaw,
            jump: state.jump,
            boost: state.boost,
            handbrake: state.handbrake,
            use_item: state.use_item,
        }
    }
}

/// A colour as a loadout takes it.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Rgba {
    pub r: u8,
    pub g: u8,
    pub b: u8,
    pub a: u8,
}

/// How a spawned bot plays: the game's `BotSkill`. `Custom` is a bot an outside program drives.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
#[repr(u8)]
pub enum BotSkill {
    Intro = 0,
    Easy = 1,
    Medium = 2,
    Hard = 3,
    Custom = 4,
}

impl BotSkill {
    fn from_u8(value: u8) -> Result<BotSkill> {
        Ok(match value {
            0 => BotSkill::Intro,
            1 => BotSkill::Easy,
            2 => BotSkill::Medium,
            3 => BotSkill::Hard,
            4 => BotSkill::Custom,
            value => {
                return Err(WireError::UnknownEnum {
                    name: "BotSkill",
                    value,
                })
            }
        })
    }
}

/// How many entries the game's loadout vectors carry.
pub const LOADOUT_SLOTS: usize = 11;

/// The item slots of a bot's loadout, in the game's order: car, decal, wheels, boost, antenna,
/// hat, paint finish, custom finish, engine audio, trails, goal explosion.
pub mod product {
    pub const CAR: usize = 0;
    pub const DECAL: usize = 1;
    pub const WHEELS: usize = 2;
    pub const BOOST: usize = 3;
    pub const ANTENNA: usize = 4;
    pub const HAT: usize = 5;
    pub const PAINT_FINISH: usize = 6;
    pub const CUSTOM_FINISH: usize = 7;
    pub const ENGINE_AUDIO: usize = 8;
    pub const TRAILS: usize = 9;
    pub const GOAL_EXPLOSION: usize = 10;
}

/// The paint slots, same length as the products: car, decal, wheels, boost, antenna and hat
/// paint, three slots RLBot always sends as 0 (6 to 8), trails and goal explosion paint.
pub mod paint {
    pub const TRAILS: usize = 9;
    pub const GOAL_EXPLOSION: usize = 10;
}

/// A bot's look: item ids per slot, paint ids per slot, and two colours.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Loadout {
    pub products: [u16; LOADOUT_SLOTS],
    pub paints: [u8; LOADOUT_SLOTS],
    pub primary: Rgba,
    pub secondary: Rgba,
}

impl Loadout {
    /// An Octane (car id 23) with no cosmetics. The colours are plain team colours picked here,
    /// not looked up from the game's swatches: **unverified in the game**.
    pub fn standard(team: u8) -> Loadout {
        let mut products = [0u16; LOADOUT_SLOTS];
        products[product::CAR] = 23;
        let (r, g, b) = if team == 0 {
            (57, 80, 229)
        } else {
            (229, 111, 57)
        };
        Loadout {
            products,
            paints: [0; LOADOUT_SLOTS],
            primary: Rgba { r, g, b, a: 255 },
            secondary: Rgba {
                r: 255,
                g: 255,
                b: 255,
                a: 255,
            },
        }
    }
}

/// What we tell the game.
#[derive(Debug, Clone, PartialEq)]
pub enum Command {
    /// Spawns a bot's car. The game announces it with a `PlayerSpawn` carrying this command's
    /// id, which is how it is tied to the player it was spawned for.
    SpawnBot {
        name: String,
        skill: BotSkill,
        /// 0 blue, 1 orange.
        team: u8,
        loadout: Loadout,
    },
    PlayerInput {
        actor_id: u16,
        input: VehicleInput,
    },
    /// A console command.
    Exec(String),
    SetPaused(bool),
    EndMatch,
    DestroyActor(u16),
    SetPhysics {
        actor_id: u16,
        physics: Physics,
    },
    /// Boost 0 to 100.
    SetBoost {
        actor_id: u16,
        amount: u8,
    },
}

// ---- reading ----

fn phase_from(value: u8) -> Result<MatchPhase> {
    Ok(match value {
        0 => MatchPhase::Inactive,
        1 => MatchPhase::Countdown,
        2 => MatchPhase::Kickoff,
        3 => MatchPhase::Active,
        4 => MatchPhase::GoalScored,
        5 => MatchPhase::Replay,
        6 => MatchPhase::Paused,
        7 => MatchPhase::Ended,
        value => {
            return Err(WireError::UnknownEnum {
                name: "GameStateType",
                value,
            })
        }
    })
}

fn u16_struct(t: &Table<'_>, slot: usize) -> Result<Option<u16>> {
    Ok(t.struct_bytes(slot, 2)?
        .and_then(|b| b.try_into().ok())
        .map(u16::from_le_bytes))
}

fn id(t: &Table<'_>, slot: usize, name: &'static str) -> Result<u16> {
    u16_struct(t, slot)?.ok_or(WireError::Missing(name))
}

fn vec3(t: &Table<'_>, slot: usize) -> Result<Option<Vec3>> {
    Ok(t.struct_bytes(slot, <Vec3 as Struct>::SIZE)?
        .and_then(<Vec3 as Struct>::read))
}

fn vec3_req(t: &Table<'_>, slot: usize, name: &'static str) -> Result<Vec3> {
    vec3(t, slot)?.ok_or(WireError::Missing(name))
}

fn physics(t: &Table<'_>, slot: usize, name: &'static str) -> Result<Physics> {
    t.struct_bytes(slot, <Physics as Struct>::SIZE)?
        .and_then(<Physics as Struct>::read)
        .ok_or(WireError::Missing(name))
}

fn rgba(t: &Table<'_>, slot: usize, name: &'static str) -> Result<Rgba> {
    match t.struct_bytes(slot, 4)? {
        Some([r, g, b, a]) => Ok(Rgba {
            r: *r,
            g: *g,
            b: *b,
            a: *a,
        }),
        _ => Err(WireError::Missing(name)),
    }
}

fn read_event(tag: u8, t: &Table<'_>) -> Result<GameEvent> {
    use game_tag::*;
    Ok(match tag {
        GAME_SPAWN => GameEvent::GameSpawn {
            map_name: t.string(0)?.unwrap_or_default().to_owned(),
            mode: t.scalar(1, 0u8)?,
            team_size: t.scalar(2, 0u8)?,
            gravity: vec3(t, 4)?,
        },
        GAME_STATE_CHANGE => GameEvent::StateChange {
            phase: phase_from(t.scalar(0, 0u8)?)?,
            is_overtime: t.scalar(1, false)?,
        },
        GAME_TIME_CHANGE => GameEvent::TimeChange {
            display_seconds: t.scalar(0, 0i16)?,
        },
        GOAL_SPAWN => GameEvent::GoalSpawn {
            id: id(t, 0, "goal id")?,
            team: t.scalar(1, 0u8)?,
            center: vec3_req(t, 2, "goal center")?,
            direction: vec3_req(t, 3, "goal direction")?,
            width: t.scalar(4, 0f32)?,
            height: t.scalar(5, 0f32)?,
        },
        PICKUP_SPAWN => GameEvent::PickupSpawn {
            id: id(t, 0, "pickup id")?,
            location: vec3_req(t, 1, "pickup location")?,
            big: t.scalar(2, 0u8)? == 1,
        },
        PICKUP_STATE_CHANGE => GameEvent::PickupState {
            id: id(t, 0, "pickup id")?,
            picked_up: t.scalar(1, 0u8)? == 1,
        },
        PLAYER_SPAWN => GameEvent::PlayerSpawn {
            id: id(t, 0, "player id")?,
            command_id: u16_struct(t, 1)?,
            name: t.string(2)?.unwrap_or_default().to_owned(),
            team: t.scalar(3, 0u8)?,
        },
        PLAYER_PHYSICS_CHANGE => GameEvent::PlayerPhysics {
            id: id(t, 0, "player id")?,
            physics: physics(t, 1, "player physics")?,
            state: t.scalar(2, 0u8)?,
        },
        PLAYER_BOOST_CHANGE => GameEvent::PlayerBoost {
            id: id(t, 0, "player id")?,
            is_active: t.scalar(1, false)?,
            amount: t.scalar(2, 0u8)?,
        },
        BALL_SPAWN => GameEvent::BallSpawn {
            id: id(t, 0, "ball id")?,
        },
        BALL_PHYSICS_CHANGE => GameEvent::BallPhysics {
            id: id(t, 0, "ball id")?,
            physics: physics(t, 1, "ball physics")?,
        },
        TEAM_SCORE_CHANGE => GameEvent::TeamScore {
            team: t.scalar(0, 0u8)?,
            score: t.scalar(1, 0u8)?,
        },
        ACTOR_DESTROYED => GameEvent::ActorDestroyed {
            id: id(t, 0, "actor id")?,
        },
        tag => GameEvent::Other(tag),
    })
}

/// Decodes a payload sent by the game.
pub fn decode_game_packet(payload: &[u8]) -> Result<GameBundle> {
    let root = Table::root(payload)?;
    let mut events = Vec::new();
    if let Some(messages) = root.vector(1)? {
        if messages.len() > MAX_EVENTS_PER_BUNDLE {
            return Err(WireError::TooLarge("events"));
        }
        let mut string_bytes = 0usize;
        for i in 0..messages.len() {
            let wrapper = messages.table(i)?;
            let tag = wrapper.scalar(0, 0u8)?;
            let event = match wrapper.table(1)? {
                // An event with an enum value or required field this module does not know is
                // one odd event, not a reason to end a session: it is reported as `Other`.
                Some(body) => match read_event(tag, &body) {
                    Ok(event) => event,
                    Err(WireError::UnknownEnum { .. } | WireError::Missing(_)) => {
                        GameEvent::Other(tag)
                    }
                    Err(e) => return Err(e),
                },
                // NONE, or a union member with no table: nothing to read.
                None => GameEvent::Other(tag),
            };
            string_bytes += match &event {
                GameEvent::GameSpawn { map_name, .. } => map_name.len(),
                GameEvent::PlayerSpawn { name, .. } => name.len(),
                _ => 0,
            };
            if string_bytes > MAX_STRING_BYTES_PER_BUNDLE {
                return Err(WireError::TooLarge("string bytes"));
            }
            events.push(event);
        }
    }
    Ok(GameBundle {
        app_seconds: root.scalar(0, 0f32)?,
        delta_physics_frames: root.scalar(2, 0u8)?,
        events,
    })
}

/// Decodes a payload sent to the game (what a stand-in game receives): the commands with their
/// ids. Commands this module does not model are skipped.
pub fn decode_controller_packet(payload: &[u8]) -> Result<Vec<(u16, Command)>> {
    use command_tag::*;
    let root = Table::root(payload)?;
    let Some(messages) = root.vector(0)? else {
        return Ok(Vec::new());
    };
    let mut commands = Vec::new();
    for i in 0..messages.len() {
        let message = messages.table(i)?;
        let command_id = id(&message, 0, "command id")?;
        let tag = message.scalar(1, 0u8)?;
        let Some(t) = message.table(2)? else {
            continue;
        };
        let command = match tag {
            SPAWN_BOT => {
                let l = t.table(3)?.ok_or(WireError::Missing("loadout"))?;
                let mut loadout = Loadout::standard(0);
                loadout.products = [0; LOADOUT_SLOTS];
                if let Some(v) = l.vector(0)? {
                    for (i, slot) in loadout.products.iter_mut().enumerate().take(v.len()) {
                        *slot = v.scalar::<u16>(i)?;
                    }
                }
                if let Some(v) = l.vector(3)? {
                    for (i, slot) in loadout.paints.iter_mut().enumerate().take(v.len()) {
                        *slot = v.scalar::<u8>(i)?;
                    }
                }
                loadout.primary = rgba(&l, 1, "primary colour")?;
                loadout.secondary = rgba(&l, 2, "secondary colour")?;
                Command::SpawnBot {
                    name: t.string(0)?.unwrap_or_default().to_owned(),
                    skill: BotSkill::from_u8(t.scalar(1, 0u8)?)?,
                    team: t.scalar(2, 0u8)?,
                    loadout,
                }
            }
            PLAYER_INPUT => {
                let input = t.table(1)?.ok_or(WireError::Missing("input"))?;
                Command::PlayerInput {
                    actor_id: id(&t, 0, "player id")?,
                    input: VehicleInput {
                        throttle: input.scalar(0, 0f32)?,
                        steer: input.scalar(1, 0f32)?,
                        pitch: input.scalar(2, 0f32)?,
                        yaw: input.scalar(3, 0f32)?,
                        roll: input.scalar(4, 0f32)?,
                        dodge_forward: input.scalar(5, 0f32)?,
                        dodge_strafe: input.scalar(6, 0f32)?,
                        jump: input.scalar(7, false)?,
                        boost: input.scalar(8, false)?,
                        handbrake: input.scalar(9, false)?,
                        use_item: input.scalar(10, false)?,
                    },
                }
            }
            EXEC => Command::Exec(t.string(0)?.unwrap_or_default().to_owned()),
            SET_PAUSED => Command::SetPaused(t.scalar(0, false)?),
            END_MATCH => Command::EndMatch,
            DESTROY_ACTOR => Command::DestroyActor(id(&t, 0, "actor id")?),
            SET_PHYSICS => Command::SetPhysics {
                actor_id: id(&t, 0, "actor id")?,
                physics: physics(&t, 1, "physics")?,
            },
            SET_BOOST => Command::SetBoost {
                actor_id: id(&t, 0, "player id")?,
                amount: t.scalar(1, 0u8)?,
            },
            _ => continue,
        };
        commands.push((command_id, command));
    }
    Ok(commands)
}

// ---- writing ----

fn build(
    b: &mut Builder,
    fill: impl FnOnce(&mut TableBuilder<'_>) -> Result<()>,
) -> Result<Offset> {
    let mut t = b.start_table();
    fill(&mut t)?;
    Ok(t.finish()?)
}

fn add_u16(t: &mut TableBuilder<'_>, slot: usize, v: u16) -> Result<()> {
    Ok(t.add_struct(slot, &v.to_le_bytes(), 2)?)
}

fn add_vec3(t: &mut TableBuilder<'_>, slot: usize, v: &Vec3) -> Result<()> {
    let mut bytes = Vec::new();
    v.write(&mut bytes);
    Ok(t.add_struct(slot, &bytes, <Vec3 as Struct>::ALIGN)?)
}

fn add_physics(t: &mut TableBuilder<'_>, slot: usize, p: &Physics) -> Result<()> {
    let mut bytes = Vec::new();
    p.write(&mut bytes);
    Ok(t.add_struct(slot, &bytes, <Physics as Struct>::ALIGN)?)
}

fn phase_byte(phase: MatchPhase) -> u8 {
    phase as u8
}

fn write_event(b: &mut Builder, event: &GameEvent) -> Result<(u8, Option<Offset>)> {
    use game_tag::*;
    Ok(match event {
        GameEvent::GameSpawn {
            map_name,
            mode,
            team_size,
            gravity,
        } => {
            let name = b.create_string(map_name)?;
            let o = build(b, |t| {
                t.add_offset(0, name)?;
                t.add_scalar(1, *mode, 0u8)?;
                t.add_scalar(2, *team_size, 0u8)?;
                gravity.as_ref().map_or(Ok(()), |g| add_vec3(t, 4, g))
            })?;
            (GAME_SPAWN, Some(o))
        }
        GameEvent::StateChange { phase, is_overtime } => (
            GAME_STATE_CHANGE,
            Some(build(b, |t| {
                t.add_scalar(0, phase_byte(*phase), 0u8)?;
                Ok(t.add_scalar(1, *is_overtime, false)?)
            })?),
        ),
        GameEvent::TimeChange { display_seconds } => (
            GAME_TIME_CHANGE,
            Some(build(b, |t| {
                Ok(t.add_scalar(0, *display_seconds, 0i16)?)
            })?),
        ),
        GameEvent::GoalSpawn {
            id,
            team,
            center,
            direction,
            width,
            height,
        } => (
            GOAL_SPAWN,
            Some(build(b, |t| {
                add_u16(t, 0, *id)?;
                t.add_scalar(1, *team, 0u8)?;
                add_vec3(t, 2, center)?;
                add_vec3(t, 3, direction)?;
                t.add_scalar(4, *width, 0f32)?;
                Ok(t.add_scalar(5, *height, 0f32)?)
            })?),
        ),
        GameEvent::PickupSpawn { id, location, big } => (
            PICKUP_SPAWN,
            Some(build(b, |t| {
                add_u16(t, 0, *id)?;
                add_vec3(t, 1, location)?;
                Ok(t.add_scalar(2, u8::from(*big), 0u8)?)
            })?),
        ),
        GameEvent::PickupState { id, picked_up } => (
            PICKUP_STATE_CHANGE,
            Some(build(b, |t| {
                add_u16(t, 0, *id)?;
                Ok(t.add_scalar(1, u8::from(*picked_up), 0u8)?)
            })?),
        ),
        GameEvent::PlayerSpawn {
            id,
            command_id,
            name,
            team,
        } => {
            let name = b.create_string(name)?;
            let o = build(b, |t| {
                add_u16(t, 0, *id)?;
                if let Some(c) = command_id {
                    add_u16(t, 1, *c)?;
                }
                t.add_offset(2, name)?;
                Ok(t.add_scalar(3, *team, 0u8)?)
            })?;
            (PLAYER_SPAWN, Some(o))
        }
        GameEvent::PlayerPhysics { id, physics, state } => (
            PLAYER_PHYSICS_CHANGE,
            Some(build(b, |t| {
                add_u16(t, 0, *id)?;
                add_physics(t, 1, physics)?;
                Ok(t.add_scalar(2, *state, 0u8)?)
            })?),
        ),
        GameEvent::PlayerBoost {
            id,
            is_active,
            amount,
        } => (
            PLAYER_BOOST_CHANGE,
            Some(build(b, |t| {
                add_u16(t, 0, *id)?;
                t.add_scalar(1, *is_active, false)?;
                Ok(t.add_scalar(2, *amount, 0u8)?)
            })?),
        ),
        GameEvent::BallSpawn { id } => (BALL_SPAWN, Some(build(b, |t| add_u16(t, 0, *id))?)),
        GameEvent::BallPhysics { id, physics } => (
            BALL_PHYSICS_CHANGE,
            Some(build(b, |t| {
                add_u16(t, 0, *id)?;
                add_physics(t, 1, physics)
            })?),
        ),
        GameEvent::TeamScore { team, score } => (
            TEAM_SCORE_CHANGE,
            Some(build(b, |t| {
                t.add_scalar(0, *team, 0u8)?;
                Ok(t.add_scalar(1, *score, 0u8)?)
            })?),
        ),
        GameEvent::ActorDestroyed { id } => {
            (ACTOR_DESTROYED, Some(build(b, |t| add_u16(t, 0, *id))?))
        }
        // Not buildable: there is nothing to say about an event we do not model.
        GameEvent::Other(tag) => (*tag, None),
    })
}

/// Encodes what the game would send (for a stand-in game).
pub fn encode_game_packet(bundle: &GameBundle) -> Result<Vec<u8>> {
    let mut b = Builder::new();
    let mut wrappers = Vec::new();
    for event in &bundle.events {
        let (tag, body) = write_event(&mut b, event)?;
        wrappers.push(build(&mut b, |t| {
            t.add_scalar(0, tag, 0u8)?;
            body.map_or(Ok(()), |o| Ok(t.add_offset(1, o)?))
        })?);
    }
    let messages = b.create_offset_vector(&wrappers)?;
    let root = build(&mut b, |t| {
        t.add_scalar(0, bundle.app_seconds, 0f32)?;
        t.add_offset(1, messages)?;
        Ok(t.add_scalar(2, bundle.delta_physics_frames, 0u8)?)
    })?;
    Ok(b.finish(root, None)?)
}

fn write_command(b: &mut Builder, command: &Command) -> Result<(u8, Offset)> {
    use command_tag::*;
    Ok(match command {
        Command::SpawnBot {
            name,
            skill,
            team,
            loadout,
        } => {
            let products = b.create_vector::<u16>(&loadout.products)?;
            let paints = b.create_vector::<u8>(&loadout.paints)?;
            let color = |c: &Rgba| [c.r, c.g, c.b, c.a];
            let l = build(b, |t| {
                t.add_offset(0, products)?;
                t.add_struct(1, &color(&loadout.primary), 1)?;
                t.add_struct(2, &color(&loadout.secondary), 1)?;
                Ok(t.add_offset(3, paints)?)
            })?;
            let name = b.create_string(name)?;
            let o = build(b, |t| {
                t.add_offset(0, name)?;
                t.add_scalar(1, *skill as u8, 0u8)?;
                t.add_scalar(2, *team, 0u8)?;
                Ok(t.add_offset(3, l)?)
            })?;
            (SPAWN_BOT, o)
        }
        Command::PlayerInput { actor_id, input } => {
            let vehicle = build(b, |t| {
                t.add_scalar(0, input.throttle, 0f32)?;
                t.add_scalar(1, input.steer, 0f32)?;
                t.add_scalar(2, input.pitch, 0f32)?;
                t.add_scalar(3, input.yaw, 0f32)?;
                t.add_scalar(4, input.roll, 0f32)?;
                t.add_scalar(5, input.dodge_forward, 0f32)?;
                t.add_scalar(6, input.dodge_strafe, 0f32)?;
                t.add_scalar(7, input.jump, false)?;
                t.add_scalar(8, input.boost, false)?;
                t.add_scalar(9, input.handbrake, false)?;
                Ok(t.add_scalar(10, input.use_item, false)?)
            })?;
            let o = build(b, |t| {
                add_u16(t, 0, *actor_id)?;
                Ok(t.add_offset(1, vehicle)?)
            })?;
            (PLAYER_INPUT, o)
        }
        Command::Exec(text) => {
            let text = b.create_string(text)?;
            (EXEC, build(b, |t| Ok(t.add_offset(0, text)?))?)
        }
        Command::SetPaused(paused) => (
            SET_PAUSED,
            build(b, |t| Ok(t.add_scalar(0, *paused, false)?))?,
        ),
        Command::EndMatch => (END_MATCH, build(b, |_| Ok(()))?),
        Command::DestroyActor(actor) => (DESTROY_ACTOR, build(b, |t| add_u16(t, 0, *actor))?),
        Command::SetPhysics { actor_id, physics } => (
            SET_PHYSICS,
            build(b, |t| {
                add_u16(t, 0, *actor_id)?;
                add_physics(t, 1, physics)
            })?,
        ),
        Command::SetBoost { actor_id, amount } => (
            SET_BOOST,
            build(b, |t| {
                add_u16(t, 0, *actor_id)?;
                Ok(t.add_scalar(1, *amount, 0u8)?)
            })?,
        ),
    })
}

/// Encodes commands as one `ControllerPacket`. Command ids count up from `first_id`, wrapping
/// at 65535 (RLBot starts a queue at 1); the ids are returned with the bytes.
pub fn encode_controller_packet(first_id: u16, commands: &[Command]) -> Result<Vec<u8>> {
    let mut b = Builder::new();
    let mut messages = Vec::new();
    let mut next = first_id;
    for command in commands {
        let (tag, body) = write_command(&mut b, command)?;
        let command_id = next;
        next = next.wrapping_add(1);
        messages.push(build(&mut b, |t| {
            add_u16(t, 0, command_id)?;
            t.add_scalar(1, tag, 0u8)?;
            Ok(t.add_offset(2, body)?)
        })?);
    }
    let vector = b.create_offset_vector(&messages)?;
    let root = build(&mut b, |t| Ok(t.add_offset(0, vector)?))?;
    Ok(b.finish(root, None)?)
}
