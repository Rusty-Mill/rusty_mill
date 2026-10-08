//! Behavior-isolating metric tests: hand-built frame sequences that exercise a
//! single metric and assert its raw value.

use replay_analyzer::model::{Event, Kin, Rot3, Vec3};
use replay_scoring::config::{Metric, ScoreConfig};
use replay_scoring::features::{CarView, FrameView, Third};
use replay_scoring::{metrics, roles};

fn v(x: f32, y: f32, z: f32) -> Vec3 {
    Vec3 { x, y, z }
}

#[allow(clippy::too_many_arguments)]
fn cv(pri: i32, ttb: f32, p: Vec3, pax: f32, goalside: bool, third: Third) -> CarView {
    CarView {
        pri,
        team: 0,
        p,
        v: v(0.0, 0.0, 0.0),
        pa: v(pax, 0.0, 17.0),
        boost: Some(50),
        valid_pos: true,
        dist_to_ball: 0.0,
        closing_speed: 0.0,
        time_to_ball: ttb,
        goalside,
        ball_third: third,
        rot: None,
        airborne: false,
        upright: false,
        attack_sign: 1,
    }
}

fn frame(t: f32, cars: Vec<CarView>) -> FrameView {
    FrameView {
        t,
        ball: Some(replay_analyzer::model::Kin {
            p: v(0.0, 0.0, 100.0),
            v: v(0.0, 0.0, 0.0),
        }),
        cars,
    }
}

fn raws_for(frames: &[FrameView], target: i32) -> std::collections::BTreeMap<Metric, Option<f32>> {
    let cfg = ScoreConfig::default();
    let roles = roles::assign(frames, &[0], &cfg);
    metrics::compute(frames, &roles, &[] as &[Event], target, 0, &cfg)
}

#[test]
fn candidate_metrics_compute() {
    // Ball at origin. Target (pri 1) sits behind (−Y), faces the ball (+Y), and
    // drives *backward* (−Y); teammate (pri 2) is ahead (+Y). So the target is
    // facing the ball and is in reverse — every frame.
    let car = |pri, p: Vec3, vel: Vec3, yaw: f32, pay: f32| CarView {
        pri,
        team: 0,
        p,
        v: vel,
        pa: v(0.0, pay, 17.0),
        boost: Some(50),
        valid_pos: true,
        dist_to_ball: 0.0,
        closing_speed: 0.0,
        time_to_ball: 1.0,
        goalside: true,
        ball_third: Third::Mid,
        rot: Some(Rot3 {
            pitch: 0.0,
            yaw,
            roll: 0.0,
        }),
        airborne: false,
        upright: true,
        attack_sign: 1,
    };
    let frames: Vec<_> = (0..10)
        .map(|i| FrameView {
            t: i as f32 * 0.1,
            ball: Some(Kin {
                p: v(0.0, 0.0, 100.0),
                v: v(0.0, 0.0, 0.0),
            }),
            cars: vec![
                car(
                    1,
                    v(0.0, -1000.0, 17.0),
                    v(0.0, -800.0, 0.0),
                    std::f32::consts::FRAC_PI_2,
                    -1000.0,
                ),
                car(
                    2,
                    v(0.0, 1000.0, 17.0),
                    v(0.0, 0.0, 0.0),
                    -std::f32::consts::FRAC_PI_2,
                    1000.0,
                ),
            ],
        })
        .collect();
    let raws = raws_for(&frames, 1);
    assert_eq!(raws[&Metric::FacingBallShare], Some(1.0)); // yaw points at the ball
    assert_eq!(raws[&Metric::ReverseDriving], Some(1.0)); // velocity opposes heading
}

