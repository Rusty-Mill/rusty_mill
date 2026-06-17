//! **Ballchasing-parity stat aggregates** (spec §13 / `docs/ballchasing-parity.md`).
//!
//! A pure reducer over the canonical match — like `scoring`/`skills`/`value`, it
//! consumes [`CanonicalMatch`] and returns a ballchasing-shaped per-player stat
//! block without mutating the model (so the canonical golden is untouched). It
//! reduces the existing 30 Hz resample grid + tracks + events into the boost /
//! movement / positioning / demo aggregates ballchasing exposes, closing most of
//! the gap our 5-field [`crate::model::PlayerFeatures`] left open.
//!
//! These are time-uniform reductions of a **sampled** reconstruction, so — as
//! ballchasing itself notes for its own sampling — treat them as accurate to a
//! couple of percent, not frame-exact. Height-band air/ground and the speed and
//! boost buckets use the documented thresholds below; positioning is computed in
//! each team's attack-direction frame (every team attacks `+Y`).
//!
//! Not yet covered (tracked in `docs/ballchasing-parity.md`): boost-pad pickup
//! attribution (`amount_stolen`/`big`/`small`/overfill — needs a pad model),
//! possession-split distance-to-ball, and most-back/most-forward/last-defender
//! (needs per-team ordering). `amount_collected`/`amount_used` here are net
//! boost-gauge integrals, not pad pickups.

use crate::field;
use crate::model::{CanonicalMatch, Event, GridFrame, PlayerTrack, Vec3};
use serde::{Deserialize, Serialize};
use std::collections::BTreeMap;

/// Floor band: a car centre below this height counts as on the ground (uu).
/// Approximate — a car driving on a wall reads as "air" here, the same caveat
/// ballchasing's height sampling carries.
const GROUND_Z: f32 = 50.0;
/// Low-air / high-air split (uu): roughly goal-height.
const HIGH_AIR_Z: f32 = 600.0;
/// Boost gauge is "full" at this percent (raw byte 255).
const FULL_BOOST_PCT: f32 = 100.0;

/// Per-player ballchasing-shaped stat block.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct BcPlayerStats {
    pub pri: i32,
    pub player: String,
    pub team: Option<i32>,
    pub boost: BcBoost,
    pub movement: BcMovement,
    pub positioning: BcPositioning,
    pub demo: BcDemo,
}

/// Boost economy. `amount_collected`/`amount_used` are net gauge integrals (sum
/// of positive / negative boost-gauge deltas), in boost units (0–100 scale); they
/// are **not** pad pickups (see the module note). `bpm`/`bcpm` are those per
/// minute of the player's tracked time.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct BcBoost {
    pub avg_amount: f32,
    pub amount_collected: f32,
    pub amount_used: f32,
    pub bpm: f32,
    pub bcpm: f32,
    pub time_zero_s: f32,
    pub percent_zero: f32,
    pub time_full_s: f32,
    pub percent_full: f32,
    /// Time with the boost gauge in each quartile (`[0,25) [25,50) [50,75)
    /// [75,100]`). The four sum to the tracked boost time; `zero`/`full` above
    /// are overlapping sub-counters (zero ⊂ 0–25, full ⊂ 75–100).
    pub time_0_25_s: f32,
    pub time_25_50_s: f32,
    pub time_50_75_s: f32,
    pub time_75_100_s: f32,
    pub percent_0_25: f32,
    pub percent_25_50: f32,
    pub percent_50_75: f32,
    pub percent_75_100: f32,
}

/// Speed / air distribution.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct BcMovement {
    pub avg_speed: f32,
    pub total_distance: f32,
    pub time_slow_s: f32,
    pub percent_slow: f32,
    pub time_boost_speed_s: f32,
    pub percent_boost_speed: f32,
    pub time_supersonic_s: f32,
    pub percent_supersonic: f32,
    pub time_ground_s: f32,
    pub percent_ground: f32,
    pub time_low_air_s: f32,
    pub percent_low_air: f32,
    pub time_high_air_s: f32,
    pub percent_high_air: f32,
}

/// Field occupancy, in the team's attack-direction frame (`+Y` = attacking).
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct BcPositioning {
    pub avg_dist_to_ball: f32,
    pub avg_dist_to_mates: f32,
    pub time_defensive_third_s: f32,
    pub percent_defensive_third: f32,
    pub time_neutral_third_s: f32,
    pub percent_neutral_third: f32,
    pub time_offensive_third_s: f32,
    pub percent_offensive_third: f32,
    pub time_defensive_half_s: f32,
    pub percent_defensive_half: f32,
    pub time_offensive_half_s: f32,
    pub percent_offensive_half: f32,
    pub time_behind_ball_s: f32,
    pub percent_behind_ball: f32,
    pub time_infront_ball_s: f32,
    pub percent_infront_ball: f32,
}

/// Demolitions (from authoritative demo events).
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct BcDemo {
    pub inflicted: u32,
    pub taken: u32,
}

