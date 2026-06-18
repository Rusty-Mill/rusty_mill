//! **Phase 2 reconstruction cross-check** (spec §13 "second parser adapter for
//! redundancy", Phase 2 — independent *frame-level* corroboration).
//!
//! Phase 1 ([`replay_scoring::contract`]) cross-checks header *facts* against
//! ballchasing. This crate cross-checks the *reconstruction* — the ball and car
//! trajectories over time — against [`subtr_actor`], a separately-authored Rust
//! reconstructor. It is the one reconstruction surface nothing else corroborates:
//! `external_validation` only checks per-player *aggregates*, and the golden test
//! pins our output to its own past (regression, not correctness).
//!
//! ## Independence boundary (the whole point)
//! Two copies of the same bug agreeing proves nothing, so the *independent* side
//! must not share our reconstruction logic. This crate gets our side from
//! [`build_canonical`] (the public entry — the reconstruction under test) and the
//! independent side from `subtr-actor`; it never reaches into
//! `replay_analyzer::analyze` internals. **Caveat:** both decode the replay with
//! `boxcars` (the shared parse layer), so a `boxcars`-level decode bug stays
//! invisible here — that boundary is what the Phase 1 ballchasing check covers
//! for header facts. This is *reconstruction-layer* independence.
//!
//! ## What it cross-checks (tiered, with tolerances — float/time data)
//! - **Tier R1 — fail:** roster (player set, matched on the network-derived
//!   *tracks* so empty-header replays still resolve), adequate frame coverage, and
//!   **ball position** agreement on the shared fixed-rate grid (median + agree-rate
//!   under tolerance). Ball stats are taken over *gameplay* frames only — post-goal
//!   reset windows `[goal, next kickoff)` are excluded, since the ball is
//!   non-gameplay there and the two reconstructions legitimately diverge. The spike
//!   measured ~20–30 uu mid-match, well under a ball radius (~93 uu).
//! - **Tier R2 — advisory:** per-player **car position** agreement (matched by
//!   name) and boost. Boost units differ by convention (Phase 1 Tier-3 territory),
//!   so it is reported, never gated.

use std::collections::HashMap;
use std::error::Error;

use replay_analyzer::model::{CanonicalMatch, Event, Vec3};
use serde::{Deserialize, Serialize};
use subtr_actor::{Collector, FrameRateDecorator, NDArrayCollector};

pub use replay_analyzer::analyze::build_canonical;
pub use replay_analyzer::decode::{boxcars_adapter::BoxcarsParser, ReplayParser};

/// Default Tier-R1 tolerances. Ball position is the gated signal; these are set
/// with healthy margin over the spike's measured agreement (~20–30 uu median).
pub const BALL_MEDIAN_MAX_UU: f32 = 60.0;
/// A frame's ball positions "agree" when within this distance.
pub const BALL_AGREE_TOL_UU: f32 = 200.0;
/// Minimum fraction of aligned frames that must agree (the rest are tolerated as
/// goal-celebration / reset windows).
pub const MIN_AGREE_RATE: f32 = 0.80;
/// Minimum fraction of our ball-bearing frames that must align to a subtr frame.
pub const MIN_COVERAGE: f32 = 0.90;

// ---------------------------------------------------------------------------
// The independent reconstruction (subtr-actor), distilled to a comparable grid.
// ---------------------------------------------------------------------------

/// One sampled frame of subtr-actor's reconstruction. `None` fields are absent
/// (NaN in the source matrix — e.g. a demolished/respawning car).
#[derive(Debug, Clone)]
pub struct SubtrFrame {
    /// Game clock at this sample (same origin as our grid's `t`).
    pub time: f32,
    pub ball: Option<Vec3>,
    pub cars: Vec<SubtrCar>,
}

#[derive(Debug, Clone)]
pub struct SubtrCar {
    pub player: String,
    pub pos: Option<Vec3>,
    pub boost: Option<f32>,
}

/// subtr-actor's reconstruction, sampled at the same fixed rate as our grid.
#[derive(Debug, Clone)]
pub struct SubtrGrid {
    pub hz: f32,
    pub players: Vec<String>,
    pub frames: Vec<SubtrFrame>,
}

/// `NDArrayCollector` column layout for our chosen feature adders (see the spike):
/// 13 global cols then 13 per player, both blocks ordered pos(0..3), rot(3..6),
/// linear vel(6..9), angular vel(9..12), then time (global col 12) / boost
/// (player col 12).
const GLOBAL_COLS: usize = 13;
const PLAYER_COLS: usize = 13;