#[test]
fn pace_and_agility_compute() {
    // Velocity ramps 0,300,…,1200 in +Y over 5 frames @ 0.1 s.
    let car = |vel: Vec3| CarView {
        pri: 1,
        team: 0,
        p: v(0.0, 0.0, 17.0),
        v: vel,
        pa: v(0.0, 0.0, 17.0),
        boost: Some(50),
        valid_pos: true,
        dist_to_ball: 0.0,
        closing_speed: 0.0,
        time_to_ball: 1.0,
        goalside: true,
        ball_third: Third::Mid,
        rot: None,
        airborne: false,
        upright: true,
        attack_sign: 1,
    };
    let frames: Vec<_> = (0..5)
        .map(|i| FrameView {
            t: i as f32 * 0.1,
            ball: Some(Kin {
                p: v(0.0, 0.0, 100.0),
                v: v(0.0, 0.0, 0.0),
            }),
            cars: vec![car(v(0.0, i as f32 * 300.0, 0.0))],
        })
        .collect();
    let raws = raws_for(&frames, 1);
    assert!((raws[&Metric::Pace].unwrap() - 600.0).abs() < 1.0); // mean of 0..1200
    assert!((raws[&Metric::Agility].unwrap() - 3000.0).abs() < 1.0); // Δv 300 / dt 0.1
}

#[test]
fn boost_starvation_measures_run_length_not_count() {
    let car = |boost: u8| CarView {
        pri: 1,
        team: 0,
        p: v(0.0, 0.0, 17.0),
        v: v(0.0, 0.0, 0.0),
        pa: v(0.0, 0.0, 17.0),
        boost: Some(boost),
        valid_pos: true,
        dist_to_ball: 0.0,
        closing_speed: 0.0,
        time_to_ball: 1.0,
        goalside: true,
        ball_third: Third::Mid,
        rot: None,
        airborne: false,
        upright: true,
        attack_sign: 1,
    };
    // Zero-boost runs of 3 frames and 1 frame → mean 2 frames × 0.1 s = 0.2 s.
    let boosts = [50u8, 50, 0, 0, 0, 50, 0, 50, 50, 50];
    let frames: Vec<_> = boosts
        .iter()
        .enumerate()
        .map(|(i, &b)| FrameView {
            t: i as f32 * 0.1,
            ball: Some(Kin {
                p: v(0.0, 0.0, 100.0),
                v: v(0.0, 0.0, 0.0),
            }),
            cars: vec![car(b)],
        })
        .collect();
    let raws = raws_for(&frames, 1);
    assert!((raws[&Metric::BoostStarvation].unwrap() - 0.2).abs() < 1e-4);

    // Never empty → 0.0 (not stranded), distinct from "frequently dips to 0".
    let full: Vec<_> = (0..5)
        .map(|i| FrameView {
            t: i as f32 * 0.1,
            ball: Some(Kin {
                p: v(0.0, 0.0, 100.0),
                v: v(0.0, 0.0, 0.0),
            }),
            cars: vec![car(50)],
        })
        .collect();
    assert_eq!(raws_for(&full, 1)[&Metric::BoostStarvation], Some(0.0));
}

#[test]
fn double_commit_fires_when_both_press() {
    // Both cars pressuring (ttb < press_ttb=1.2) every frame -> rate 1.0.
    let frames: Vec<_> = (0..10)
        .map(|i| {
            frame(
                i as f32 * 0.1,
                vec![
                    cv(1, 0.5, v(0.0, -100.0, 17.0), 0.0, true, Third::Mid),
                    cv(2, 0.6, v(0.0, 100.0, 17.0), 0.0, true, Third::Mid),
                ],
            )
        })
        .collect();
    let raws = raws_for(&frames, 1);
    assert_eq!(raws[&Metric::DoubleCommitRate], Some(1.0));
}

#[test]
fn central_support_high_when_target_supports_centrally() {
    // pri2 leads (1st man), pri1 is 2nd man sitting central + goal-side.
    let frames: Vec<_> = (0..10)
        .map(|i| {
            frame(
                i as f32 * 0.1,
                vec![
                    cv(1, 1.5, v(0.0, -2000.0, 17.0), 300.0, true, Third::Mid), // 2nd man, central, goalside
                    cv(2, 0.5, v(0.0, 0.0, 17.0), 0.0, false, Third::Mid),      // 1st man
                ],
            )
        })
        .collect();
    let raws = raws_for(&frames, 1);
    assert_eq!(raws[&Metric::CentralSupportFraction], Some(1.0));
    // And it is NOT counted as a double-commit (only one car pressuring).
    assert_eq!(raws[&Metric::DoubleCommitRate], Some(0.0));
}