/// Per-pri running accumulator over the grid pass.
#[derive(Default)]
struct Acc {
    present: u64,
    speed_sum: f64,
    slow: u64,
    boost_spd: u64,
    super_spd: u64,
    ground: u64,
    low_air: u64,
    high_air: u64,
    // boost
    boost_frames: u64,
    boost_pct_sum: f64,
    b_zero: u64,
    b_full: u64,
    b_q: [u64; 4],
    // distances
    ball_frames: u64,
    ball_dist_sum: f64,
    mate_pairs: u64,
    mate_dist_sum: f64,
    // positioning (team known)
    pos_frames: u64,
    def3: u64,
    neu3: u64,
    off3: u64,
    def_half: u64,
    off_half: u64,
    // behind/infront (team known AND ball present)
    behind: u64,
    infront: u64,
}

fn speed(v: Vec3) -> f32 {
    (v.x * v.x + v.y * v.y + v.z * v.z).sqrt()
}

fn dist(a: Vec3, b: Vec3) -> f64 {
    let (dx, dy, dz) = (a.x - b.x, a.y - b.y, a.z - b.z);
    ((dx * dx + dy * dy + dz * dz) as f64).sqrt()
}

/// Reduce one grid frame into the per-pri accumulators.
fn fold_frame(frame: &GridFrame, signs: &BTreeMap<i32, i32>, accs: &mut BTreeMap<i32, Acc>) {
    let third_y = field::BACK_WALL_Y / 3.0;
    for c in &frame.cars {
        let a = accs.entry(c.pri).or_default();
        a.present += 1;

        let s = speed(c.v);
        a.speed_sum += s as f64;
        if s >= field::SUPERSONIC_SPEED {
            a.super_spd += 1;
        } else if s >= field::BOOST_SPEED {
            a.boost_spd += 1;
        } else {
            a.slow += 1;
        }

        let z = c.p.z;
        if z < GROUND_Z {
            a.ground += 1;
        } else if z < HIGH_AIR_Z {
            a.low_air += 1;
        } else {
            a.high_air += 1;
        }

        if let Some(b) = c.boost {
            a.boost_frames += 1;
            let pct = field::boost_percent(b);
            a.boost_pct_sum += pct as f64;
            if b == 0 {
                a.b_zero += 1;
            }
            if pct >= FULL_BOOST_PCT {
                a.b_full += 1;
            }
            let q = if pct < 25.0 {
                0
            } else if pct < 50.0 {
                1
            } else if pct < 75.0 {
                2
            } else {
                3
            };
            a.b_q[q] += 1;
        }

        if let Some(ball) = &frame.ball {
            a.ball_frames += 1;
            a.ball_dist_sum += dist(c.p, ball.p);
        }

        // Positioning in the car's own attack frame (+Y attacking).
        if let Some(team) = c.team {
            let sign = signs.get(&team).copied().unwrap_or(1);
            let ny = if sign >= 0 { c.p.y } else { -c.p.y };
            a.pos_frames += 1;
            if ny < -third_y {
                a.def3 += 1;
            } else if ny <= third_y {
                a.neu3 += 1;
            } else {
                a.off3 += 1;
            }
            if ny < 0.0 {
                a.def_half += 1;
            } else {
                a.off_half += 1;
            }
            if let Some(ball) = &frame.ball {
                let nby = if sign >= 0 { ball.p.y } else { -ball.p.y };
                if ny < nby {
                    a.behind += 1;
                } else {
                    a.infront += 1;
                }
            }
        }
    }

    // Distance to mates: every ordered same-team pair present this frame.
    for c in &frame.cars {
        let mut sum = 0.0f64;
        let mut n = 0u64;
        for o in &frame.cars {
            if o.pri != c.pri && o.team.is_some() && o.team == c.team {
                sum += dist(c.p, o.p);
                n += 1;
            }
        }
        if n > 0 {
            let a = accs.entry(c.pri).or_default();
            a.mate_dist_sum += sum;
            a.mate_pairs += n;
        }
    }
}

/// Net boost-gauge `(collected, used)` over a track, in boost units (0–100),
/// summing positive / negative deltas within a car life (skipping respawn gaps).
fn boost_flow(track: &PlayerTrack) -> (f32, f32) {
    let (mut up, mut down) = (0.0f32, 0.0f32);
    for pair in track.samples.windows(2) {
        let (prev, cur) = (&pair[0], &pair[1]);
        let across_gap = track
            .gaps
            .iter()
            .any(|g| prev.t <= g.start && cur.t >= g.end);
        if across_gap {
            continue;
        }
        if let (Some(pb), Some(cb)) = (prev.boost, cur.boost) {
            if cb > pb {
                up += (cb - pb) as f32;
            } else {
                down += (pb - cb) as f32;
            }
        }
    }
    let scale = field::BOOST_MAX_BYTE as f32 / 100.0;
    (up / scale, down / scale)
}

