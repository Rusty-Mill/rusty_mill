//! Per-player derived feature aggregates (T5).
//!
//! A first, deliberately small set of scoring-agnostic summary stats computed
//! from the reconstructed/resampled data and derived events: boost used, time
//! supersonic, touch count, possession time, and mean distance to the ball.
//! These are sample features (a downstream consumer's summary), not the scoring
//! rubric — no judgment, just kinematics reduced to numbers.

use crate::field;
use crate::model::{Event, PlayerFeatures, PlayerTrack, Resampled};
use std::collections::BTreeMap;

/// Compute per-player features from the resampled grid, coalesced tracks, and
/// derived events. One [`PlayerFeatures`] per track, sorted by team then PRI.
pub fn player_features(
    resampled: &Resampled,
    tracks: &[PlayerTrack],
    events: &[Event],
) -> Vec<PlayerFeatures> {
    let dt = if resampled.hz > 0.0 {
        1.0 / resampled.hz
    } else {
        0.0
    };

    // Touches per player.
    let mut touches: BTreeMap<i32, usize> = BTreeMap::new();
    // Team possession time.
    let mut team_possession: BTreeMap<i32, f32> = BTreeMap::new();
    for e in events {
        match e {
            Event::Touch { pri, .. } => *touches.entry(*pri).or_default() += 1,
            Event::Possession {
                team, start, end, ..
            } => {
                *team_possession.entry(*team).or_default() += end - start;
            }
            _ => {}
        }
    }

    // Single grid pass: supersonic time and mean distance-to-ball per car.
    let mut supersonic_frames: BTreeMap<i32, u64> = BTreeMap::new();
    let mut dist_sum: BTreeMap<i32, f64> = BTreeMap::new();
    let mut dist_cnt: BTreeMap<i32, u64> = BTreeMap::new();
    for f in &resampled.frames {
        for c in &f.cars {
            let v = c.v;
            let speed = (v.x * v.x + v.y * v.y + v.z * v.z).sqrt();
            if speed >= field::SUPERSONIC_SPEED {
                *supersonic_frames.entry(c.pri).or_default() += 1;
            }
            if let Some(ball) = &f.ball {
                let (dx, dy, dz) = (c.p.x - ball.p.x, c.p.y - ball.p.y, c.p.z - ball.p.z);
                *dist_sum.entry(c.pri).or_default() +=
                    ((dx * dx + dy * dy + dz * dz) as f64).sqrt();
                *dist_cnt.entry(c.pri).or_default() += 1;
            }
        }
    }

    let mut out: Vec<PlayerFeatures> = tracks
        .iter()
        .map(|t| {
            let cnt = dist_cnt.get(&t.pri).copied().unwrap_or(0);
            let mean_dist_to_ball = if cnt > 0 {
                (dist_sum.get(&t.pri).copied().unwrap_or(0.0) / cnt as f64) as f32
            } else {
                0.0
            };
            PlayerFeatures {
                pri: t.pri,
                player: t.player.clone(),
                team: t.team,
                touches: touches.get(&t.pri).copied().unwrap_or(0),
                boost_used: boost_used(t),
                time_supersonic_s: supersonic_frames.get(&t.pri).copied().unwrap_or(0) as f32 * dt,
                mean_dist_to_ball,
                possession_time_s: t
                    .team
                    .and_then(|tm| team_possession.get(&tm).copied())
                    .unwrap_or(0.0),
            }
        })
        .collect();

    out.sort_by(|a, b| a.team.cmp(&b.team).then(a.pri.cmp(&b.pri)));
    out
}

/// Total boost consumed over a track, in boost units (0–100 scale).
///
/// Sums only *decreases* in boost between consecutive samples within the same
/// car life (pickups are not "used"); deltas that straddle a respawn gap are
/// skipped, since a fresh car spawns with its own boost.
fn boost_used(track: &PlayerTrack) -> f32 {
    let mut used_bytes: f32 = 0.0;
    for pair in track.samples.windows(2) {
        let (prev, cur) = (&pair[0], &pair[1]);
        // Skip a delta that brackets a respawn gap.
        let across_gap = track
            .gaps
            .iter()
            .any(|g| prev.t <= g.start && cur.t >= g.end);
        if across_gap {
            continue;
        }
        if let (Some(pb), Some(cb)) = (prev.boost, cur.boost) {
            if pb > cb {
                used_bytes += (pb - cb) as f32;
            }
        }
    }
    // Bytes (0..255 scale) -> boost units (0..100 scale).
    used_bytes / (field::BOOST_MAX_BYTE as f32) * 100.0
}