#[test]
fn goalside_first_tracks_defensive_goalside_share() {
    // Target is 1st man in the defensive third; goal-side in 7/10 frames.
    let frames: Vec<_> = (0..10)
        .map(|i| {
            let goalside = i < 7;
            frame(
                i as f32 * 0.1,
                vec![
                    cv(1, 0.5, v(0.0, -3000.0, 17.0), 0.0, goalside, Third::Def), // 1st man on defense
                    cv(2, 1.5, v(0.0, -1000.0, 17.0), 0.0, true, Third::Def),
                ],
            )
        })
        .collect();
    let raws = raws_for(&frames, 1);
    assert_eq!(raws[&Metric::GoalsideDiscipline1st], Some(0.7));
}

#[test]
fn possession_retention_from_touch_chains() {
    let cfg = ScoreConfig::default();
    let roles = roles::assign(&[], &[0], &cfg);
    // pri 1 (team 0) touches: kept then lost; one trailing touch by pri 1.
    let events = vec![
        Event::Touch {
            t: 0.0,
            pri: 1,
            player: None,
            team: Some(0),
        },
        Event::Touch {
            t: 1.0,
            pri: 9,
            player: None,
            team: Some(0),
        }, // same team -> kept
        Event::Touch {
            t: 2.0,
            pri: 1,
            player: None,
            team: Some(0),
        },
        Event::Touch {
            t: 3.0,
            pri: 7,
            player: None,
            team: Some(1),
        }, // other team -> lost
    ];
    let raws = metrics::compute(&[], &roles, &events, 1, 0, &cfg);
    // 2 target touches with a successor; 1 kept -> 0.5.
    assert_eq!(raws[&Metric::PossessionRetention], Some(0.5));
}

fn rot3(pitch: f32, yaw: f32, roll: f32) -> Rot3 {
    Rot3 { pitch, yaw, roll }
}

/// A neutral car at `p`: full boost, valid, ttb 1.0, no rotation, grounded.
fn mkcar(pri: i32, team: i32, p: Vec3) -> CarView {
    CarView {
        pri,
        team,
        p,
        v: v(0.0, 0.0, 0.0),
        pa: p,
        boost: Some(100),
        valid_pos: true,
        dist_to_ball: 0.0,
        closing_speed: 0.0,
        time_to_ball: 1.0,
        goalside: false,
        ball_third: Third::Mid,
        rot: None,
        airborne: false,
        upright: false,
        attack_sign: 1,
    }
}

fn framed(t: f32, ball: Option<Kin>, cars: Vec<CarView>) -> FrameView {
    FrameView { t, ball, cars }
}

const FACE_Y: f32 = std::f32::consts::FRAC_PI_2; // yaw facing +Y

#[test]
fn challenge_timing_scores_controlled_vs_uncontrolled_arrival() {
    let cfg = ScoreConfig::default();
    let ball = Kin {
        p: v(0.0, 0.0, 100.0),
        v: v(0.0, 0.0, 0.0),
    };
    // 2v2: target pri1 & opp pri3 are their teams' 1st men, both ~507uu from the
    // ball (inside the 900uu contest radius), arriving level (ttb equal).
    let build = |yaw: f32| -> Vec<FrameView> {
        (0..20)
            .map(|i| {
                let mut t1 = mkcar(1, 0, v(0.0, -500.0, 17.0));
                t1.time_to_ball = 0.5;
                t1.rot = Some(rot3(0.0, yaw, 0.0));
                let mut t2 = mkcar(2, 0, v(0.0, -3000.0, 17.0));
                t2.time_to_ball = 1.5;
                let mut o3 = mkcar(3, 1, v(0.0, 500.0, 17.0));
                o3.time_to_ball = 0.5;
                let mut o4 = mkcar(4, 1, v(0.0, 3000.0, 17.0));
                o4.time_to_ball = 1.5;
                framed(i as f32 * 0.1, Some(ball), vec![t1, t2, o3, o4])
            })
            .collect()
    };
    // Facing the ball (+Y): with boost, level timing -> a controlled arrival.
    let good = build(FACE_Y);
    let roles = roles::assign(&good, &[0, 1], &cfg);
    let raws = metrics::compute(&good, &roles, &[] as &[Event], 1, 0, &cfg);
    assert_eq!(raws[&Metric::ChallengeTiming], Some(1.0));
    // Facing away (-Y): same contest, but uncontrolled -> 0.
    let bad = build(-FACE_Y);
    let roles = roles::assign(&bad, &[0, 1], &cfg);
    let raws = metrics::compute(&bad, &roles, &[] as &[Event], 1, 0, &cfg);
    assert_eq!(raws[&Metric::ChallengeTiming], Some(0.0));
}