fn vec3_or_none(x: f32, y: f32, z: f32) -> Option<Vec3> {
    if x.is_nan() || y.is_nan() || z.is_nan() {
        None
    } else {
        Some(Vec3 { x, y, z })
    }
}

/// Run subtr-actor over a decoded replay and distill its per-frame ball/car state
/// onto a fixed-rate grid (sample at `hz` to match our `Resampled` grid).
pub fn subtr_grid(replay: &boxcars::Replay, hz: f32) -> Result<SubtrGrid, Box<dyn Error>> {
    let mut collector = NDArrayCollector::<f32>::from_strings(
        &["BallRigidBody", "CurrentTime"],
        &["PlayerRigidBody", "PlayerBoost"],
    )
    .map_err(|e| format!("subtr collector: {e:?}"))?;
    FrameRateDecorator::new_from_fps(hz, &mut collector)
        .process_replay(replay)
        .map_err(|e| format!("subtr process: {e:?}"))?;
    let (meta, arr) = collector
        .get_meta_and_ndarray()
        .map_err(|e| format!("subtr ndarray: {e:?}"))?;

    let players: Vec<String> = meta
        .replay_meta
        .player_order()
        .map(|p| p.name.clone())
        .collect();

    let nframes = arr.shape()[0];
    let ncols = arr.shape()[1];
    let expected = GLOBAL_COLS + PLAYER_COLS * players.len();
    if ncols != expected {
        return Err(format!("unexpected ndarray width {ncols} (expected {expected})").into());
    }

    let mut frames = Vec::with_capacity(nframes);
    for row in 0..nframes {
        let at = |c: usize| arr[[row, c]];
        let ball = vec3_or_none(at(0), at(1), at(2));
        let time = at(12);
        let cars = players
            .iter()
            .enumerate()
            .map(|(pi, name)| {
                let b = GLOBAL_COLS + pi * PLAYER_COLS;
                let boost = {
                    let v = at(b + 12);
                    if v.is_nan() {
                        None
                    } else {
                        Some(v)
                    }
                };
                SubtrCar {
                    player: name.clone(),
                    pos: vec3_or_none(at(b), at(b + 1), at(b + 2)),
                    boost,
                }
            })
            .collect();
        frames.push(SubtrFrame { time, ball, cars });
    }
    Ok(SubtrGrid {
        hz,
        players,
        frames,
    })
}

