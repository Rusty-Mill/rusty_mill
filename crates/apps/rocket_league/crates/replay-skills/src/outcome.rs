//! Tying skills to **outcomes**: which reps preceded a goal.
//!
//! A first, honest outcome link: for each skill instance, did the player's team
//! score within a window after it? Goals are sparse (a handful per match), so
//! conversion counts are small by nature — read this as "buildup involvement",
//! not a per-skill success rate. A richer link (the value model's per-touch ΔV
//! swing) is a follow-up; this needs only the match's goal events.

use std::collections::BTreeMap;

use replay_analyzer::model::Event;
use serde::{Deserialize, Serialize};

use crate::report::SkillReport;
use crate::skill::Skill;

/// Per-player skill→goal involvement within a window.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct SkillOutcome {
    pub pri: i32,
    pub player: String,
    pub team: Option<i32>,
    /// Per skill: `(total instances, instances within the window before a
    /// same-team goal)`.
    pub by_skill: BTreeMap<Skill, (usize, usize)>,
    pub total: usize,
    /// Total instances that preceded a same-team goal within the window.
    pub total_before_goal: usize,
}

/// Link each player's skills to same-team goals: count, per skill, how many reps
/// occurred within `window_s` seconds *before* a goal by that player's team.
pub fn outcomes(report: &SkillReport, events: &[Event], window_s: f32) -> Vec<SkillOutcome> {
    let goals: Vec<(f32, i32)> = events
        .iter()
        .filter_map(|e| match e {
            Event::Goal {
                t,
                team: Some(team),
                ..
            } => Some((*t, *team)),
            _ => None,
        })
        .collect();

    let before_goal = |t: f32, team: Option<i32>| -> bool {
        let Some(team) = team else { return false };
        goals
            .iter()
            .any(|&(gt, gteam)| gteam == team && gt > t && gt <= t + window_s)
    };

    let mut by_pri: BTreeMap<i32, BTreeMap<Skill, (usize, usize)>> = BTreeMap::new();
    for i in &report.instances {
        let e = by_pri
            .entry(i.pri)
            .or_default()
            .entry(i.skill)
            .or_insert((0, 0));
        e.0 += 1;
        if before_goal(i.t, i.team) {
            e.1 += 1;
        }
    }

    report
        .players
        .iter()
        .map(|p| {
            let by_skill = by_pri.get(&p.pri).cloned().unwrap_or_default();
            let total = by_skill.values().map(|&(t, _)| t).sum();
            let total_before_goal = by_skill.values().map(|&(_, c)| c).sum();
            SkillOutcome {
                pri: p.pri,
                player: p.player.clone(),
                team: p.team,
                by_skill,
                total,
                total_before_goal,
            }
        })
        .collect()
}
