//! **`bc-clone`** — a reverse-engineered clone of ballchasing.com's replay
//! analyzer that emits a **ballchasing-shaped stats document** from a `.replay`.
//!
//! Where the rest of this workspace exposes our *own* neutral model
//! (`CanonicalMatch`, `BcPlayerStats`), this crate reshapes that into the exact
//! JSON `GET /replays/{id}` returns: `blue`/`orange` sides, each with a per-side
//! `stats` block and a `players[]` array, every player carrying the five nested
//! stat groups (`core`, `boost`, `movement`, `positioning`, `demo`) under the
//! field names ballchasing uses. That makes the output drop-in comparable to the
//! real API — see [`crate::compare`] / the `bc-validate` binary.
//!
//! Faithfulness: this reuses the workspace analyzer (decode → reconstruct →
//! `bcstats`) as its engine, exactly as ballchasing reuses an external parser
//! (rattletrap) under its own analyzer. The "clone" is the *output schema and
//! stat semantics*, reproduced field-for-field — not a from-scratch second
//! reconstruction (that independent check already exists as `recon-check`). The
//! mechanical spec it implements is `docs/ballchasing-analyzer-teardown.md`.
//!
//! One field ballchasing exposes that we don't yet compute
//! every ballchasing player field. `amount_used_while_supersonic` is emitted but
//! **approximate** (boost burned while supersonic on the ground — noisy on sparse
//! replays), so it stays in [`UNIMPLEMENTED`] and the validator skips it.

pub mod compare;
pub mod html;

use replay_analyzer::analyze::bcstats::{ballchasing_stats, BcPlayerStats};
use replay_analyzer::model::{Camera, CanonicalMatch, Event, PlayerMeta};
use serde::{Deserialize, Serialize};

/// Maximum car speed (uu/s), for `avg_speed_percentage`.
const MAX_SPEED: f32 = 2300.0;

/// Ballchasing fields the validator skips: `amount_used_while_supersonic` is
/// emitted but only **approximate** (reconstruction-noisy), so it must not gate
/// an exact-match comparison.
pub const UNIMPLEMENTED: &[&str] = &["amount_used_while_supersonic"];

/// Top-level document, matching ballchasing's `GET /replays/{id}`.
#[derive(Debug, Clone, Serialize)]
pub struct BcReplayDoc {
    pub id: String,
    pub status: &'static str,
    pub map_name: Option<String>,
    pub team_size: Option<i32>,
    pub duration: i64,
    /// Ball-control split (% of touched-possession time per team).
    pub possession: TeamSplit,
    /// Pressure split — % of time the ball sits in each team's own half.
    pub pressure: TeamSplit,
    pub blue: Side,
    pub orange: Side,
}

/// A blue-vs-orange percentage split (the two sum to ~100).
#[derive(Debug, Clone, Copy, Default, Serialize, Deserialize)]
pub struct TeamSplit {
    pub blue: f32,
    pub orange: f32,
}

/// One team side (`blue` = team 0, `orange` = team 1).
#[derive(Debug, Clone, Serialize)]
pub struct Side {
    pub name: &'static str,
    pub color: &'static str,
    /// Team-aggregate stats (per-player blocks summed/averaged by field).
    pub stats: Stats,
    pub players: Vec<Player>,
}

/// One player, with identity and the five nested stat groups.
#[derive(Debug, Clone, Serialize)]
pub struct Player {
    pub name: String,
    /// Car-body product id + resolved name (ballchasing's `car_id`/`car_name`),
    /// from the player's loadout. `None` when the replay carries no loadout.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub car_id: Option<u32>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub car_name: Option<String>,
    /// In-game camera profile + steering sensitivity (ballchasing's `camera` /
    /// `steering_sensitivity`).
    #[serde(skip_serializing_if = "Option::is_none")]
    pub camera: Option<Camera>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub steering_sensitivity: Option<f32>,
    pub stats: Stats,
}

/// The five ballchasing stat groups.
#[derive(Debug, Clone, Default, Serialize, Deserialize)]
pub struct Stats {
    pub core: Core,
    pub boost: Boost,
    pub movement: Movement,
    pub positioning: Positioning,
    pub demo: Demo,
}

#[derive(Debug, Clone, Default, Serialize, Deserialize)]
pub struct Core {
    pub shots: i32,
    pub shots_against: i32,
    pub goals: i32,
    pub goals_against: i32,
    pub saves: i32,
    pub assists: i32,
    pub score: i32,
    pub mvp: bool,
    pub shooting_percentage: f32,
}

