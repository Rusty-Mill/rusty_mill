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
    /// Team id -> final score.
    pub team_scores: BTreeMap<i32, i32>,
    /// Per-player summary stats from the header `PlayerStats` array.
    pub players: Vec<PlayerMeta>,
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
        /// Sleeping bodies report no velocity (treated as zero).
        sleeping: bool,
    },
    /// A car (`Engine.Pawn`) was bound to a player-replication-info actor.
    CarPri { car: i32, pri: i32 },
    /// A PRI actor was assigned a player name.
    PriName { pri: i32, name: String },
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
