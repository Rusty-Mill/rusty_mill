//! The skill report and the verification query surface.
//!
//! [`SkillReport`] is the per-replay catalog of every detected skill occurrence
//! ([`SkillInstance`]) plus a per-player roll-up ([`PlayerSkills`]). Its inherent
//! methods are the **verification API** — the answer to "was this skill performed
//! within the replay", sliceable by player and by time window.

use std::collections::BTreeMap;

use serde::{Deserialize, Serialize};

use crate::skill::Skill;

/// One detected occurrence of a skill, attributed to a player at a time.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct SkillInstance {
    pub skill: Skill,
    /// Match time of the occurrence (s).
    pub t: f32,
    /// Stable player identity (PRI) credited with the skill.
    pub pri: i32,
    pub player: Option<String>,
    pub team: Option<i32>,
    /// Heuristic confidence in this detection, `0.0..=1.0` (1.0 for skills with
    /// an authoritative event source).
    pub confidence: f32,
    /// The skill's **primary evidence magnitude** in its natural unit — the
    /// structured number behind the detection (aerial peak height in uu, dribble
    /// duration in s, power-shot ball speed in uu/s, redirect angle in deg, …),
    /// described by [`Skill::metric_label`] / [`Skill::metric_unit`]. Distinct
    /// from `confidence` (a normalized detector certainty): this is the raw
    /// physical quantity, so profiles can report mean aerial height rather than
    /// only a mean confidence. Interpretation is per-skill (most read
    /// higher-is-bigger; kickoff `reaction` is lower-is-better).
    pub metric: f32,
    /// Human-readable supporting evidence (e.g. `"car_z=850 ball_z=910"`).
    pub detail: String,
}

/// Per-player roll-up: how many times the player performed each skill.
///
/// `counts` only carries skills performed at least once, so an empty map means
/// "no catalogued skills detected for this player".
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct PlayerSkills {
    pub pri: i32,
    pub player: String,
    pub team: Option<i32>,
    pub counts: BTreeMap<Skill, usize>,
}

impl PlayerSkills {
    /// Did this player perform `skill` at least once?
    pub fn performed(&self, skill: Skill) -> bool {
        self.count(skill) > 0
    }

    /// How many times this player performed `skill`.
    pub fn count(&self, skill: Skill) -> usize {
        self.counts.get(&skill).copied().unwrap_or(0)
    }

    /// The distinct skills this player performed, in catalog order.
    pub fn skills(&self) -> Vec<Skill> {
        self.counts.keys().copied().collect()
    }
}

/// The full per-replay skill catalog plus the verification query surface.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct SkillReport {
    pub replay_id: String,
    /// `SkillConfig` version this report was produced under (reproducibility).
    pub config_version: String,
    pub analyzer_version: String,
    pub parser_version: String,
    /// Every detected occurrence, sorted by time then skill then player.
    pub instances: Vec<SkillInstance>,
    /// Per-player roll-up, sorted by team then PRI.
    pub players: Vec<PlayerSkills>,
}

impl SkillReport {
    /// Was `skill` performed by **anyone** in the replay?
    pub fn performed(&self, skill: Skill) -> bool {
        self.instances.iter().any(|i| i.skill == skill)
    }

    /// Was `skill` performed by the player with this PRI?
    pub fn performed_by(&self, skill: Skill, pri: i32) -> bool {
        self.count_for(skill, pri) > 0
    }

    /// Was `skill` performed by the (first) player with this name?
    pub fn performed_by_name(&self, skill: Skill, name: &str) -> bool {
        self.player_by_name(name)
            .map(|p| p.performed(skill))
            .unwrap_or(false)
    }

    /// How many times the player with this PRI performed `skill`.
    pub fn count_for(&self, skill: Skill, pri: i32) -> usize {
        self.player(pri).map(|p| p.count(skill)).unwrap_or(0)
    }

    /// Total occurrences of `skill` across all players.
    pub fn total(&self, skill: Skill) -> usize {
        self.instances.iter().filter(|i| i.skill == skill).count()
    }

    /// Was `skill` performed within the time window `[start, end]` (inclusive)?
    /// Lets a caller verify a skill happened inside a specific clip of the replay.
    pub fn performed_in_window(&self, skill: Skill, start: f32, end: f32) -> bool {
        self.instances
            .iter()
            .any(|i| i.skill == skill && i.t >= start && i.t <= end)
    }

    /// Was `skill` performed by this PRI within the time window `[start, end]`?
    pub fn performed_by_in_window(&self, skill: Skill, pri: i32, start: f32, end: f32) -> bool {
        self.instances
            .iter()
            .any(|i| i.skill == skill && i.pri == pri && i.t >= start && i.t <= end)
    }

    /// All occurrences of `skill`, time-sorted (a borrowing iterator).
    pub fn instances_of(&self, skill: Skill) -> impl Iterator<Item = &SkillInstance> {
        self.instances.iter().filter(move |i| i.skill == skill)
    }

    /// The per-player roll-up for a PRI, if present.
    pub fn player(&self, pri: i32) -> Option<&PlayerSkills> {
        self.players.iter().find(|p| p.pri == pri)
    }

    /// The per-player roll-up for the first player with this name, if present.
    pub fn player_by_name(&self, name: &str) -> Option<&PlayerSkills> {
        self.players.iter().find(|p| p.player == name)
    }
}