// ---------------------------------------------------------------------------
// Cross-check.
// ---------------------------------------------------------------------------

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum Tier {
    /// Exact-ish contract within tolerance: a breach fails the cross-check.
    R1,
    /// Advisory: surfaced, never fails.
    R2,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct Finding {
    pub tier: Tier,
    pub field: String,
    pub detail: String,
}

/// Per-player car-position agreement (advisory).
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct PlayerReconStat {
    pub player: String,
    pub frames: usize,
    pub car_median_uu: f32,
    pub car_p95_uu: f32,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct ReconReport {
    pub replay_id: String,
    pub hz: f32,
    pub our_frames: usize,
    pub subtr_frames: usize,
    /// Frames aligned (within tolerance) with a ball in both reconstructions.
    pub matched_frames: usize,
    pub coverage: f32,
    pub ball_median_uu: f32,
    pub ball_p95_uu: f32,
    pub ball_max_uu: f32,
    /// Fraction of aligned frames whose ball positions agree within tolerance.
    pub ball_agree_rate: f32,
    pub per_player: Vec<PlayerReconStat>,
    pub tier1: Vec<Finding>,
    pub tier2: Vec<Finding>,
}

impl ReconReport {
    /// True when no Tier-R1 tolerance was breached — the pass/fail gate.
    pub fn tier1_ok(&self) -> bool {
        self.tier1.is_empty()
    }
}

fn dist(a: Vec3, b: Vec3) -> f32 {
    let (dx, dy, dz) = (a.x - b.x, a.y - b.y, a.z - b.z);
    (dx * dx + dy * dy + dz * dz).sqrt()
}

fn norm_name(s: &str) -> String {
    s.split_whitespace()
        .collect::<Vec<_>>()
        .join(" ")
        .to_lowercase()
}

/// `sorted` must be ascending. Returns (median, p95, max), or zeros if empty.
fn percentiles(mut v: Vec<f32>) -> (f32, f32, f32) {
    if v.is_empty() {
        return (0.0, 0.0, 0.0);
    }
    v.sort_by(|a, b| a.partial_cmp(b).unwrap());
    let n = v.len();
    let at = |q: f32| v[((q * n as f32) as usize).min(n - 1)];
    (at(0.5), at(0.95), v[n - 1])
}

/// Cross-check our reconstruction against subtr-actor's, both on the same
/// fixed-rate grid. Frames align by nearest game-clock time (both share the
/// clock origin), tolerant to either side trimming a few frames.
pub fn cross_check_recon(our: &CanonicalMatch, subtr: &SubtrGrid) -> ReconReport {
    let mut tier1 = Vec::new();
    let mut tier2 = Vec::new();
    let hz = our.resampled.hz;

    // --- roster (R1): the player sets must match (by normalized name) ---
    // Use the network-derived tracks, not the header player list: empty-header
    // replays carry no header players, but their tracks still bind the real names
    // (the same source the per-player car matching below uses).
    let mut ours_names: Vec<String> = our.tracks.iter().map(|t| norm_name(&t.player)).collect();
    ours_names.sort();
    ours_names.dedup();
    let subtr_names: Vec<String> = subtr.players.iter().map(|p| norm_name(p)).collect();
    for n in &ours_names {
        if !subtr_names.contains(n) {
            tier1.push(Finding {
                tier: Tier::R1,
                field: "roster".into(),
                detail: format!("{n:?} in our decode but not subtr-actor"),
            });
        }
    }
    for n in &subtr_names {
        if !ours_names.contains(n) {
            tier1.push(Finding {
                tier: Tier::R1,
                field: "roster".into(),
                detail: format!("{n:?} in subtr-actor but not our decode"),
            });
        }
    }

    // pri -> normalized name, to match our grid cars (keyed by pri) to subtr cars.
    let pri_name: HashMap<i32, String> = our
        .tracks
        .iter()
        .map(|t| (t.pri, norm_name(&t.player)))
        .collect();

    // Ascending subtr sample times for nearest-time alignment.
    let subtr_times: Vec<f32> = subtr.frames.iter().map(|f| f.time).collect();
    let tol = 0.5 / hz.max(1.0); // half a frame

    // Post-goal reset windows `[goal, next kickoff)`: the ball is non-gameplay
    // (celebration / goal replay / kickoff setup) and the two reconstructions
    // legitimately diverge there, so these frames are excluded from the ball-delta
    // stats. Parameter-free — built from the goal + kickoff events we already emit.
    let kickoff_times: Vec<f32> = our
        .events
        .iter()
        .filter(|e| matches!(e, Event::Kickoff { .. }))
        .map(|e| e.time())
        .collect();
    let reset_windows: Vec<(f32, f32)> = our
        .events
        .iter()
        .filter(|e| matches!(e, Event::Goal { .. }))
        .map(|e| e.time())
        .map(|g| {
            let next_ko = kickoff_times
                .iter()
                .copied()
                .find(|&k| k > g)
                .unwrap_or(f32::INFINITY);
            (g, next_ko)
        })
        .collect();
    let in_reset = |t: f32| reset_windows.iter().any(|&(a, b)| t >= a && t < b);

    // (frame time, ball Δ) so reset-window frames can be filtered from the stats.
    let mut ball_deltas: Vec<(f32, f32)> = Vec::new();
    let mut car_deltas: HashMap<String, Vec<f32>> = HashMap::new();
    let mut our_ball_frames = 0usize;

    for gf in &our.resampled.frames {
        let Some(our_ball) = gf.ball else { continue };
        our_ball_frames += 1;
        // nearest subtr frame by time
        let idx = subtr_times.partition_point(|&t| t < gf.t);
        let cand = [idx.wrapping_sub(1), idx];
        let best = cand
            .into_iter()
            .filter(|&i| i < subtr.frames.len())
            .min_by(|&a, &b| {
                (subtr_times[a] - gf.t)
                    .abs()
                    .partial_cmp(&(subtr_times[b] - gf.t).abs())
                    .unwrap()
            });
        let Some(si) = best else { continue };
        if (subtr_times[si] - gf.t).abs() > tol {
            continue;
        }
        let sf = &subtr.frames[si];
        if let Some(sb) = sf.ball {
            ball_deltas.push((gf.t, dist(our_ball.p, sb)));
        }
        // per-player car positions (advisory)
        for gc in &gf.cars {
            let Some(name) = pri_name.get(&gc.pri) else {
                continue;
            };
            let Some(sc) = sf.cars.iter().find(|c| norm_name(&c.player) == *name) else {
                continue;
            };
            if let Some(sp) = sc.pos {
                car_deltas
                    .entry(name.clone())
                    .or_default()
                    .push(dist(gc.p, sp));
            }
        }
    }

    let matched_frames = ball_deltas.len();
    let coverage = if our_ball_frames == 0 {
        0.0
    } else {
        matched_frames as f32 / our_ball_frames as f32
    };
    // Ball-delta stats over *gameplay* frames only: post-goal reset windows are
    // excluded (the ball is non-gameplay there and the reconstructions legitimately
    // diverge — they blow up p95/max and the agree rate but not the bulk agreement).
    // Coverage above stays over all aligned frames (an alignment metric).
    let gameplay: Vec<f32> = ball_deltas
        .iter()
        .filter(|&&(t, _)| !in_reset(t))
        .map(|&(_, d)| d)
        .collect();
    let agree = gameplay.iter().filter(|&&d| d <= BALL_AGREE_TOL_UU).count();
    let agree_rate = if gameplay.is_empty() {
        1.0 // no gameplay frames to disagree on (degenerate); don't false-fail
    } else {
        agree as f32 / gameplay.len() as f32
    };
    let (ball_median, ball_p95, ball_max) = percentiles(gameplay);

    // --- Tier R1 gates ---
    if matched_frames == 0 {
        tier1.push(Finding {
            tier: Tier::R1,
            field: "coverage".into(),
            detail: "no frames aligned between the two reconstructions".into(),
        });
    } else {
        if coverage < MIN_COVERAGE {
            tier1.push(Finding {
                tier: Tier::R1,
                field: "coverage".into(),
                detail: format!(
                    "only {:.1}% of frames aligned (min {:.0}%)",
                    coverage * 100.0,
                    MIN_COVERAGE * 100.0
                ),
            });
        }
        if ball_median > BALL_MEDIAN_MAX_UU {
            tier1.push(Finding {
                tier: Tier::R1,
                field: "ball_position".into(),
                detail: format!("median Δ {ball_median:.1} uu > {BALL_MEDIAN_MAX_UU:.0} uu"),
            });
        }
        if agree_rate < MIN_AGREE_RATE {
            tier1.push(Finding {
                tier: Tier::R1,
                field: "ball_position".into(),
                detail: format!(
                    "only {:.1}% of frames agree within {:.0} uu (min {:.0}%)",
                    agree_rate * 100.0,
                    BALL_AGREE_TOL_UU,
                    MIN_AGREE_RATE * 100.0
                ),
            });
        }
    }

    // --- Tier R2: per-player car agreement (advisory) ---
    let mut per_player: Vec<PlayerReconStat> = car_deltas
        .into_iter()
        .map(|(player, deltas)| {
            let frames = deltas.len();
            let (m, p95, _) = percentiles(deltas);
            PlayerReconStat {
                player,
                frames,
                car_median_uu: m,
                car_p95_uu: p95,
            }
        })
        .collect();
    per_player.sort_by(|a, b| a.player.cmp(&b.player));
    for s in &per_player {
        // Cars are smaller and move faster than the ball; flag only gross drift.
        if s.car_median_uu > 150.0 {
            tier2.push(Finding {
                tier: Tier::R2,
                field: "car_position".into(),
                detail: format!("{:?}: median Δ {:.1} uu", s.player, s.car_median_uu),
            });
        }
    }

    ReconReport {
        replay_id: our.replay_id.clone(),
        hz,
        our_frames: our.resampled.frames.len(),
        subtr_frames: subtr.frames.len(),
        matched_frames,
        coverage,
        ball_median_uu: ball_median,
        ball_p95_uu: ball_p95,
        ball_max_uu: ball_max,
        ball_agree_rate: agree_rate,
        per_player,
        tier1,
        tier2,
    }
}

/// Build both reconstructions of one replay: ours (the model under test, via
/// [`build_canonical`]) and subtr-actor's, sampled onto our grid's rate. Each
/// side runs its own `boxcars` parse — the deliberately-shared layer.
pub fn reconstructions(
    bytes: &[u8],
    replay_id: &str,
) -> Result<(CanonicalMatch, SubtrGrid), Box<dyn Error>> {
    let decoded = BoxcarsParser::new().parse(bytes)?;
    let our = build_canonical(&decoded, replay_id);
    let replay = boxcars::ParserBuilder::new(bytes)
        .must_parse_network_data()
        .on_error_check_crc()
        .parse()?;
    let subtr = subtr_grid(&replay, our.resampled.hz)?;
    Ok((our, subtr))
}

/// Decode a `.replay` and cross-check our reconstruction against subtr-actor's.
/// One call so the binary and tests share the exact pipeline.
pub fn cross_check_replay(bytes: &[u8], replay_id: &str) -> Result<ReconReport, Box<dyn Error>> {
    let (our, subtr) = reconstructions(bytes, replay_id)?;
    Ok(cross_check_recon(&our, &subtr))
}

#[cfg(test)]
mod tests {
    use super::*;
    use replay_analyzer::model::{GridCar, GridFrame, Kin, PlayerMeta, PlayerTrack, Resampled};
    use std::collections::BTreeMap;

    fn v(x: f32, y: f32, z: f32) -> Vec3 {
        Vec3 { x, y, z }
    }

    /// A tiny two-player canonical with a 3-frame ball/car grid at 30 Hz.
    fn our_match() -> CanonicalMatch {
        let car = |pri, x: f32| GridCar {
            pri,
            team: Some(0),
            p: v(x, 0.0, 17.0),
            v: v(0.0, 0.0, 0.0),
            boost: Some(100),
            rot: None,
        };
        let frame = |t: f32, bx: f32| GridFrame {
            t,
            ball: Some(Kin {
                p: v(bx, 0.0, 93.0),
                v: v(0.0, 0.0, 0.0),
            }),
            cars: vec![car(1, bx - 50.0), car(2, bx + 50.0)],
        };
        CanonicalMatch {
            replay_id: "t".into(),
            parser_version: "test".into(),
            analyzer_version: "test".into(),
            map: Some("Stadium_P".into()),
            team_size: Some(1),
            record_fps: Some(30.0),
            num_frames: 3,
            duration_s: 0.1,
            team_scores: BTreeMap::from([(0, 0), (1, 0)]),
            players: vec![
                PlayerMeta {
                    name: "Alice".into(),
                    team: 0,
                    score: 0,
                    goals: 0,
                    assists: 0,
                    saves: 0,
                    shots: 0,
                },
                PlayerMeta {
                    name: "Bob".into(),
                    team: 1,
                    score: 0,
                    goals: 0,
                    assists: 0,
                    saves: 0,
                    shots: 0,
                },
            ],
            tracks: vec![
                PlayerTrack {
                    player: "Alice".into(),
                    pri: 1,
                    team: Some(0),
                    num_segments: 1,
                    samples: vec![],
                    gaps: vec![],
                },
                PlayerTrack {
                    player: "Bob".into(),
                    pri: 2,
                    team: Some(1),
                    num_segments: 1,
                    samples: vec![],
                    gaps: vec![],
                },
            ],
            frames: vec![],
            resampled: Resampled {
                hz: 30.0,
                team_attack_sign: BTreeMap::new(),
                frames: vec![
                    frame(0.0, 0.0),
                    frame(1.0 / 30.0, 100.0),
                    frame(2.0 / 30.0, 200.0),
                ],
            },
            events: vec![],
            features: vec![],
        }
    }

    /// A subtr grid that agrees with `our_match` (same ball/car at same times).
    fn subtr_agreeing(offset: f32) -> SubtrGrid {
        let frame = |t: f32, bx: f32| SubtrFrame {
            time: t,
            ball: Some(v(bx + offset, 0.0, 93.0)),
            cars: vec![
                SubtrCar {
                    player: "Alice".into(),
                    pos: Some(v(bx - 50.0, 0.0, 17.0)),
                    boost: Some(255.0),
                },
                SubtrCar {
                    player: "Bob".into(),
                    pos: Some(v(bx + 50.0, 0.0, 17.0)),
                    boost: Some(255.0),
                },
            ],
        };
        SubtrGrid {
            hz: 30.0,
            players: vec!["Alice".into(), "Bob".into()],
            frames: vec![
                frame(0.0, 0.0),
                frame(1.0 / 30.0, 100.0),
                frame(2.0 / 30.0, 200.0),
            ],
        }
    }

    #[test]
    fn agreeing_reconstructions_pass_r1() {
        let rep = cross_check_recon(&our_match(), &subtr_agreeing(5.0));
        assert!(rep.tier1_ok(), "expected R1 pass, got {:?}", rep.tier1);
        assert_eq!(rep.matched_frames, 3);
        assert!(rep.coverage > 0.99);
        assert!(rep.ball_median_uu < 10.0);
    }

    #[test]
    fn ball_drift_trips_r1() {
        // A constant 600 uu ball offset blows past both the median and agree gates.
        let rep = cross_check_recon(&our_match(), &subtr_agreeing(600.0));
        assert!(!rep.tier1_ok());
        assert!(rep.tier1.iter().any(|f| f.field == "ball_position"));
    }

    #[test]
    fn roster_mismatch_is_r1() {
        let mut subtr = subtr_agreeing(0.0);
        subtr.players[1] = "Charlie".into();
        for f in &mut subtr.frames {
            f.cars[1].player = "Charlie".into();
        }
        let rep = cross_check_recon(&our_match(), &subtr);
        assert!(rep.tier1.iter().any(|f| f.field == "roster"));
    }

    #[test]
    fn empty_header_roster_matches_via_tracks() {
        // Empty-header replays carry no header players, but the tracks still bind
        // the real names — the roster check must use those, not `players`.
        let mut our = our_match();
        our.players = vec![];
        let rep = cross_check_recon(&our, &subtr_agreeing(5.0));
        assert!(
            !rep.tier1.iter().any(|f| f.field == "roster"),
            "track names match subtr; no roster breach expected: {:?}",
            rep.tier1
        );
        assert!(rep.tier1_ok(), "{:?}", rep.tier1);
    }

    /// A 6-frame match whose ball agrees everywhere except the `diverge` times
    /// (5000 uu off), with the given events — for the reset-window test.
    fn diverging_pair(events: Vec<Event>, diverge: &[f32]) -> (CanonicalMatch, SubtrGrid) {
        let times = [0.0_f32, 0.1, 0.2, 0.3, 0.4, 0.5];
        let car = |pri, x: f32| GridCar {
            pri,
            team: Some(0),
            p: v(x, 0.0, 17.0),
            v: v(0.0, 0.0, 0.0),
            boost: Some(100),
            rot: None,
        };
        let mut our = our_match();
        our.players = vec![];
        our.events = events;
        our.resampled.frames = times
            .iter()
            .map(|&t| GridFrame {
                t,
                ball: Some(Kin {
                    p: v(0.0, 0.0, 93.0),
                    v: v(0.0, 0.0, 0.0),
                }),
                cars: vec![car(1, -50.0), car(2, 50.0)],
            })
            .collect();
        let subtr = SubtrGrid {
            hz: 30.0,
            players: vec!["Alice".into(), "Bob".into()],
            frames: times
                .iter()
                .map(|&t| SubtrFrame {
                    time: t,
                    ball: Some(v(
                        if diverge.contains(&t) { 5000.0 } else { 0.0 },
                        0.0,
                        93.0,
                    )),
                    cars: vec![
                        SubtrCar {
                            player: "Alice".into(),
                            pos: Some(v(-50.0, 0.0, 17.0)),
                            boost: Some(255.0),
                        },
                        SubtrCar {
                            player: "Bob".into(),
                            pos: Some(v(50.0, 0.0, 17.0)),
                            boost: Some(255.0),
                        },
                    ],
                })
                .collect(),
        };
        (our, subtr)
    }

    #[test]
    fn post_goal_reset_frames_excluded_from_agree_rate() {
        // Goal at 0.25, kickoff at 0.45 → reset window [0.25, 0.45) covers the
        // diverging frames at 0.3 and 0.4. They must be excluded from the stats.
        let events = vec![
            Event::Goal {
                t: 0.25,
                scorer: None,
                team: Some(0),
            },
            Event::Kickoff { t: 0.45 },
        ];
        let (our, subtr) = diverging_pair(events, &[0.3, 0.4]);
        let rep = cross_check_recon(&our, &subtr);
        assert!(
            !rep.tier1.iter().any(|f| f.field == "ball_position"),
            "reset-window divergence must be excluded: {:?}",
            rep.tier1
        );
        assert!(rep.tier1_ok(), "{:?}", rep.tier1);
    }

    #[test]
    fn gameplay_divergence_still_trips_r1() {
        // Same divergence but with no goal events → no reset window → the tripwire
        // must still fire (we relax false positives, not real ones).
        let (our, subtr) = diverging_pair(vec![], &[0.3, 0.4]);
        let rep = cross_check_recon(&our, &subtr);
        assert!(
            rep.tier1.iter().any(|f| f.field == "ball_position"),
            "gameplay divergence must still fail R1"
        );
    }
}