#[derive(Debug, Clone, Default, Serialize, Deserialize)]
pub struct Boost {
    pub bpm: f32,
    pub bcpm: f32,
    pub avg_amount: f32,
    pub amount_collected: f32,
    pub amount_stolen: f32,
    pub amount_collected_big: f32,
    pub amount_stolen_big: f32,
    pub amount_collected_small: f32,
    pub amount_stolen_small: f32,
    pub count_collected_big: u32,
    pub count_stolen_big: u32,
    pub count_collected_small: u32,
    pub count_stolen_small: u32,
    pub amount_overfill: f32,
    pub amount_overfill_stolen: f32,
    pub amount_used_while_supersonic: f32,
    pub time_zero_boost: f32,
    pub percent_zero_boost: f32,
    pub time_full_boost: f32,
    pub percent_full_boost: f32,
    pub time_boost_0_25: f32,
    pub time_boost_25_50: f32,
    pub time_boost_50_75: f32,
    pub time_boost_75_100: f32,
    pub percent_boost_0_25: f32,
    pub percent_boost_25_50: f32,
    pub percent_boost_50_75: f32,
    pub percent_boost_75_100: f32,
}

#[derive(Debug, Clone, Default, Serialize, Deserialize)]
pub struct Movement {
    pub avg_speed: f32,
    pub total_distance: f32,
    pub time_supersonic_speed: f32,
    pub time_boost_speed: f32,
    pub time_slow_speed: f32,
    pub time_ground: f32,
    pub time_low_air: f32,
    pub time_high_air: f32,
    pub time_powerslide: f32,
    pub count_powerslide: u32,
    pub avg_powerslide_duration: f32,
    pub avg_speed_percentage: f32,
    pub percent_slow_speed: f32,
    pub percent_boost_speed: f32,
    pub percent_supersonic_speed: f32,
    pub percent_ground: f32,
    pub percent_low_air: f32,
    pub percent_high_air: f32,
}

#[derive(Debug, Clone, Default, Serialize, Deserialize)]
pub struct Positioning {
    pub avg_distance_to_ball: f32,
    pub avg_distance_to_ball_possession: f32,
    pub avg_distance_to_ball_no_possession: f32,
    pub avg_distance_to_mates: f32,
    pub time_defensive_third: f32,
    pub time_neutral_third: f32,
    pub time_offensive_third: f32,
    pub time_defensive_half: f32,
    pub time_offensive_half: f32,
    pub time_behind_ball: f32,
    pub time_infront_ball: f32,
    pub time_most_back: f32,
    pub time_most_forward: f32,
    pub goals_against_while_last_defender: u32,
    pub time_closest_to_ball: f32,
    pub time_farthest_from_ball: f32,
    pub percent_defensive_third: f32,
    pub percent_neutral_third: f32,
    pub percent_offensive_third: f32,
    pub percent_defensive_half: f32,
    pub percent_offensive_half: f32,
    pub percent_behind_ball: f32,
    pub percent_infront_ball: f32,
    pub percent_most_back: f32,
    pub percent_most_forward: f32,
    pub percent_closest_to_ball: f32,
    pub percent_farthest_from_ball: f32,
}

#[derive(Debug, Clone, Default, Serialize, Deserialize)]
pub struct Demo {
    pub inflicted: u32,
    pub taken: u32,
}

