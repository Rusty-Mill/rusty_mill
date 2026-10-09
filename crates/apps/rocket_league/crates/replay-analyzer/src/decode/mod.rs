//! Decode port (ports-and-adapters boundary).
//!
//! The analyzer never touches `boxcars` types directly. A [`ReplayParser`]
//! adapter translates a raw `.replay` into a neutral [`DecodedReplay`]: header
//! metadata plus a normalized per-frame event stream containing only the facts
//! reconstruction needs. Swapping decoders (carball, ballchasing) means writing
//! another adapter — the analyze layer is unaffected and stays unit-testable
//! against hand-built event streams.

pub mod boxcars_adapter;

use crate::model::PlayerMeta;
use std::collections::BTreeMap;
use std::fmt;

/// What a network actor represents, classified at spawn from its archetype name.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ActorClass {
    Ball,
    Car,
    Other,
}

/// Header-derived match metadata, decoder-agnostic.
#[derive(Debug, Clone, PartialEq)]
pub struct ReplayMeta {
    /// `boxcars-<resolved version>`.
    pub parser_version: String,
    pub map: Option<String>,
    pub team_size: Option<i32>,
    pub record_fps: Option<f32>,
    /// When the match was played: the header `Date` (the recorder's clock, read as UTC), unix seconds.
    pub played_at: Option<u64>,
    /// Team id -> final score.
    pub team_scores: BTreeMap<i32, i32>,
    /// Per-player summary stats from the header `PlayerStats` array.
    pub players: Vec<PlayerMeta>,
    /// Goals from the header `Goals` array (authoritative scorer/team per goal).
    pub goals: Vec<GoalInfo>,
}

/// One scored goal, from the header `Goals` array.
#[derive(Debug, Clone, PartialEq)]
pub struct GoalInfo {
    /// Network frame index the goal was recorded at.
    pub frame: i32,
    pub scorer: Option<String>,
    pub team: Option<i32>,
}

/// A normalized actor-attribute update — only the kinds reconstruction consumes.
/// All positions/velocities are in unreal units (uu).
#[derive(Debug, Clone, PartialEq)]
pub enum ActorUpdate {
    /// Rigid-body state for a ball or car actor.
    RigidBody {
        actor: i32,
        p: [f32; 3],
        v: [f32; 3],
        /// Orientation `[pitch, yaw, roll]` in radians.
        rot: [f32; 3],
        /// Sleeping bodies report no velocity (treated as zero).
        sleeping: bool,
    },
    /// A car (`Engine.Pawn`) was bound to a player-replication-info actor.
    CarPri { car: i32, pri: i32 },
    /// A PRI actor was assigned a player name.
    PriName { pri: i32, name: String },
    /// A PRI actor was assigned to a team (`0` = blue, `1` = orange), resolved
    /// from `Engine.PlayerReplicationInfo:Team` → the team actor's archetype.
    PriTeam { pri: i32, team: i32 },
    /// A car-component actor was linked to its car (`TAGame.CarComponent_TA:Vehicle`).
    /// Only boost components are queried downstream, but all links are emitted.
    CompVehicle { comp: i32, car: i32 },
    /// A boost component reported its boost amount (raw byte, 0..=255; ~`/2.55`
    /// for percent). Sourced from `ReplicatedBoostAmount` or `ReplicatedBoost`.
    BoostAmount { comp: i32, amount: u8 },
    /// A car was demolished. Car actor ids (resolve to players via current
    /// binding); sourced from `ReplicatedDemolish`/`ReplicatedDemolishExtended`.
    Demolish { attacker_car: i32, victim_car: i32 },
    /// A per-player scoreboard counter replicated on the PRI
    /// (`TAGame.PRI_TA:Match{Score,Goals,Saves,Assists,Shots}`). Authoritative and
    /// present in the network stream even when the header `PlayerStats[]` array is
    /// empty — the fallback source for per-player core stats.
    PriStat {
        pri: i32,
        stat: PriStatKind,
        value: i32,
    },
    /// A boost pad was **collected** (`TAGame.VehiclePickup_TA:(New)ReplicatedPickupData`).
    /// `instigator_car` is the collecting car actor (resolve to a PRI). Emitted
    /// only when an instigator is present — i.e. a real collection, not a pad
    /// becoming available again.
    PickupBoost { instigator_car: i32 },
    /// A car's handbrake (powerslide) state toggled
    /// (`TAGame.Vehicle_TA:bReplicatedHandbrake`).
    Handbrake { car: i32, on: bool },
    /// A PRI's chosen car-body product ids, from its loadout
    /// (`TAGame.PRI_TA:ClientLoadout(s)` → `Loadout.body`). The team loadout
    /// carries a body for each side; the player uses the one matching their team
    /// (`blue_body` for team 0, `orange_body` for team 1 — usually identical). The
    /// singular `ClientLoadout` sets both to the same value.
    Loadout {
        pri: i32,
        blue_body: u32,
        orange_body: u32,
    },
    /// A camera-settings actor reported its profile (`CamSettings`). Attributed to
    /// a PRI via the [`ActorUpdate::CameraPri`] link on the same `cam_actor`.
    /// `angle`/`swivel`/`transition` are ballchasing's `pitch`/`swivel_speed`/
    /// `transition_speed`.
    CameraSettings {
        cam_actor: i32,
        fov: f32,
        height: f32,
        angle: f32,
        distance: f32,
        stiffness: f32,
        swivel: f32,
        transition: f32,
    },
    /// A camera-settings actor was bound to its owning PRI
    /// (`TAGame.CameraSettingsActor_TA:PRI`).
    CameraPri { cam_actor: i32, pri: i32 },
    /// A PRI's steering sensitivity (`TAGame.PRI_TA:SteeringSensitivity`, a float;
    /// RL's default is `1.0`).
    SteeringSensitivity { pri: i32, value: f32 },
}

/// Which per-player scoreboard counter a [`ActorUpdate::PriStat`] carries.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum PriStatKind {
    Score,
    Goals,
    Saves,
    Assists,
    Shots,
}

/// A new actor appearing in a frame, with its classification.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct NewActorEvent {
    pub actor_id: i32,
    pub class: ActorClass,
}

/// One frame's worth of normalized network events.
#[derive(Debug, Clone, PartialEq, Default)]
pub struct RawFrame {
    pub time: f32,
    pub delta: f32,
    pub new_actors: Vec<NewActorEvent>,
    pub updates: Vec<ActorUpdate>,
    pub deleted: Vec<i32>,
}

/// The full neutral decode: header metadata plus the frame event stream.
#[derive(Debug, Clone, PartialEq)]
pub struct DecodedReplay {
    pub meta: ReplayMeta,
    pub frames: Vec<RawFrame>,
}

/// Errors a decoder adapter can surface.
#[derive(Debug)]
pub enum DecodeError {
    /// The underlying parser rejected the replay.
    Parse(String),
    /// The replay parsed but carried no network frames to reconstruct from.
    NoNetworkData,
}

impl fmt::Display for DecodeError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            DecodeError::Parse(e) => write!(f, "failed to parse replay: {e}"),
            DecodeError::NoNetworkData => {
                write!(f, "replay contained no network data to reconstruct")
            }
        }
    }
}

impl std::error::Error for DecodeError {}

/// The decode port. An adapter turns raw `.replay` bytes into a neutral
/// [`DecodedReplay`]; the analyze layer depends only on this trait.
pub trait ReplayParser {
    fn parse(&self, data: &[u8]) -> Result<DecodedReplay, DecodeError>;
}