#[test]
fn first_touch_value_rewards_forward_penalizes_booms() {
    let cfg = ScoreConfig::default();
    // Target pri1 is 1st man; teammate pri2 trails.
    let car = |t: f32, ballv: Vec3| {
        let mut c = mkcar(1, 0, v(0.0, -1000.0, 17.0));
        c.time_to_ball = 0.5;
        let mut mate = mkcar(2, 0, v(0.0, -3000.0, 17.0));
        mate.time_to_ball = 1.5;
        framed(
            t,
            Some(Kin {
                p: v(0.0, 0.0, 100.0),
                v: ballv,
            }),
            vec![c, mate],
        )
    };
    // Touch 1: controlled forward push (+Y, below boom speed) -> +1.
    // Touch 2: backward boom (-Y, above boom speed) -> -1 - 1 = -2. Mean -0.5.
    let frames = vec![
        car(0.0, v(0.0, 1000.0, 0.0)),
        car(1.0, v(0.0, -5000.0, 0.0)),
    ];
    let events = vec![
        Event::Touch {
            t: 0.0,
            pri: 1,
            player: None,
            team: Some(0),
        },
        Event::Touch {
            t: 1.0,
            pri: 1,
            player: None,
            team: Some(0),
        },
    ];
    let roles = roles::assign(&frames, &[0], &cfg);
    let raws = metrics::compute(&frames, &roles, &events, 1, 0, &cfg);
    assert_eq!(raws[&Metric::FirstTouchValue], Some(-0.5));
}

#[test]
fn transition_readiness_credits_stepping_up_after_a_flip() {
    let cfg = ScoreConfig::default();
    let ball = Kin {
        p: v(0.0, 0.0, 100.0),
        v: v(0.0, 0.0, 0.0),
    };
    // pri1 (team 0) is always 2nd man (teammate pri2 is always closer to the
    // ball). Its time-to-ball dips mid-match then rises, so over the default
    // 1.5 s window it *closes* after the first flip but *drifts* after the second.
    let ttb = |t: f32| if (1.25..3.0).contains(&t) { 1.0 } else { 2.0 };
    let frames: Vec<_> = (0..=36)
        .map(|i| {
            let t = i as f32 * 0.1;
            let mut me = mkcar(1, 0, v(0.0, -1000.0, 17.0));
            me.time_to_ball = ttb(t);
            let mut mate = mkcar(2, 0, v(0.0, 100.0, 17.0));
            mate.time_to_ball = 0.3;
            framed(t, Some(ball), vec![me, mate])
        })
        .collect();
    let events = vec![
        Event::Possession {
            team: 0,
            start: 0.0,
            end: 1.0,
            touches: 1,
        },
        Event::Possession {
            team: 1,
            start: 1.0,
            end: 2.0,
            touches: 1,
        },
        Event::Possession {
            team: 0,
            start: 2.0,
            end: 3.0,
            touches: 1,
        },
    ];
    // Flip t=1.0: ttb 2.0 -> 1.0 by t=2.5 (closes) -> ready.
    // Flip t=2.0: ttb 1.0 -> 2.0 by t=3.5 (drifts)  -> not ready.  => 0.5
    let roles = roles::assign(&frames, &[0], &cfg);
    let raws = metrics::compute(&frames, &roles, &events, 1, 0, &cfg);
    assert_eq!(raws[&Metric::TransitionReadiness], Some(0.5));
}

