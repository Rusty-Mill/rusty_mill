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
//! Boost-pad attribution (collected/big/small/stolen/overfill) uses the
//! **gauge-delta** model in [`crate::analyze::boost_pads`] — the sum of positive
//! boost-gauge steps (above the jitter floor), classified by step size. This
//! matches ballchasing within ~1-3% on collected and ±1-2 on the big/small split
//! (`docs/ballchasing-comparison.md`), because ballchasing's `amount_collected`
//! and BPM are gauge-based. The authoritative `TAGame.VehiclePickup_TA` event
//! stream on [`CanonicalMatch::pickups`] is deliberately **not** the stat source
//! (it over-counts no-gain drive-overs / pad resets); it remains available as a
//! raw, player-attributed pickup-timing stream for the viewer. Powerslide usage
//! (`time_powerslide`/`count_powerslide`/`avg_powerslide_duration`) comes from the
//! authoritative handbrake intervals on [`CanonicalMatch::powerslides`].
//! Positioning covers
//! possession-split distance-to-ball, most-back/most-forward,
//! goals-against-while-last-defender, and `time_ball_in_side` (a team/ball stat
//! denormalized onto each player). Not yet covered (tracked in
//! `docs/ballchasing-parity.md`): per-pad boost heatmaps.

use crate::field;
use crate::model::{CanonicalMatch, Event, GridFrame, Vec3};
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
/// Below this ground speed (uu/s) a car is treated as parked, so neither
/// forward nor reverse "driving" is counted (kills orientation jitter at rest).
const DRIVE_MIN_SPEED: f32 = 150.0;
/// Facing cone half-angle: a car is "facing the ball" when the angle between its
/// forward axis (yaw) and the horizontal vector to the ball is within this. Stored
/// as the cosine (≈ cos 35°) for a cheap dot-product test.
const FACING_CONE_COS: f32 = 0.819;

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
    /// Boost burned while supersonic on the ground (ballchasing's
    /// `amount_used_while_supersonic`). **Approximate** — see
    /// [`crate::analyze::boost_pads::BoostEconomy::used_supersonic`].
    pub amount_used_while_supersonic: f32,
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
    /// Pad-pickup breakdown (from [`crate::analyze::boost_pads`]). `collected`
    /// here equals `amount_collected` above (both sum the same boost-gauge gains),
    /// now split big/small and by "stolen" (collected in the opponent's half).
    pub amount_collected_big: f32,
    pub amount_collected_small: f32,
    pub amount_stolen: f32,
    pub amount_stolen_big: f32,
    pub amount_stolen_small: f32,
    pub count_collected_big: u32,
    pub count_collected_small: u32,
    pub count_stolen_big: u32,
    pub count_stolen_small: u32,
    pub amount_overfill: f32,
    pub amount_overfill_stolen: f32,
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
    /// Powerslide (handbrake) usage, from the authoritative replicated handbrake
    /// boolean (`CanonicalMatch::powerslides`). Zero when the replay carried no
    /// handbrake stream (e.g. a hand-built canonical fixture).
    pub time_powerslide_s: f32,
    pub count_powerslide: u32,
    pub avg_powerslide_duration_s: f32,
    /// Time driven **in reverse** (s) and its share of driving time: grounded,
    /// moving above [`DRIVE_MIN_SPEED`], with the horizontal velocity pointing
    /// *behind* the car's forward axis (yaw). Derived from per-frame orientation —
    /// not a ballchasing stat. `percent_reverse` is over grounded driving frames
    /// (forward + reverse), so it reads as "of the time you were driving, how much
    /// was backwards". Zero when the replay carried no orientation.
    pub time_reverse_s: f32,
    pub percent_reverse: f32,
}

