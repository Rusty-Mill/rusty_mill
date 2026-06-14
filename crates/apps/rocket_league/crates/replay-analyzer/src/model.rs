//! The **canonical match model**: the parser-agnostic contract every downstream
//! consumer (scoring, heatmaps, validation) depends on. Nothing here references
//! `boxcars` or any decoder type, and nothing here knows about scoring.

use serde::{Deserialize, Serialize};
use std::collections::BTreeMap;

/// A 3D vector / position in unreal units (uu).
#[derive(Debug, Clone, Copy, PartialEq, Serialize, Deserialize)]
pub struct Vec3 {
    pub x: f32,
    pub y: f32,
    pub z: f32,
}

impl Vec3 {
    /// Construct from a raw `[x, y, z]` array.
    pub fn from_arr(a: [f32; 3]) -> Self {
        Vec3 { x: a[0], y: a[1], z: a[2] }
    }

    /// Convert back to a raw `[x, y, z]` array.
    pub fn to_arr(self) -> [f32; 3] {
        [self.x, self.y, self.z]
    }
}

/// Header-sourced per-player metadata (from the replay's `PlayerStats` array).
/// This is end-of-match summary truth, independent of frame reconstruction.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct PlayerMeta {
    pub name: String,
    pub team: i32,
    pub score: i32,
    pub goals: i32,
    pub assists: i32,
    pub saves: i32,
    pub shots: i32,
}

/// Per-frame state of a single car, keyed by the *raw* car actor id (which RL
/// recycles — see [`PlayerTrack`] for the coalesced, identity-stable view).
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct CarState {
    /// Raw car actor id for this frame (not stable across respawns).
    pub actor_id: i32,
    /// Stable player-replication-info id this car was bound to this frame.
    pub pri: i32,
    /// Resolved player name, if the car was bound to a named PRI.
    pub player: Option<String>,
    pub p: Vec3,
    pub v: Vec3,
    /// Boost amount, raw replicated byte (0..=255; ~`/2.55` for percent).
    /// `None` until the car's boost component reports.
    pub boost: Option<u8>,
}

/// One reconstructed frame of world state (carry-forward of last-known actor
/// positions). Cars present in this frame are those with a live actor.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct FrameOut {
    pub t: f32,
    pub ball: Option<Vec3>,
    pub cars: Vec<CarState>,
}

/// One position/velocity observation belonging to a coalesced player track.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct TrackSample {
    pub t: f32,
    /// The car actor id that produced this sample (changes across respawns).
    pub actor_id: i32,
    pub p: Vec3,
    pub v: Vec3,
    /// Boost amount at this sample, raw replicated byte (0..=255). `None` for the
    /// ball and for cars whose boost has not yet been observed.
    pub boost: Option<u8>,
}

/// Why a player's track has a discontinuity. Reconstruction marks the boundary
/// between two live car segments; classifying it as demo vs. goal-reset vs.
/// generic respawn requires event extraction (a later milestone), so for now
/// every reconstructed gap is [`GapReason::Respawn`].
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum GapReason {
    /// The player's car actor was destroyed and a new one later took its place
    /// (respawn, demo, or goal reset — not yet distinguished).
    Respawn,
}

/// An explicit gap in a [`PlayerTrack`]: the interval during which the player
/// had no live car. Consumers MUST NOT carry-forward state across a gap.
#[derive(Debug, Clone, Copy, PartialEq, Serialize, Deserialize)]
pub struct TrackGap {
    /// Time of the last sample before the gap.
    pub start: f32,
    /// Time of the first sample after the gap.
    pub end: f32,
    pub reason: GapReason,
}

impl TrackGap {
    /// Duration of the gap in seconds.
    pub fn duration(&self) -> f32 {
        self.end - self.start
    }
}

/// All car-actor segments belonging to one player, coalesced into a single
/// continuous track keyed on stable player identity (PRI), with respawn/dead
/// gaps marked explicitly. This is the T1 deliverable that fixes one player
/// fragmenting into dozens of recycled actor-id segments.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct PlayerTrack {
    /// Resolved player name (the human identity).
    pub player: String,
    /// Stable player-replication-info actor id (the in-replay identity key).
    pub pri: i32,
    /// Team (0/1) joined from header `PlayerStats`, if known.
    pub team: Option<i32>,
    /// Number of distinct live car segments that were coalesced.
    pub num_segments: usize,
    /// All samples across every segment, in ascending time order.
    pub samples: Vec<TrackSample>,
    /// Boundaries between segments; never carry-forward across these.
    pub gaps: Vec<TrackGap>,
}

/// Position + velocity of an actor at an instant (unreal units; uu/s).
#[derive(Debug, Clone, Copy, PartialEq, Serialize, Deserialize)]
pub struct Kin {
    pub p: Vec3,
    pub v: Vec3,
}

/// One car on the fixed-rate resample grid, in **world** coordinates.
///
/// To view this car in its team's attacking-direction frame (each team attacks
/// `+Y`), apply [`crate::analyze::normalize::flip_xy`] with
/// [`Resampled::team_attack_sign`] for [`GridCar::team`].
#[derive(Debug, Clone, Copy, PartialEq, Serialize, Deserialize)]
pub struct GridCar {
    /// Stable player identity (PRI) — the grid key for a player.
    pub pri: i32,
    /// Team (0/1), if known.
    pub team: Option<i32>,
    pub p: Vec3,
    pub v: Vec3,
    /// Boost amount, raw replicated byte (0..=255), carried from the most recent
    /// observation at or before this grid time.
    pub boost: Option<u8>,
}

/// One frame of the fixed-rate resample grid. Every live actor is present
/// (interpolated); actors dead/respawning at this instant are absent.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct GridFrame {
    pub t: f32,
    /// Ball kinematics in **world** coordinates (shared across teams; normalize
    /// per-team on demand).
    pub ball: Option<Kin>,
    pub cars: Vec<GridCar>,
}

/// The fixed-rate, gap-aware resampling of the match (T2), in world coordinates,
/// plus the per-team transform that puts each team attacking `+Y`.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct Resampled {
    /// Grid rate in Hz (e.g. 30.0).
    pub hz: f32,
    /// Team id -> sign in `{+1, -1}`. Multiplying a world `x` and `y` by this
    /// sign rotates that team's frame so it attacks `+Y` (a 180° z-rotation when
    /// `-1`). `z` and speeds are unchanged.
    pub team_attack_sign: BTreeMap<i32, i32>,
    pub frames: Vec<GridFrame>,
}

/// The canonical match model emitted by the analyzer.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct CanonicalMatch {
    pub replay_id: String,
    /// Real decoder version, e.g. `boxcars-0.11.3` (pinned for reproducibility).
    pub parser_version: String,
    /// This analyzer crate's version.
    pub analyzer_version: String,
    pub map: Option<String>,
    pub team_size: Option<i32>,
    pub record_fps: Option<f32>,
    pub num_frames: usize,
    pub duration_s: f32,
    /// Team id -> final score.
    pub team_scores: BTreeMap<i32, i32>,
    pub players: Vec<PlayerMeta>,
    /// Coalesced per-player tracks (T1), keyed by stable identity.
    pub tracks: Vec<PlayerTrack>,
    /// Per-frame reconstructed world state (native frame rate, carry-forward).
    pub frames: Vec<FrameOut>,
    /// Fixed-rate, gap-aware resampling with attack-direction normalization (T2).
    pub resampled: Resampled,
}
