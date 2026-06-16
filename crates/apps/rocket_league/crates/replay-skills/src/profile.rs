//! Per-player skill **proficiency profiles**.
//!
//! [`SkillReport`] answers *whether* and *how many times* a skill was performed.
//! This module adds the next layer — *how good / how often* — by turning the
//! per-player roll-up into rates (per minute) and a quality proxy, so two players
//! with the same aerial count are distinguishable by how clean and how frequent
//! their aerials are.
//!
//! Quality is the mean detection **confidence**: detectors whose confidence ramps
//! with magnitude (aerial height, dribble duration, redirect angle, power-shot
//! speed, …) yield a higher mean for more pronounced reps; event-sourced skills
//! (demo) sit at 1.0. It's a proxy, not a coached grade — see the spec's §0
//! caveat that these are kinematic inferences.

use std::collections::BTreeMap;

use serde::{Deserialize, Serialize};

use crate::report::SkillReport;
use crate::skill::Skill;

/// Proficiency for one skill: how often, and a quality proxy.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct SkillStat {
    pub count: usize,
    /// Occurrences per minute of match time.
    pub per_min: f32,
    /// Mean detection confidence (`0.0..=1.0`) for this player's reps of the
    /// skill — a quality proxy.
    pub mean_quality: f32,
}

/// A player's mechanical profile across the catalogued skills.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct PlayerSkillProfile {
    pub pri: i32,
    pub player: String,
    pub team: Option<i32>,
    /// Per-skill proficiency, only for skills performed at least once
    /// (catalog-ordered).
    pub skills: BTreeMap<Skill, SkillStat>,
    /// Total catalogued skills per minute — overall mechanical activity.
    pub total_per_min: f32,
}

impl PlayerSkillProfile {
    /// Proficiency for one skill, if the player performed it.
    pub fn stat(&self, skill: Skill) -> Option<&SkillStat> {
        self.skills.get(&skill)
    }
}

fn round3(x: f32) -> f32 {
    (x * 1000.0).round() / 1000.0
}

/// Build per-player proficiency profiles from a [`SkillReport`] and the match
/// duration (seconds). Rates are per minute; quality is the mean detection
/// confidence over that player's reps.
pub fn profiles(report: &SkillReport, duration_s: f32) -> Vec<PlayerSkillProfile> {
    let mins = (duration_s / 60.0).max(1e-6);

    // Sum/count of confidence per (player, skill) from the flat instance list.
    let mut sum: BTreeMap<(i32, Skill), f32> = BTreeMap::new();
    let mut cnt: BTreeMap<(i32, Skill), usize> = BTreeMap::new();
    for i in &report.instances {
        *sum.entry((i.pri, i.skill)).or_default() += i.confidence;
        *cnt.entry((i.pri, i.skill)).or_default() += 1;
    }

    report
        .players
        .iter()
        .map(|p| {
            let skills = p
                .counts
                .iter()
                .map(|(&skill, &count)| {
                    let c = cnt.get(&(p.pri, skill)).copied().unwrap_or(0);
                    let mean_quality = if c > 0 {
                        sum[&(p.pri, skill)] / c as f32
                    } else {
                        0.0
                    };
                    (
                        skill,
                        SkillStat {
                            count,
                            per_min: round3(count as f32 / mins),
                            mean_quality: round3(mean_quality),
                        },
                    )
                })
                .collect();
            let total: usize = p.counts.values().sum();
            PlayerSkillProfile {
                pri: p.pri,
                player: p.player.clone(),
                team: p.team,
                skills,
                total_per_min: round3(total as f32 / mins),
            }
        })
        .collect()
}
