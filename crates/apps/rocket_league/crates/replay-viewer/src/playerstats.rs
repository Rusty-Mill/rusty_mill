//! Optional per-player analysis aggregates for the viewer's stat strips.
//!
//! A thin bridge to [`replay_analyzer::analyze::bcstats`]: it runs the
//! ballchasing-shaped reducer over the match and keeps just the few channels the
//! viewer's strips draw (speed buckets, thirds occupancy, most-back share). Kept
//! separate from [`crate::scene`] so the core projection stays decoupled, mirroring
//! [`crate::winprob`] / [`crate::impact`].

use replay_analyzer::analyze::bcstats::ballchasing_stats;
use replay_analyzer::model::CanonicalMatch;

use crate::scene::{Scene, ScenePlayerStat};

fn round1(x: f32) -> f32 {
    (x * 10.0).round() / 10.0
}

/// Fill [`Scene::player_stats`] with each player's movement/positioning split.
/// No-op shape is an empty vec (the viewer simply omits the strips then).
pub fn attach_player_stats(scene: &mut Scene, m: &CanonicalMatch) {
    scene.player_stats = ballchasing_stats(m)
        .into_iter()
        .map(|s| ScenePlayerStat {
            pri: s.pri,
            speed: [
                round1(s.movement.percent_slow),
                round1(s.movement.percent_boost_speed),
                round1(s.movement.percent_supersonic),
            ],
            thirds: [
                round1(s.positioning.percent_defensive_third),
                round1(s.positioning.percent_neutral_third),
                round1(s.positioning.percent_offensive_third),
            ],
            most_back: round1(s.positioning.percent_most_back),
        })
        .collect();
}