#[test]
fn recovery_speed_times_airborne_to_wheels_down_facing() {
    let cfg = ScoreConfig::default();
    let ball = Kin {
        p: v(0.0, 0.0, 100.0),
        v: v(0.0, 0.0, 0.0),
    };
    // Airborne through t=0.4, grounded-but-not-upright t=0.5..0.7, then fully
    // recovered (upright + facing ball) from t=0.8 -> recovery = 0.4 s.
    let frames: Vec<_> = (0..=10)
        .map(|i| {
            let t = i as f32 * 0.1;
            let airborne = i <= 4;
            let recovered = i >= 8;
            let mut me = mkcar(1, 0, v(0.0, -1000.0, if airborne { 600.0 } else { 17.0 }));
            me.airborne = airborne;
            me.upright = recovered;
            me.rot = Some(rot3(0.0, FACE_Y, 0.0)); // always faces +Y (the ball)
            framed(t, Some(ball), vec![me])
        })
        .collect();
    let roles = roles::assign(&frames, &[0], &cfg);
    let raws = metrics::compute(&frames, &roles, &[] as &[Event], 1, 0, &cfg);
    let rs = raws[&Metric::RecoverySpeed].expect("a recovery was detected");
    assert!(
        (rs - 0.4).abs() < 1e-3,
        "recovery_speed = {rs}, expected ~0.4"
    );
}

#[test]
fn boost_management_blends_supersonic_and_collection() {
    let cfg = ScoreConfig::default();
    // 10 frames: first 5 slow at 0 boost, last 5 supersonic at full boost. The
    // single 0->100 jump at frame 5 is the only collection (+100 over ~1 s).
    let frames: Vec<_> = (0..10)
        .map(|i| {
            let mut c = mkcar(1, 0, v(0.0, 0.0, 17.0));
            if i >= 5 {
                c.v = v(2300.0, 0.0, 0.0); // supersonic (> 2200 uu/s)
                c.boost = Some(255);
            } else {
                c.v = v(0.0, 0.0, 0.0);
                c.boost = Some(0);
            }
            framed(i as f32 * 0.1, None, vec![c])
        })
        .collect();
    let roles = roles::assign(&frames, &[0], &cfg);
    let raws = metrics::compute(&frames, &roles, &[] as &[Event], 1, 0, &cfg);
    // frac_supersonic = 5/10 = 0.5; collected = 100 over ~1 s -> collect_norm = 1.0
    // (capped); composite = 0.5*0.5 + 0.5*1.0 = 0.75.
    let bm = raws[&Metric::BoostManagement].expect("boost computable");
    assert!((bm - 0.75).abs() < 1e-3, "boost_management = {bm}");
}

#[test]
fn aerial_presence_is_airborne_fraction() {
    let cfg = ScoreConfig::default();
    // 3 of 10 frames airborne -> 0.3.
    let frames: Vec<_> = (0..10)
        .map(|i| {
            let mut c = mkcar(1, 0, v(0.0, 0.0, 17.0));
            c.airborne = i < 3;
            framed(i as f32 * 0.1, None, vec![c])
        })
        .collect();
    let roles = roles::assign(&frames, &[0], &cfg);
    let raws = metrics::compute(&frames, &roles, &[] as &[Event], 1, 0, &cfg);
    assert_eq!(raws[&Metric::AerialPresence], Some(0.3));
}

#[test]
fn dangerous_turnover_weights_by_field_depth() {
    use replay_analyzer::field::BACK_WALL_Y;
    let cfg = ScoreConfig::default();
    // A single car for pri 1 (team 0, attacking +Y). The metric only needs the
    // ball position at each touch frame and the car's attack_sign.
    let mk = |t: f32, ball_y: f32| FrameView {
        t,
        ball: Some(Kin {
            p: v(0.0, ball_y, 100.0),
            v: v(0.0, 0.0, 0.0),
        }),
        cars: vec![cv(1, 1.0, v(0.0, 0.0, 17.0), 0.0, true, Third::Mid)],
    };
    // Two touches by pri 1 then an opponent touch (team 1) after each — both are
    // turnovers. First giveaway is at the back wall (own half, max danger ~1),
    // second at midfield (danger ~0).
    let frames = vec![mk(0.0, -BACK_WALL_Y), mk(1.0, 0.0)];
    let events = vec![
        Event::Touch {
            t: 0.0,
            pri: 1,
            team: Some(0),
            player: None,
        },
        Event::Touch {
            t: 0.5,
            pri: 9,
            team: Some(1),
            player: None,
        },
        Event::Touch {
            t: 1.0,
            pri: 1,
            team: Some(0),
            player: None,
        },
        Event::Touch {
            t: 1.5,
            pri: 9,
            team: Some(1),
            player: None,
        },
    ];
    let roles = roles::assign(&frames, &[0, 1], &cfg);
    let raws = metrics::compute(&frames, &roles, &events, 1, 0, &cfg);
    // 2 followed touches, both turnovers; danger ≈ (1.0 + 0.0) / 2 = 0.5.
    let dt = raws[&Metric::DangerousTurnover].expect("computable");
    assert!((dt - 0.5).abs() < 0.02, "expected ~0.5, got {dt}");
}