/// Compute ballchasing-shaped per-player stats from the canonical match. One
/// [`BcPlayerStats`] per track, sorted by team then PRI (matching
/// [`crate::analyze::features::player_features`]).
pub fn ballchasing_stats(m: &CanonicalMatch) -> Vec<BcPlayerStats> {
    let dt = if m.resampled.hz > 0.0 {
        1.0 / m.resampled.hz
    } else {
        0.0
    };
    let signs = &m.resampled.team_attack_sign;

    let mut accs: BTreeMap<i32, Acc> = BTreeMap::new();
    for f in &m.resampled.frames {
        fold_frame(f, signs, &mut accs);
    }

    // Demo tallies from authoritative events.
    let mut demo_in: BTreeMap<i32, u32> = BTreeMap::new();
    let mut demo_taken: BTreeMap<i32, u32> = BTreeMap::new();
    for e in &m.events {
        if let Event::Demo {
            attacker_pri,
            victim_pri,
            ..
        } = e
        {
            if let Some(p) = attacker_pri {
                *demo_in.entry(*p).or_default() += 1;
            }
            if let Some(p) = victim_pri {
                *demo_taken.entry(*p).or_default() += 1;
            }
        }
    }

    let pct = |n: u64, d: u64| {
        if d > 0 {
            n as f32 / d as f32 * 100.0
        } else {
            0.0
        }
    };
    let secs = |n: u64| n as f32 * dt;
    let mean = |sum: f64, n: u64| if n > 0 { (sum / n as f64) as f32 } else { 0.0 };

    let mut out: Vec<BcPlayerStats> = m
        .tracks
        .iter()
        .map(|t| {
            let a = accs.remove(&t.pri).unwrap_or_default();
            let (collected, used) = boost_flow(t);
            let minutes = secs(a.present) / 60.0;
            let per_min = |amt: f32| if minutes > 0.0 { amt / minutes } else { 0.0 };
            let behind_den = a.behind + a.infront;

            BcPlayerStats {
                pri: t.pri,
                player: t.player.clone(),
                team: t.team,
                boost: BcBoost {
                    avg_amount: mean(a.boost_pct_sum, a.boost_frames),
                    amount_collected: collected,
                    amount_used: used,
                    bpm: per_min(collected),
                    bcpm: per_min(used),
                    time_zero_s: secs(a.b_zero),
                    percent_zero: pct(a.b_zero, a.boost_frames),
                    time_full_s: secs(a.b_full),
                    percent_full: pct(a.b_full, a.boost_frames),
                    time_0_25_s: secs(a.b_q[0]),
                    time_25_50_s: secs(a.b_q[1]),
                    time_50_75_s: secs(a.b_q[2]),
                    time_75_100_s: secs(a.b_q[3]),
                    percent_0_25: pct(a.b_q[0], a.boost_frames),
                    percent_25_50: pct(a.b_q[1], a.boost_frames),
                    percent_50_75: pct(a.b_q[2], a.boost_frames),
                    percent_75_100: pct(a.b_q[3], a.boost_frames),
                },
                movement: BcMovement {
                    avg_speed: mean(a.speed_sum, a.present),
                    total_distance: (a.speed_sum * dt as f64) as f32,
                    time_slow_s: secs(a.slow),
                    percent_slow: pct(a.slow, a.present),
                    time_boost_speed_s: secs(a.boost_spd),
                    percent_boost_speed: pct(a.boost_spd, a.present),
                    time_supersonic_s: secs(a.super_spd),
                    percent_supersonic: pct(a.super_spd, a.present),
                    time_ground_s: secs(a.ground),
                    percent_ground: pct(a.ground, a.present),
                    time_low_air_s: secs(a.low_air),
                    percent_low_air: pct(a.low_air, a.present),
                    time_high_air_s: secs(a.high_air),
                    percent_high_air: pct(a.high_air, a.present),
                },
                positioning: BcPositioning {
                    avg_dist_to_ball: mean(a.ball_dist_sum, a.ball_frames),
                    avg_dist_to_mates: mean(a.mate_dist_sum, a.mate_pairs),
                    time_defensive_third_s: secs(a.def3),
                    percent_defensive_third: pct(a.def3, a.pos_frames),
                    time_neutral_third_s: secs(a.neu3),
                    percent_neutral_third: pct(a.neu3, a.pos_frames),
                    time_offensive_third_s: secs(a.off3),
                    percent_offensive_third: pct(a.off3, a.pos_frames),
                    time_defensive_half_s: secs(a.def_half),
                    percent_defensive_half: pct(a.def_half, a.pos_frames),
                    time_offensive_half_s: secs(a.off_half),
                    percent_offensive_half: pct(a.off_half, a.pos_frames),
                    time_behind_ball_s: secs(a.behind),
                    percent_behind_ball: pct(a.behind, behind_den),
                    time_infront_ball_s: secs(a.infront),
                    percent_infront_ball: pct(a.infront, behind_den),
                },
                demo: BcDemo {
                    inflicted: demo_in.get(&t.pri).copied().unwrap_or(0),
                    taken: demo_taken.get(&t.pri).copied().unwrap_or(0),
                },
            }
        })
        .collect();

    out.sort_by(|a, b| a.team.cmp(&b.team).then(a.pri.cmp(&b.pri)));
    out
}
