//! Optional scoring-role overlay: tag each scene car with its 1st/2nd-man role,
//! and carry over the `support_spacing` target band for the distance tool.
//!
//! A thin bridge to `replay_scoring`: it assigns per-frame roles over the same
//! resampled grid the scene is built from (`build_frames` maps 1:1 over
//! `resampled.frames`), so the viewer can colour cars by role. Kept separate
//! from [`crate::scene`] so the core projection stays decoupled from scoring.

use std::collections::{BTreeMap, BTreeSet};

use replay_analyzer::model::CanonicalMatch;
use replay_scoring::config::{Curve, Metric};
use replay_scoring::{features, roles, ScoreConfig};

use crate::scene::Scene;

/// Fill each [`crate::scene::SceneCar::role`] (`1` = 1st man, `2` = 2nd man) from
/// `replay_scoring`'s per-frame role assignment, and attach the
/// [`support_spacing_band`](Scene::support_spacing_band) coaching reference the
/// viewer's distance tool uses — both are read off the same `ScoreConfig`
/// already threaded through here.
///
/// MUST run before any [`crate::scene::downsample`] — it matches scene frames to
/// scoring frames 1:1 by index, which only holds at the full grid rate. If the
/// counts disagree (already thinned), it skips rather than mis-tag.
pub fn attach_roles(scene: &mut Scene, m: &CanonicalMatch, cfg: &ScoreConfig) {
    scene.support_spacing_band = support_spacing_band(cfg);
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

/// The `support_spacing` metric's `[lo, hi]` band (uu) — how far apart teammates
/// should sit while one is 2nd man (not double-committing, not unreachable).
/// Read straight off the live config rather than duplicated as a magic number,
/// so a rubric recalibration (a `SCORE_CONFIG_VERSION` bump) updates the
/// viewer's coaching hint for free. `None` if the metric isn't configured with
/// a banded curve (e.g. a custom `ScoreConfig` that dropped it).
fn support_spacing_band(cfg: &ScoreConfig) -> Option<[f32; 2]> {
    cfg.metrics.iter().find_map(|spec| match spec.curve {
        Curve::Band { lo, hi, .. } if spec.metric == Metric::SupportSpacing => Some([lo, hi]),
        _ => None,
    })
}