#[test]
fn shot_angle_measures_off_target_degrees() {
    let cfg = ScoreConfig::default();
    // A single car for pri 1 (team 0, attacking +Y); the metric reads the ball's
    // position/velocity at each touch frame and the car's attack_sign.
    let mk = |t: f32, bp: Vec3, bv: Vec3| FrameView {
        t,
        ball: Some(Kin { p: bp, v: bv }),
        cars: vec![cv(1, 1.0, v(0.0, 0.0, 17.0), 0.0, true, Third::Mid)],
    };
    let diag = std::f32::consts::FRAC_1_SQRT_2 * 2000.0;
    let frames = vec![
        // Strike 1: offensive half, dead centre, straight at goal -> 0°.
        mk(0.0, v(0.0, 2560.0, 100.0), v(0.0, 2000.0, 0.0)),
        // Strike 2: same spot, 45° wide of the goal line -> 45°.
        mk(1.0, v(0.0, 2560.0, 100.0), v(diag, diag, 0.0)),
        // Defensive-half clear: fast and goalward but never a shot.
        mk(2.0, v(0.0, -2560.0, 100.0), v(0.0, 3000.0, 0.0)),
        // Soft goalward tap: below the strike floor.
        mk(3.0, v(0.0, 2560.0, 100.0), v(0.0, 800.0, 0.0)),
    ];
    let touch = |t: f32| Event::Touch {
        t,
        pri: 1,
        team: Some(0),
        player: None,
    };
    let events = vec![touch(0.0), touch(1.0), touch(2.0), touch(3.0)];
    let roles = roles::assign(&frames, &[0], &cfg);
    let raws = metrics::compute(&frames, &roles, &events, 1, 0, &cfg);
    // Only the two real strikes qualify; mean of 0° and 45° = 22.5°.
    let sa = raws[&Metric::ShotAngle].expect("computable");
    assert!((sa - 22.5).abs() < 0.1, "expected ~22.5°, got {sa}");
}

#[test]
fn whiff_rate_counts_missed_swings() {
    let cfg = ScoreConfig::default();
    let ball = Kin {
        p: v(0.0, 0.0, 100.0),
        v: v(0.0, 0.0, 0.0),
    };
    // The car's distance-to-ball and approach reads are set directly; a swing is
    // judged off the frame that enters striking range (dist ≤ 320).
    let mk = |t: f32, dist: f32, closing: f32| {
        let mut c = mkcar(1, 0, v(0.0, -dist, 17.0));
        c.dist_to_ball = dist;
        c.closing_speed = closing;
        c.v = v(0.0, closing, 0.0);
        framed(t, Some(ball), vec![c])
    };
    let frames = vec![
        // Swing 1: fast ball-directed entry, touch lands at t=0.3 -> hit.
        mk(0.0, 500.0, 1000.0),
        mk(0.1, 500.0, 1000.0),
        mk(0.2, 300.0, 1000.0),
        mk(0.3, 150.0, 1000.0),
        mk(0.4, 300.0, 1000.0),
        mk(0.5, 500.0, 1000.0),
        // Swing 2: same approach, no touch -> a whiff.
        mk(1.0, 300.0, 1000.0),
        mk(1.1, 200.0, 1000.0),
        mk(1.2, 500.0, 1000.0),
        // Slow drift into range (closing below the floor): not an attempt.
        mk(2.0, 310.0, 200.0),
        mk(2.1, 250.0, 200.0),
        mk(2.2, 500.0, 200.0),
    ];
    let events = vec![Event::Touch {
        t: 0.3,
        pri: 1,
        team: Some(0),
        player: None,
    }];
    let roles = roles::assign(&frames, &[0], &cfg);
    let raws = metrics::compute(&frames, &roles, &events, 1, 0, &cfg);
    // Two real swings, one missed; the slow drift never counts -> 1/2.
    assert_eq!(raws[&Metric::WhiffRate], Some(0.5));
}
