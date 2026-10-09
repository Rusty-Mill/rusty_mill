//! Linking detected skills to the value model's per-touch **ΔV** swing.
//!
//! [`crate::outcome`] links skills to *goals* — a sparse, binary "was there a
//! goal soon after" signal. This is the richer companion: a ball-contact skill
//! instance is credited the scoring-probability swing of the touch it rode on, so
//! a player's aerials, redirects, and flicks carry a signed "how much did it move
//! the needle" value — the continuous outcome the goal-conversion view only
//! approximates.
//!
//! Kept independent of the `replay-value` crate: the per-touch ΔV is fed in as
//! plain [`TouchDv`] records (pri, time, ΔV), so this stays a pure function of a
//! [`SkillReport`] plus numbers and is unit-testable without the value model.
//!
//! Only skills emitted *at a ball contact* (aerial, air dribble, wall play,
//! flick, power shot, redirect, kickoff first touch) line up with a touch and get
//! linked; run-based skills (supersonic, ceiling, ground dribble) and non-contact
//! events (boost steal, demo) have no touch within tolerance and stay unlinked.

use std::collections::BTreeMap;

use serde::{Deserialize, Serialize};

use crate::report::SkillReport;
use crate::skill::Skill;

/// A touch's value swing, as produced by the value model — passed in by value so
/// the skills crate needn't depend on `replay-value`'s types.
#[derive(Debug, Clone, Copy, PartialEq, Serialize, Deserialize)]
pub struct TouchDv {
    pub pri: i32,
    /// Match time of the touch (s).
    pub t: f32,
    /// Scoring-probability swing the touch caused (signed).
    pub dv: f32,
}

/// Per-player skill→value link: how much each skill moved scoring probability.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct SkillValue {
    pub pri: i32,
    pub player: String,
    pub team: Option<i32>,
    /// `skill -> (reps linked to a touch ΔV, summed ΔV)`, catalog-ordered.
    pub by_skill: BTreeMap<Skill, (usize, f32)>,
    /// Reps linked to a touch — the denominator behind the means.
    pub linked: usize,
    /// Total ΔV across this player's linked skill reps.
    pub sum_dv: f32,
}

impl SkillValue {
    /// Mean ΔV per linked rep across all skills (0 if none linked).
    pub fn mean_dv(&self) -> f32 {
        if self.linked > 0 {
            self.sum_dv / self.linked as f32
        } else {
            0.0
        }
    }

    /// Mean ΔV for one skill (0 if no rep of it was linked).
    pub fn mean_dv_for(&self, skill: Skill) -> f32 {
        match self.by_skill.get(&skill) {
            Some((n, s)) if *n > 0 => s / *n as f32,
            _ => 0.0,
        }
    }
}

/// Credit each skill instance with the ΔV of the same player's touch nearest in
/// time, within `tol_s`. Per-player results follow the report's player order; a
/// player with no linked reps is still emitted (with an empty `by_skill`).
pub fn skill_values(report: &SkillReport, touch_dv: &[TouchDv], tol_s: f32) -> Vec<SkillValue> {
    // Group touch ΔV by PRI for a quick nearest-in-time scan per instance.
    let mut by_pri: BTreeMap<i32, Vec<TouchDv>> = BTreeMap::new();
    for td in touch_dv {
        by_pri.entry(td.pri).or_default().push(*td);
    }

    // Per (pri, skill) accumulation, only for instances we can link to a touch.
    let mut acc: BTreeMap<i32, BTreeMap<Skill, (usize, f32)>> = BTreeMap::new();
    for i in &report.instances {
        // Only ball-contact skills *are* a touch; a run-start (supersonic, ground
        // dribble) or event (demo) merely coinciding with a touch isn't its swing.
        if !i.skill.is_ball_contact() {
            continue;
        }
        let Some(cands) = by_pri.get(&i.pri) else {
            continue;
        };
        let nearest = cands
            .iter()
            .filter(|td| (td.t - i.t).abs() <= tol_s)
            .min_by(|a, b| (a.t - i.t).abs().total_cmp(&(b.t - i.t).abs()));
        if let Some(td) = nearest {
            let e = acc
                .entry(i.pri)
                .or_default()
                .entry(i.skill)
                .or_insert((0, 0.0));
            e.0 += 1;
            e.1 += td.dv;
        }
    }

    report
        .players
        .iter()
        .map(|p| {
            let by_skill = acc.get(&p.pri).cloned().unwrap_or_default();
            let linked: usize = by_skill.values().map(|(n, _)| *n).sum();
            let sum_dv: f32 = by_skill.values().map(|(_, s)| *s).sum();
            SkillValue {
                pri: p.pri,
                player: p.player.clone(),
                team: p.team,
                by_skill,
                linked,
                sum_dv,
            }
        })
        .collect()
}