/// Field occupancy, in the team's attack-direction frame (`+Y` = attacking).
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct BcPositioning {
    pub avg_dist_to_ball: f32,
    /// Mean distance to ball while the player's team has possession / does not
    /// (split by the possession run containing each frame; ambiguous frames are
    /// in neither).
    pub avg_dist_to_ball_possession: f32,
    pub avg_dist_to_ball_no_possession: f32,
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
    /// Time the ball spent in this team's defensive half (a team/ball stat,
    /// denormalized onto each player; over the player's present-with-ball frames).
    pub time_ball_in_side_s: f32,
    pub percent_ball_in_side: f32,
    /// Time as the back-most / forward-most player on the team (by attack-frame
    /// `y`); the back-most is the "last defender".
    pub time_most_back_s: f32,
    pub percent_most_back: f32,
    pub time_most_forward_s: f32,
    pub percent_most_forward: f32,
    /// Time as the closest / farthest player of the team to the ball (ball
    /// present), as seconds and as a percent of the player's positional time.
    pub time_closest_to_ball_s: f32,
    pub percent_closest_to_ball: f32,
    pub time_farthest_from_ball_s: f32,
    pub percent_farthest_from_ball: f32,
    /// Goals conceded while this player was the team's last defender (back-most).
    pub goals_against_while_last_defender: u32,
    /// Mean angle (degrees, 0 = dead-on) between the car's forward axis and the
    /// horizontal direction to the ball, and the share of ball-present frames the
    /// car was facing the ball within [`FACING_CONE_COS`]. Derived from per-frame
    /// orientation — not a ballchasing stat. Zero when the replay carried no
    /// orientation.
    pub avg_facing_ball_deg: f32,
    pub percent_facing_ball: f32,
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
    // reverse driving (grounded + moving; needs orientation)
    drive: u64,
    reverse: u64,
    // facing the ball (ball present + orientation)
    facing_frames: u64,
    facing_deg_sum: f64,
    facing_cone: u64,
    // boost
    boost_frames: u64,
    boost_pct_sum: f64,
    b_zero: u64,
    b_full: u64,
    b_q: [u64; 4],
    // distances
    ball_frames: u64,
    ball_dist_sum: f64,
    // ball distance split by team possession (ball present AND possession known)
    gp_frames: u64,
    gp_dist_sum: f64,
    gnp_frames: u64,
    gnp_dist_sum: f64,
    mate_pairs: u64,
    mate_dist_sum: f64,
    // positioning (team known)
    pos_frames: u64,
    def3: u64,
    neu3: u64,
    off3: u64,
    def_half: u64,
    off_half: u64,
    most_back: u64,
    most_forward: u64,
    // closest / farthest to the ball among present teammates (ball present)
    closest_ball: u64,
    farthest_ball: u64,
    // behind/infront (team known AND ball present)
    behind: u64,
    infront: u64,
    // ball in this team's defensive half (team known AND ball present)
    in_side: u64,
}

fn speed(v: Vec3) -> f32 {
    (v.x * v.x + v.y * v.y + v.z * v.z).sqrt()
}

/// Unit forward axis of a car on the ground plane, from its yaw (Rocket League
/// forward is `(cos yaw, sin yaw)`). Pitch/roll are ignored — these are
/// plane-projected stats (reverse driving, facing the ball).
fn forward_xy(r: crate::model::Rot3) -> (f32, f32) {
    (r.yaw.cos(), r.yaw.sin())
}

fn dist(a: Vec3, b: Vec3) -> f64 {
    let (dx, dy, dz) = (a.x - b.x, a.y - b.y, a.z - b.z);
    ((dx * dx + dy * dy + dz * dz) as f64).sqrt()
}

/// Y in the team's attack frame (`+Y` attacking): unchanged for `sign>=0`, negated
/// for `sign<0`.
fn norm_y(y: f32, sign: i32) -> f32 {
    if sign >= 0 {
        y
    } else {
        -y
    }
}

