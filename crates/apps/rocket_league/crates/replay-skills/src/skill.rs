//! The skill catalog (taxonomy).
//!
//! Each [`Skill`] is one **individual mechanic** the detectors in
//! [`crate::detect`] look for. The existing scoring layer is deliberately scoped
//! to *decision discipline, not mechanics* (see the design spec §0); this catalog
//! is the complementary axis — "what did the player *do*", inferred from the same
//! kinematics.
//!
//! Skills carry their [`SkillCategory`] (how they group) and [`Detection`] source
//! (whether they fall out of authoritative events or are heuristically inferred
//! from motion) so a consumer can be honest about how much to trust each one.

use serde::{Deserialize, Serialize};

/// One mechanical skill the analyzer can verify within a replay.
///
/// The set is deliberately curated to skills with a *robust kinematic
/// signature*; input-only mechanics (flip resets, half-flips, speedflips) are
/// intentionally absent because replays carry motion, not controller inputs
/// (spec §0). Adding a variant is additive — wire a detector in [`crate::detect`]
/// and a row in [`Skill::ALL`].
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum Skill {
    /// Reached supersonic speed (≥ `field::SUPERSONIC_SPEED`).
    Supersonic,
    /// Touched the ball while airborne and elevated.
    Aerial,
    /// Two or more consecutive aerial touches kept up in the air (air dribble).
    AirDribble,
    /// Drove on / dropped from the ceiling.
    CeilingPlay,
    /// Touched the ball while up on a wall.
    WallPlay,
    /// Carried the ball balanced on the car along the ground (ground dribble).
    GroundDribble,
    /// Popped a carried ball sharply upward off the car (flick).
    Flick,
    /// Struck the ball hard toward the opponent goal (power shot / clear).
    PowerShot,
    /// Sharply changed an incoming ball's direction toward goal (redirect).
    Redirect,
    /// Won the first touch off a kickoff.
    KickoffFirstTouch,
    /// Grabbed a full corner boost pad in the opponent's half (boost steal).
    BoostSteal,
    /// Demolished an opponent.
    Demo,
}

/// How a family of skills groups, for display and roll-ups.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum SkillCategory {
    Aerial,
    Dribbling,
    Striking,
    Wall,
    Speed,
    Kickoff,
    Boost,
    Aggression,
}

/// Where a skill's evidence comes from — how much to trust a positive.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum Detection {
    /// Falls out of an authoritative replay event (e.g. a demolish attribute).
    Event,
    /// Heuristically inferred from motion (position/velocity/rotation/boost).
    Kinematic,
}

impl Skill {
    /// Every skill in the catalog, in a stable order (used by the CLI `--list`
    /// and to iterate the taxonomy).
    pub const ALL: [Skill; 12] = [
        Skill::Supersonic,
        Skill::Aerial,
        Skill::AirDribble,
        Skill::CeilingPlay,
        Skill::WallPlay,
        Skill::GroundDribble,
        Skill::Flick,
        Skill::PowerShot,
        Skill::Redirect,
        Skill::KickoffFirstTouch,
        Skill::BoostSteal,
        Skill::Demo,
    ];

    /// Stable string key (for reports, CLI `--verify`, and serialization).
    pub fn key(&self) -> &'static str {
        match self {
            Skill::Supersonic => "supersonic",
            Skill::Aerial => "aerial",
            Skill::AirDribble => "air_dribble",
            Skill::CeilingPlay => "ceiling_play",
            Skill::WallPlay => "wall_play",
            Skill::GroundDribble => "ground_dribble",
            Skill::Flick => "flick",
            Skill::PowerShot => "power_shot",
            Skill::Redirect => "redirect",
            Skill::KickoffFirstTouch => "kickoff_first_touch",
            Skill::BoostSteal => "boost_steal",
            Skill::Demo => "demo",
        }
    }

    /// Resolve a skill from its [`Skill::key`] (for CLI `--verify <skill>`).
    pub fn from_key(s: &str) -> Option<Skill> {
        Skill::ALL.into_iter().find(|sk| sk.key() == s)
    }

    /// Human-readable display name.
    pub fn display_name(&self) -> &'static str {
        match self {
            Skill::Supersonic => "Supersonic",
            Skill::Aerial => "Aerial",
            Skill::AirDribble => "Air dribble",
            Skill::CeilingPlay => "Ceiling play",
            Skill::WallPlay => "Wall play",
            Skill::GroundDribble => "Ground dribble",
            Skill::Flick => "Flick",
            Skill::PowerShot => "Power shot",
            Skill::Redirect => "Redirect",
            Skill::KickoffFirstTouch => "Kickoff first touch",
            Skill::BoostSteal => "Boost steal",
            Skill::Demo => "Demo",
        }
    }

    /// Which family the skill belongs to.
    pub fn category(&self) -> SkillCategory {
        match self {
            Skill::Aerial | Skill::AirDribble | Skill::CeilingPlay => SkillCategory::Aerial,
            Skill::WallPlay => SkillCategory::Wall,
            Skill::GroundDribble | Skill::Flick => SkillCategory::Dribbling,
            Skill::PowerShot | Skill::Redirect => SkillCategory::Striking,
            Skill::Supersonic => SkillCategory::Speed,
            Skill::KickoffFirstTouch => SkillCategory::Kickoff,
            Skill::BoostSteal => SkillCategory::Boost,
            Skill::Demo => SkillCategory::Aggression,
        }
    }

    /// Evidence provenance — authoritative event vs. inferred from motion.
    pub fn detection(&self) -> Detection {
        match self {
            Skill::Demo => Detection::Event,
            _ => Detection::Kinematic,
        }
    }

    /// One-line description of what the detector looks for.
    pub fn description(&self) -> &'static str {
        match self {
            Skill::Supersonic => "Reached supersonic speed.",
            Skill::Aerial => "Touched the ball while airborne and elevated.",
            Skill::AirDribble => "Kept the ball up with consecutive aerial touches.",
            Skill::CeilingPlay => "Drove on or dropped from the ceiling.",
            Skill::WallPlay => "Touched the ball while up on a wall.",
            Skill::GroundDribble => "Carried the ball balanced on the car along the ground.",
            Skill::Flick => "Popped a carried ball sharply upward off the car.",
            Skill::PowerShot => "Struck the ball hard toward the opponent goal.",
            Skill::Redirect => "Sharply changed an incoming ball's direction toward goal.",
            Skill::KickoffFirstTouch => "Won the first touch off a kickoff.",
            Skill::BoostSteal => "Grabbed a full corner boost pad in the opponent's half.",
            Skill::Demo => "Demolished an opponent.",
        }
    }
}
