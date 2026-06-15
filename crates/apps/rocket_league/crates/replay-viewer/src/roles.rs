//! Optional scoring-role overlay: tag each scene car with its 1st/2nd-man role.
//!
//! A thin bridge to `replay_scoring`: it assigns per-frame roles over the same
//! resampled grid the scene is built from (`build_frames` maps 1:1 over
//! `resampled.frames`), so the viewer can colour cars by role. Kept separate
//! from [`crate::scene`] so the core projection stays decoupled from scoring.

use std::collections::{BTreeMap, BTreeSet};

use replay_analyzer::model::CanonicalMatch;
use replay_scoring::{features, roles, ScoreConfig};

use crate::scene::Scene;

/// Fill each [`crate::scene::SceneCar::role`] (`1` = 1st man, `2` = 2nd man) from
/// `replay_scoring`'s per-frame role assignment.
///
/// MUST run before any [`crate::scene::downsample`] — it matches scene frames to
/// scoring frames 1:1 by index, which only holds at the full grid rate. If the
/// counts disagree (already thinned), it skips rather than mis-tag.
pub fn attach_roles(scene: &mut Scene, m: &CanonicalMatch, cfg: &ScoreConfig) {
    let frames = features::build_frames(m, cfg);
    if frames.len() != scene.frames.len() {
        return;
    }
    let teams: Vec<i32> = m
        .tracks
        .iter()
        .filter_map(|t| t.team)
        .collect::<BTreeSet<_>>()
        .into_iter()
        .collect();
    let assigned = roles::assign(&frames, &teams, cfg);
    let pri_team: BTreeMap<i32, i32> = scene
        .players
        .iter()
        .filter_map(|p| p.team.map(|t| (p.pri, t)))
        .collect();

    for (i, sf) in scene.frames.iter_mut().enumerate() {
        for car in &mut sf.cars {
            let Some(&team) = pri_team.get(&car.pri) else {
                continue;
            };
            car.role = Some(if assigned.first(i, team) == Some(car.pri) {
                1
            } else {
                2
            });
        }
    }
}