/// Reduce one grid frame into the per-pri accumulators. `possessing` is the team
/// holding the ball this frame (from the possession run containing it), if any.
fn fold_frame(
    frame: &GridFrame,
    signs: &BTreeMap<i32, i32>,
    possessing: Option<i32>,
    accs: &mut BTreeMap<i32, Acc>,
) {
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

        // Orientation-derived: reverse driving (grounded + moving) and facing the
        // ball. Both need the per-frame yaw; skip frames without it.
        if let Some(rot) = c.rot {
            let (fx, fy) = forward_xy(rot);
            let hspeed = (c.v.x * c.v.x + c.v.y * c.v.y).sqrt();
            if z < GROUND_Z && hspeed >= DRIVE_MIN_SPEED {
                a.drive += 1;
                // Velocity component along the forward axis; negative ⇒ reversing.
                if c.v.x * fx + c.v.y * fy < 0.0 {
                    a.reverse += 1;
                }
            }
            if let Some(ball) = &frame.ball {
                let (tx, ty) = (ball.p.x - c.p.x, ball.p.y - c.p.y);
                let tlen = (tx * tx + ty * ty).sqrt();
                if tlen > 1.0 {
                    let cos = (fx * tx + fy * ty) / tlen; // forward · unit-to-ball
                    a.facing_frames += 1;
                    a.facing_deg_sum += cos.clamp(-1.0, 1.0).acos().to_degrees() as f64;
                    if cos >= FACING_CONE_COS {
                        a.facing_cone += 1;
                    }
                }
            }
        }

        if let Some(ball) = &frame.ball {
            a.ball_frames += 1;
            let d = dist(c.p, ball.p);
            a.ball_dist_sum += d;
            // Split by possession when both the car's team and the possessing
            // team are known.
            if let (Some(team), Some(pt)) = (c.team, possessing) {
                if team == pt {
                    a.gp_frames += 1;
                    a.gp_dist_sum += d;
                } else {
                    a.gnp_frames += 1;
                    a.gnp_dist_sum += d;
                }
            }
        }

        // Positioning in the car's own attack frame (+Y attacking).
        if let Some(team) = c.team {
            let sign = signs.get(&team).copied().unwrap_or(1);
            let ny = norm_y(c.p.y, sign);
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
                let nby = norm_y(ball.p.y, sign);
                if ny < nby {
                    a.behind += 1;
                } else {
                    a.infront += 1;
                }
                // Ball in this team's defensive half (own half is −Y in attack frame).
                if nby < 0.0 {
                    a.in_side += 1;
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

    // Most-back / most-forward per team this frame (by attack-frame y). With one
    // car on a team it is both. Ties keep the first car seen.
    let mut by_team: BTreeMap<i32, Vec<(i32, f32)>> = BTreeMap::new();
    for c in &frame.cars {
        if let Some(team) = c.team {
            let sign = signs.get(&team).copied().unwrap_or(1);
            by_team
                .entry(team)
                .or_default()
                .push((c.pri, norm_y(c.p.y, sign)));
        }
    }
    for cars in by_team.values() {
        if let Some((pri, _)) = cars
            .iter()
            .copied()
            .reduce(|a, b| if b.1 < a.1 { b } else { a })
        {
            accs.entry(pri).or_default().most_back += 1;
        }
        if let Some((pri, _)) = cars
            .iter()
            .copied()
            .reduce(|a, b| if b.1 > a.1 { b } else { a })
        {
            accs.entry(pri).or_default().most_forward += 1;
        }
    }

    // Closest / farthest to the ball per team this frame (ball present). Ranked
    // among the team's *present* cars, mirroring most-back/forward; ties keep the
    // first car seen.
    if let Some(ball) = &frame.ball {
        let mut by_team_d: BTreeMap<i32, Vec<(i32, f64)>> = BTreeMap::new();
        for c in &frame.cars {
            if let Some(team) = c.team {
                by_team_d
                    .entry(team)
                    .or_default()
                    .push((c.pri, dist(c.p, ball.p)));
            }
        }
        for cars in by_team_d.values() {
            if let Some((pri, _)) = cars
                .iter()
                .copied()
                .reduce(|a, b| if b.1 < a.1 { b } else { a })
            {
                accs.entry(pri).or_default().closest_ball += 1;
            }
            if let Some((pri, _)) = cars
                .iter()
                .copied()
                .reduce(|a, b| if b.1 > a.1 { b } else { a })
            {
                accs.entry(pri).or_default().farthest_ball += 1;
            }
        }
    }
}

/// The grid frame whose time is nearest `t` (for joining an event to positions).
fn nearest_frame(frames: &[GridFrame], t: f32) -> Option<&GridFrame> {
    if frames.is_empty() {
        return None;
    }
    let i = frames.partition_point(|f| f.t < t);
    [i.checked_sub(1), (i < frames.len()).then_some(i)]
        .into_iter()
        .flatten()
        .map(|j| &frames[j])
        .min_by(|a, b| (a.t - t).abs().total_cmp(&(b.t - t).abs()))
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

    // Possession runs (team, start, end) for the per-frame possession split.
    let poss: Vec<(f32, f32, i32)> = m
        .events
        .iter()
        .filter_map(|e| match e {
            Event::Possession {
                team, start, end, ..
            } => Some((*start, *end, *team)),
            _ => None,
        })
        .collect();
    let possessing_at = |t: f32| -> Option<i32> {
        poss.iter()
            .find(|(s, e, _)| t >= *s && t <= *e)
            .map(|p| p.2)
    };

    let mut accs: BTreeMap<i32, Acc> = BTreeMap::new();
    for f in &m.resampled.frames {
        fold_frame(f, signs, possessing_at(f.t), &mut accs);
    }

    // Goals conceded while last defender: each attributed goal is charged to the
    // back-most player on the conceding team at goal time.
    let mut gawld: BTreeMap<i32, u32> = BTreeMap::new();
    for e in &m.events {
        if let Event::Goal {
            t,
            team: Some(scoring),
            ..
        } = e
        {
            let conceding = if *scoring == 0 { 1 } else { 0 };
            if let Some(frame) = nearest_frame(&m.resampled.frames, *t) {
                let sign = signs.get(&conceding).copied().unwrap_or(1);
                let last_def = frame
                    .cars
                    .iter()
                    .filter(|c| c.team == Some(conceding))
                    .map(|c| (c.pri, norm_y(c.p.y, sign)))
                    .reduce(|a, b| if b.1 < a.1 { b } else { a });
                if let Some((pri, _)) = last_def {
                    *gawld.entry(pri).or_default() += 1;
                }
            }
        }
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

    // Powerslide totals per PRI from the authoritative handbrake intervals, gated
    // to the **on-ground** portion: a powerslide is a grounded handbrake, so an
    // airborne handbrake hold doesn't count (ballchasing agrees — un-gated counts
    // ran ~1.23× high). Build a per-pri grounded timeline from the grid, then
    // intersect each interval with it: count an interval only if it has a grounded
    // frame, and accumulate only its grounded time.
    let mut ground_tl: BTreeMap<i32, Vec<(f32, bool)>> = BTreeMap::new();
    for f in &m.resampled.frames {
        for c in &f.cars {
            ground_tl
                .entry(c.pri)
                .or_default()
                .push((f.t, c.p.z < GROUND_Z));
        }
    }
    let mut ps_by_pri: BTreeMap<i32, (f32, u32)> = BTreeMap::new();
    for p in &m.powerslides {
        let Some(tl) = ground_tl.get(&p.pri) else {
            continue;
        };
        let lo = tl.partition_point(|(t, _)| *t < p.start);
        let hi = tl.partition_point(|(t, _)| *t <= p.end);
        let grounded = tl[lo..hi].iter().filter(|(_, g)| *g).count();
        if grounded > 0 {
            let e = ps_by_pri.entry(p.pri).or_insert((0.0, 0));
            e.0 += grounded as f32 * dt;
            e.1 += 1;
        }
    }
    // Use the authoritative pad-pickup stream (exact counts + BPM) when the replay
    // carried it; fall back to the gauge-step inference otherwise.
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
            let minutes = secs(a.present) / 60.0;
            let per_min = |amt: f32| if minutes > 0.0 { amt / minutes } else { 0.0 };
            let behind_den = a.behind + a.infront;
            // Boost economy: the **gauge-delta** model (sum of positive boost-gauge
            // steps above the jitter floor) is the source for collected / counts /
            // big-small / stolen / overfill, because ballchasing's `amount_collected`
            // and BPM are themselves gauge-based — validated near-exact against
            // ground truth (collected within ~1-3%, big/small split ±1-2). The
            // authoritative `VehiclePickup` event stream (`m.pickups`) is *not* used
            // here: it over-counts (it fires on no-gain drive-overs / pad resets) and
            // its per-event gain/kind are unreliable. See `docs/ballchasing-comparison.md`.
            let sign = t.team.and_then(|tm| signs.get(&tm).copied()).unwrap_or(1);
            let econ = crate::analyze::boost_pads::boost_economy(t, sign);
            let (collected, used, pads) = (econ.collected, econ.used, econ.pads);
            let (ps_time, ps_count) = ps_by_pri.get(&t.pri).copied().unwrap_or((0.0, 0));

            BcPlayerStats {
                pri: t.pri,
                player: t.player.clone(),
                team: t.team,
                boost: BcBoost {
                    avg_amount: mean(a.boost_pct_sum, a.boost_frames),
                    amount_collected: collected,
                    amount_used: used,
                    amount_used_while_supersonic: econ.used_supersonic,
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
                    amount_collected_big: pads.amount_collected_big,
                    amount_collected_small: pads.amount_collected_small,
                    amount_stolen: pads.amount_stolen,
                    amount_stolen_big: pads.amount_stolen_big,
                    amount_stolen_small: pads.amount_stolen_small,
                    count_collected_big: pads.count_collected_big,
                    count_collected_small: pads.count_collected_small,
                    count_stolen_big: pads.count_stolen_big,
                    count_stolen_small: pads.count_stolen_small,
                    amount_overfill: pads.amount_overfill,
                    amount_overfill_stolen: pads.amount_overfill_stolen,
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
                    time_powerslide_s: ps_time,
                    count_powerslide: ps_count,
                    avg_powerslide_duration_s: if ps_count > 0 {
                        ps_time / ps_count as f32
                    } else {
                        0.0
                    },
                    time_reverse_s: secs(a.reverse),
                    percent_reverse: pct(a.reverse, a.drive),
                },
                positioning: BcPositioning {
                    avg_dist_to_ball: mean(a.ball_dist_sum, a.ball_frames),
                    avg_dist_to_ball_possession: mean(a.gp_dist_sum, a.gp_frames),
                    avg_dist_to_ball_no_possession: mean(a.gnp_dist_sum, a.gnp_frames),
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
                    time_ball_in_side_s: secs(a.in_side),
                    percent_ball_in_side: pct(a.in_side, behind_den),
                    time_most_back_s: secs(a.most_back),
                    percent_most_back: pct(a.most_back, a.pos_frames),
                    time_most_forward_s: secs(a.most_forward),
                    percent_most_forward: pct(a.most_forward, a.pos_frames),
                    time_closest_to_ball_s: secs(a.closest_ball),
                    percent_closest_to_ball: pct(a.closest_ball, a.pos_frames),
                    time_farthest_from_ball_s: secs(a.farthest_ball),
                    percent_farthest_from_ball: pct(a.farthest_ball, a.pos_frames),
                    goals_against_while_last_defender: gawld.get(&t.pri).copied().unwrap_or(0),
                    avg_facing_ball_deg: mean(a.facing_deg_sum, a.facing_frames),
                    percent_facing_ball: pct(a.facing_cone, a.facing_frames),
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
