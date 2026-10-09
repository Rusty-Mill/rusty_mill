//! Per-player skill **proficiency profiles**.
//!
//! [`SkillReport`] answers *whether* and *how many times* a skill was performed.
//! This module adds the next layer — *how good / how often* — by turning the
//! per-player roll-up into rates (per minute) and two per-skill quality signals,
//! so two players with the same aerial count are distinguishable by how high and
//! how frequent their aerials are.
//!
//! Each [`SkillStat`] carries:
//! - `mean_metric` — the mean of the skill's **structured evidence magnitude** in
//!   its natural unit (aerial peak height in uu, dribble duration in s, power-shot
//!   ball speed in uu/s, …), straight off each instance's `metric`. This is the
//!   real physical quantity, the honest answer to "how strong were the reps".
//! - `mean_quality` — the mean detection **confidence** (`0.0..=1.0`), a
//!   normalized certainty that ramps with magnitude but saturates; a coarser
//!   companion to `mean_metric`.
//!
//! Both are proxies, not a coached grade — see the spec's §0 caveat that these
//! are kinematic inferences.

use std::collections::BTreeMap;

use serde::{Deserialize, Serialize};

use crate::report::SkillReport;
use crate::skill::Skill;

/// Proficiency for one skill: how often, and how strong each rep was.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct SkillStat {
    pub count: usize,
    /// Occurrences per minute of live play (see `CanonicalMatch::live_time_s`).
    pub per_min: f32,
    /// Mean detection confidence (`0.0..=1.0`) for this player's reps of the
    /// skill — a normalized certainty proxy.
    pub mean_quality: f32,
    /// Mean of the skill's **structured evidence magnitude** over this player's
    /// reps, in its natural unit (see [`Skill::metric_label`] /
    /// [`Skill::metric_unit`]): e.g. mean aerial peak height in uu, mean dribble
    /// duration in s. The real physical quantity rather than a normalized proxy,
    /// so two players with equal aerial counts are separable by how high they go.
    pub mean_metric: f32,
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

    // Sum/count of confidence *and* structured metric per (player, skill) from
    // the flat instance list, for the two per-skill means.
    let mut sum: BTreeMap<(i32, Skill), f32> = BTreeMap::new();
    let mut sum_metric: BTreeMap<(i32, Skill), f32> = BTreeMap::new();
    let mut cnt: BTreeMap<(i32, Skill), usize> = BTreeMap::new();
    for i in &report.instances {
        *sum.entry((i.pri, i.skill)).or_default() += i.confidence;
        *sum_metric.entry((i.pri, i.skill)).or_default() += i.metric;
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
                    let (mean_quality, mean_metric) = if c > 0 {
                        let n = c as f32;
                        (sum[&(p.pri, skill)] / n, sum_metric[&(p.pri, skill)] / n)
                    } else {
                        (0.0, 0.0)
                    };
                    (
                        skill,
                        SkillStat {
                            count,
                            per_min: round3(count as f32 / mins),
                            mean_quality: round3(mean_quality),
                            mean_metric: round3(mean_metric),
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