/// Build the ballchasing-shaped document for a decoded match.
pub fn ballchasing_document(m: &CanonicalMatch) -> BcReplayDoc {
    let stats = ballchasing_stats(m);

    // Per-side per-player core needs the header scoreboard (goals/saves/…), joined
    // to the spatial block by (name, team).
    let header = |name: &str, team: Option<i32>| -> Option<&PlayerMeta> {
        m.players
            .iter()
            .find(|p| p.name == name && Some(p.team) == team)
            .or_else(|| m.players.iter().find(|p| p.name == name))
    };
    // Team goal/shot totals for the *_against fields (replicated per player).
    let team_goals = |t: i32| -> i32 {
        m.players
            .iter()
            .filter(|p| p.team == t)
            .map(|p| p.goals)
            .sum()
    };
    let team_shots = |t: i32| -> i32 {
        m.players
            .iter()
            .filter(|p| p.team == t)
            .map(|p| p.shots)
            .sum()
    };

    let player_of = |s: &BcPlayerStats| -> Player {
        let team = s.team.unwrap_or(0);
        let opp = if team == 0 { 1 } else { 0 };
        let h = header(&s.player, s.team);
        let (goals, shots, saves, assists, score) = h
            .map(|p| (p.goals, p.shots, p.saves, p.assists, p.score))
            .unwrap_or((0, 0, 0, 0, 0));
        Player {
            name: s.player.clone(),
            car_id: h.and_then(|p| p.car_id),
            car_name: h.and_then(|p| p.car_name.clone()),
            camera: h.and_then(|p| p.camera),
            steering_sensitivity: h.and_then(|p| p.steering_sensitivity),
            stats: Stats {
                core: Core {
                    shots,
                    shots_against: team_shots(opp),
                    goals,
                    goals_against: team_goals(opp),
                    saves,
                    assists,
                    score,
                    mvp: false,
                    shooting_percentage: pct_ratio(goals as f32, shots as f32),
                },
                boost: boost_of(s),
                movement: movement_of(s),
                positioning: positioning_of(s),
                demo: Demo {
                    inflicted: s.demo.inflicted,
                    taken: s.demo.taken,
                },
            },
        }
    };

    let side = |team: i32, name: &'static str, color: &'static str| -> Side {
        let players: Vec<Player> = stats
            .iter()
            .filter(|s| s.team == Some(team))
            .map(player_of)
            .collect();
        let mut team_stats = compare::aggregate_side(&players);
        // Team shooting % is recomputed from team totals, not averaged per player.
        team_stats.core.shooting_percentage =
            pct_ratio(team_stats.core.goals as f32, team_stats.core.shots as f32);
        Side {
            name,
            color,
            stats: team_stats,
            players,
        }
    };

    let mut doc = BcReplayDoc {
        id: m.replay_id.clone(),
        status: "ok",
        map_name: m.map.clone(),
        team_size: m.team_size,
        duration: m.duration_s.round() as i64,
        possession: possession_split(m),
        pressure: pressure_split(m),
        blue: side(0, "blue", "blue"),
        orange: side(1, "orange", "orange"),
    };
    set_mvp(&mut doc);
    doc
}

/// Flag the MVP: the highest-scoring player on the winning team (the standard
/// rule). We don't read the replicated `bMatchMVP`, so this is derived.
fn set_mvp(doc: &mut BcReplayDoc) {
    let winner = if doc.blue.stats.core.goals >= doc.orange.stats.core.goals {
        &mut doc.blue
    } else {
        &mut doc.orange
    };
    if let Some(p) = winner.players.iter_mut().max_by_key(|p| p.stats.core.score) {
        p.stats.core.mvp = true;
    }
}

/// Ball-control split: share of [`Event::Possession`] run-time per team.
fn possession_split(m: &CanonicalMatch) -> TeamSplit {
    let (mut blue, mut orange) = (0.0f32, 0.0f32);
    for e in &m.events {
        if let Event::Possession {
            team, start, end, ..
        } = e
        {
            let d = (end - start).max(0.0);
            if *team == 0 {
                blue += d;
            } else {
                orange += d;
            }
        }
    }
    let tot = (blue + orange).max(1e-6);
    TeamSplit {
        blue: blue / tot * 100.0,
        orange: orange / tot * 100.0,
    }
}

/// Pressure split: share of ball-present frames the ball sits in each team's own
/// half (team 0's own half is where the ball's attack-frame `y` is negative).
fn pressure_split(m: &CanonicalMatch) -> TeamSplit {
    let s0 = m.resampled.team_attack_sign.get(&0).copied().unwrap_or(1);
    let (mut blue, mut total) = (0u64, 0u64);
    for f in &m.resampled.frames {
        if let Some(ball) = &f.ball {
            total += 1;
            let ny = if s0 >= 0 { ball.p.y } else { -ball.p.y };
            if ny < 0.0 {
                blue += 1;
            }
        }
    }
    if total == 0 {
        return TeamSplit::default();
    }
    let b = blue as f32 / total as f32 * 100.0;
    TeamSplit {
        blue: b,
        orange: 100.0 - b,
    }
}

/// `n / d × 100`, guarding division by zero.
fn pct_ratio(n: f32, d: f32) -> f32 {
    if d > 0.0 {
        n / d * 100.0
    } else {
        0.0
    }
}

fn boost_of(s: &BcPlayerStats) -> Boost {
    let b = &s.boost;
    Boost {
        bpm: b.bpm,
        bcpm: b.bcpm,
        avg_amount: b.avg_amount,
        amount_collected: b.amount_collected,
        amount_stolen: b.amount_stolen,
        amount_collected_big: b.amount_collected_big,
        amount_stolen_big: b.amount_stolen_big,
        amount_collected_small: b.amount_collected_small,
        amount_stolen_small: b.amount_stolen_small,
        count_collected_big: b.count_collected_big,
        count_stolen_big: b.count_stolen_big,
        count_collected_small: b.count_collected_small,
        count_stolen_small: b.count_stolen_small,
        amount_overfill: b.amount_overfill,
        amount_overfill_stolen: b.amount_overfill_stolen,
        amount_used_while_supersonic: b.amount_used_while_supersonic,
        time_zero_boost: b.time_zero_s,
        percent_zero_boost: b.percent_zero,
        time_full_boost: b.time_full_s,
        percent_full_boost: b.percent_full,
        time_boost_0_25: b.time_0_25_s,
        time_boost_25_50: b.time_25_50_s,
        time_boost_50_75: b.time_50_75_s,
        time_boost_75_100: b.time_75_100_s,
        percent_boost_0_25: b.percent_0_25,
        percent_boost_25_50: b.percent_25_50,
        percent_boost_50_75: b.percent_50_75,
        percent_boost_75_100: b.percent_75_100,
    }
}

fn movement_of(s: &BcPlayerStats) -> Movement {
    let mv = &s.movement;
    Movement {
        avg_speed: mv.avg_speed,
        total_distance: mv.total_distance,
        time_supersonic_speed: mv.time_supersonic_s,
        time_boost_speed: mv.time_boost_speed_s,
        time_slow_speed: mv.time_slow_s,
        time_ground: mv.time_ground_s,
        time_low_air: mv.time_low_air_s,
        time_high_air: mv.time_high_air_s,
        time_powerslide: mv.time_powerslide_s,
        count_powerslide: mv.count_powerslide,
        avg_powerslide_duration: mv.avg_powerslide_duration_s,
        avg_speed_percentage: pct_ratio(mv.avg_speed, MAX_SPEED),
        percent_slow_speed: mv.percent_slow,
        percent_boost_speed: mv.percent_boost_speed,
        percent_supersonic_speed: mv.percent_supersonic,
        percent_ground: mv.percent_ground,
        percent_low_air: mv.percent_low_air,
        percent_high_air: mv.percent_high_air,
    }
}

fn positioning_of(s: &BcPlayerStats) -> Positioning {
    let p = &s.positioning;
    Positioning {
        avg_distance_to_ball: p.avg_dist_to_ball,
        avg_distance_to_ball_possession: p.avg_dist_to_ball_possession,
        avg_distance_to_ball_no_possession: p.avg_dist_to_ball_no_possession,
        avg_distance_to_mates: p.avg_dist_to_mates,
        time_defensive_third: p.time_defensive_third_s,
        time_neutral_third: p.time_neutral_third_s,
        time_offensive_third: p.time_offensive_third_s,
        time_defensive_half: p.time_defensive_half_s,
        time_offensive_half: p.time_offensive_half_s,
        time_behind_ball: p.time_behind_ball_s,
        time_infront_ball: p.time_infront_ball_s,
        time_most_back: p.time_most_back_s,
        time_most_forward: p.time_most_forward_s,
        goals_against_while_last_defender: p.goals_against_while_last_defender,
        time_closest_to_ball: p.time_closest_to_ball_s,
        time_farthest_from_ball: p.time_farthest_from_ball_s,
        percent_defensive_third: p.percent_defensive_third,
        percent_neutral_third: p.percent_neutral_third,
        percent_offensive_third: p.percent_offensive_third,
        percent_defensive_half: p.percent_defensive_half,
        percent_offensive_half: p.percent_offensive_half,
        percent_behind_ball: p.percent_behind_ball,
        percent_infront_ball: p.percent_infront_ball,
        percent_most_back: p.percent_most_back,
        percent_most_forward: p.percent_most_forward,
        percent_closest_to_ball: p.percent_closest_to_ball,
        percent_farthest_from_ball: p.percent_farthest_from_ball,
    }
}
