//! Per-detector unit tests over hand-built grids/events — the style of
//! `replay_analyzer::analyze::events` tests: each detector is exercised in
//! isolation so a synthetic motion that isolates one mechanic asserts exactly
//! that skill fires (and bordering non-examples do not).

use std::collections::BTreeMap;

use replay_analyzer::field;
use replay_analyzer::model::{
    Event, GridCar, GridFrame, Kin, PlayerTrack, Resampled, Rot3, TrackGap, TrackSample, Vec3,
};
use replay_skills::detect::{
    aerial_candidates, aerials, air_dribbles, boost_steals, ceiling_candidates, ceiling_plays,
    demos, double_touches, dribble_candidates, flick_candidates, flicks, ground_dribbles,
    kickoff_first_touches, power_shot_candidates, power_shots, redirect_candidates, redirects,
    supersonic, supersonic_candidates, wall_plays,
};
use replay_skills::{Skill, SkillConfig};

fn v(x: f32, y: f32, z: f32) -> Vec3 {
    Vec3 { x, y, z }
}

fn kin(p: Vec3, vel: Vec3) -> Kin {
    Kin { p, v: vel }
}

fn car(pri: i32, team: i32, p: Vec3, vel: Vec3) -> GridCar {
    GridCar {
        pri,
        team: Some(team),
        p,
        v: vel,
        boost: Some(50),
        rot: None,
    }
}

fn frame(t: f32, ball: Option<Kin>, cars: Vec<GridCar>) -> GridFrame {
    GridFrame { t, ball, cars }
}

fn grid(frames: Vec<GridFrame>) -> Resampled {
    let mut signs = BTreeMap::new();
    signs.insert(0, 1);
    signs.insert(1, -1);
    Resampled {
        hz: 30.0,
        team_attack_sign: signs,
        frames,
    }
}

fn track(pri: i32, team: i32) -> PlayerTrack {
    PlayerTrack {
        player: format!("p{pri}"),
        pri,
        team: Some(team),
        num_segments: 1,
        samples: vec![],
        gaps: vec![],
    }
}

fn touch(t: f32, pri: i32, team: i32) -> Event {
    Event::Touch {
        t,
        pri,
        player: Some(format!("p{pri}")),
        team: Some(team),
    }
}

fn cfg() -> SkillConfig {
    SkillConfig::default()
}

fn only(insts: Vec<replay_skills::SkillInstance>, skill: Skill) -> replay_skills::SkillInstance {
    assert_eq!(insts.len(), 1, "expected exactly one {:?}", skill);
    assert_eq!(insts[0].skill, skill);
    insts.into_iter().next().unwrap()
}

#[test]
fn supersonic_emits_one_instance_per_sustained_run() {
    // A sustained sprint (>= 0.5s) then a slow frame closes the single run.
    let fast = |t: f32| {
        frame(
            t,
            None,
            vec![car(1, 0, v(0.0, 0.0, 17.0), v(2300.0, 0.0, 0.0))],
        )
    };
    let frames = vec![
        fast(0.0),
        fast(0.3),
        fast(0.6),
        frame(
            0.7,
            None,
            vec![car(1, 0, v(0.0, 0.0, 17.0), v(500.0, 0.0, 0.0))],
        ),
    ];
    let i = only(
        supersonic(&grid(frames), &[track(1, 0)], &cfg()),
        Skill::Supersonic,
    );
    assert_eq!(i.pri, 1);
    assert!(i.confidence >= 0.99);
}

#[test]
fn supersonic_ignores_momentary_blip() {
    // A single fast frame (e.g. a bump) is below the sustained-sprint duration.
    let frames = vec![
        frame(
            0.0,
            None,
            vec![car(1, 0, v(0.0, 0.0, 17.0), v(2300.0, 0.0, 0.0))],
        ),
        frame(
            0.1,
            None,
            vec![car(1, 0, v(0.0, 0.0, 17.0), v(500.0, 0.0, 0.0))],
        ),
    ];
    assert!(supersonic(&grid(frames), &[track(1, 0)], &cfg()).is_empty());
}

#[test]
fn no_supersonic_below_threshold() {
    let frames = vec![frame(
        0.0,
        None,
        vec![car(1, 0, v(0.0, 0.0, 17.0), v(1000.0, 0.0, 0.0))],
    )];
    assert!(supersonic(&grid(frames), &[track(1, 0)], &cfg()).is_empty());
}

#[test]
fn aerial_detected_when_car_and_ball_elevated_at_touch() {
    let ball = kin(v(0.0, 0.0, 900.0), v(0.0, 0.0, 0.0));
    let frames = vec![frame(
        1.0,
        Some(ball),
        vec![car(1, 0, v(0.0, 0.0, 850.0), v(0.0, 0.0, 0.0))],
    )];
    let i = only(
        aerials(&grid(frames), &[touch(1.0, 1, 0)], &cfg()),
        Skill::Aerial,
    );
    assert_eq!(i.pri, 1);
    // car_z (850) near high_aerial_height (900) -> high confidence.
    assert!(i.confidence > 0.9, "got {}", i.confidence);
}

#[test]
fn no_aerial_for_ground_touch() {
    let ball = kin(v(0.0, 0.0, 93.0), v(0.0, 0.0, 0.0));
    let frames = vec![frame(
        1.0,
        Some(ball),
        vec![car(1, 0, v(0.0, 0.0, 17.0), v(0.0, 0.0, 0.0))],
    )];
    assert!(aerials(&grid(frames), &[touch(1.0, 1, 0)], &cfg()).is_empty());
}

#[test]
fn aerial_candidates_emit_car_height_for_every_touch_pre_gate() {
    // Two touches: one airborne (would pass the aerial gate), one a ground touch
    // (the detector drops it). Candidate-mode must emit the car height for *both*
    // — that sub-floor reading is exactly what lets the calibrator fit the floor.
    let high = frame(
        1.0,
        Some(kin(v(0.0, 0.0, 900.0), v(0.0, 0.0, 0.0))),
        vec![car(1, 0, v(0.0, 0.0, 850.0), v(0.0, 0.0, 0.0))],
    );
    let low = frame(
        2.0,
        Some(kin(v(0.0, 0.0, 93.0), v(0.0, 0.0, 0.0))),
        vec![car(1, 0, v(0.0, 0.0, 17.0), v(0.0, 0.0, 0.0))],
    );
    let g = grid(vec![high, low]);
    let events = [touch(1.0, 1, 0), touch(2.0, 1, 0)];

    // Gated detector keeps only the airborne touch...
    assert_eq!(aerials(&g, &events, &cfg()).len(), 1);
    // ...but candidate-mode reports both car heights, including the ground touch.
    let cands = aerial_candidates(&g, &events, &cfg());
    assert_eq!(cands, vec![850.0, 17.0]);
}

#[test]
fn air_dribble_chains_consecutive_aerial_touches() {
    let high = |t: f32| {
        frame(
            t,
            Some(kin(v(0.0, 0.0, 800.0), v(0.0, 0.0, 0.0))),
            vec![car(1, 0, v(0.0, 0.0, 780.0), v(0.0, 0.0, 0.0))],
        )
    };
    let frames = vec![high(0.0), high(0.5), high(1.0)];
    let events = vec![touch(0.0, 1, 0), touch(0.5, 1, 0), touch(1.0, 1, 0)];
    let i = only(
        air_dribbles(&grid(frames), &events, &cfg()),
        Skill::AirDribble,
    );
    assert_eq!(i.pri, 1);
    assert!(i.detail.contains("touches=3"));
}

#[test]
fn double_touch_two_quick_elevated_contacts() {
    let elevated = |t: f32| {
        frame(
            t,
            Some(kin(v(0.0, 0.0, 420.0), v(0.0, 0.0, 0.0))),
            vec![car(1, 0, v(0.0, 0.0, 300.0), v(0.0, 0.0, 0.0))],
        )
    };
    let frames = vec![elevated(0.0), elevated(0.3)];
    let events = vec![touch(0.0, 1, 0), touch(0.3, 1, 0)];
    let i = only(
        double_touches(&grid(frames), &events, &cfg()),
        Skill::DoubleTouch,
    );
    assert_eq!(i.pri, 1);
    assert!(i.detail.contains("gap=0.30"));
}

#[test]
fn double_touch_ignores_low_ball_and_intervening_opponent() {
    // Ball low at the second contact → ground micro-touches, not a double touch.
    let low = |t: f32| {
        frame(
            t,
            Some(kin(v(0.0, 0.0, 120.0), v(0.0, 0.0, 0.0))),
            vec![car(1, 0, v(0.0, 0.0, 17.0), v(0.0, 0.0, 0.0))],
        )
    };
    assert!(double_touches(
        &grid(vec![low(0.0), low(0.3)]),
        &[touch(0.0, 1, 0), touch(0.3, 1, 0)],
        &cfg()
    )
    .is_empty());

    // An opponent touch between the two breaks the same-player pair.
    let hi = |t: f32| {
        frame(
            t,
            Some(kin(v(0.0, 0.0, 420.0), v(0.0, 0.0, 0.0))),
            vec![car(1, 0, v(0.0, 0.0, 300.0), v(0.0, 0.0, 0.0))],
        )
    };
    let frames = vec![hi(0.0), hi(0.15), hi(0.3)];
    let events = vec![touch(0.0, 1, 0), touch(0.15, 2, 1), touch(0.3, 1, 0)];
    assert!(double_touches(&grid(frames), &events, &cfg()).is_empty());
}

#[test]
fn ceiling_play_detected_near_ceiling() {
    let z = field::CEILING_Z - 50.0;
    let frames = vec![
        frame(0.0, None, vec![car(1, 0, v(0.0, 0.0, z), v(0.0, 0.0, 0.0))]),
        frame(
            0.15,
            None,
            vec![car(1, 0, v(0.0, 0.0, z), v(0.0, 0.0, 0.0))],
        ),
    ];
    let i = only(
        ceiling_plays(&grid(frames), &[track(1, 0)], &cfg()),
        Skill::CeilingPlay,
    );
    assert_eq!(i.pri, 1);
}

#[test]
fn wall_play_detected_up_the_side_wall() {
    let x = field::SIDE_WALL_X - 50.0;
    let ball = kin(v(x, 0.0, 420.0), v(0.0, 0.0, 0.0));
    let frames = vec![frame(
        2.0,
        Some(ball),
        vec![car(1, 0, v(x, 0.0, 400.0), v(0.0, 0.0, 0.0))],
    )];
    let i = only(
        wall_plays(&grid(frames), &[touch(2.0, 1, 0)], &cfg()),
        Skill::WallPlay,
    );
    assert!(i.detail.contains("side"));
}

#[test]
fn no_wall_play_at_ground_near_wall() {
    // Near the wall plane but at ground level: driving, not wall-riding.
    let x = field::SIDE_WALL_X - 50.0;
    let ball = kin(v(x, 0.0, 93.0), v(0.0, 0.0, 0.0));
    let frames = vec![frame(
        2.0,
        Some(ball),
        vec![car(1, 0, v(x, 0.0, 17.0), v(0.0, 0.0, 0.0))],
    )];
    assert!(wall_plays(&grid(frames), &[touch(2.0, 1, 0)], &cfg()).is_empty());
}

#[test]
fn ground_dribble_detected_for_sustained_carry() {
    // Ball balanced on the roof, near ground, for over the min duration.
    let mut frames = Vec::new();
    let mut t = 0.0;
    while t <= 1.0 + 1e-3 {
        frames.push(frame(
            t,
            Some(kin(v(0.0, 0.0, 140.0), v(0.0, 0.0, 0.0))),
            vec![car(1, 0, v(0.0, 0.0, 17.0), v(0.0, 0.0, 0.0))],
        ));
        t += 0.1;
    }
    let i = only(
        ground_dribbles(&grid(frames), &[track(1, 0)], &cfg()),
        Skill::GroundDribble,
    );
    assert_eq!(i.pri, 1);
}

#[test]
fn no_ground_dribble_when_too_brief() {
    let frames = vec![
        frame(
            0.0,
            Some(kin(v(0.0, 0.0, 140.0), v(0.0, 0.0, 0.0))),
            vec![car(1, 0, v(0.0, 0.0, 17.0), v(0.0, 0.0, 0.0))],
        ),
        frame(
            0.2,
            Some(kin(v(0.0, 0.0, 140.0), v(0.0, 0.0, 0.0))),
            vec![car(1, 0, v(0.0, 0.0, 17.0), v(0.0, 0.0, 0.0))],
        ),
    ];
    assert!(ground_dribbles(&grid(frames), &[track(1, 0)], &cfg()).is_empty());
}

#[test]
fn run_candidates_emit_durations_of_sub_floor_runs() {
    // Each run detector's candidate twin emits the duration of *every* run, including
    // short ones the gated detector drops — exactly the sub-floor tail a floor fit
    // needs. Gated output stays empty for these brief runs.

    // Supersonic: fast for 0.2s (< supersonic_min_duration_s 0.5), then slow.
    let ss = grid(vec![
        frame(
            0.0,
            None,
            vec![car(1, 0, v(0.0, 0.0, 17.0), v(2300.0, 0.0, 0.0))],
        ),
        frame(
            0.1,
            None,
            vec![car(1, 0, v(0.0, 0.0, 17.0), v(2300.0, 0.0, 0.0))],
        ),
        frame(
            0.2,
            None,
            vec![car(1, 0, v(0.0, 0.0, 17.0), v(2300.0, 0.0, 0.0))],
        ),
        frame(
            0.3,
            None,
            vec![car(1, 0, v(0.0, 0.0, 17.0), v(500.0, 0.0, 0.0))],
        ),
    ]);
    assert!(supersonic(&ss, &[track(1, 0)], &cfg()).is_empty());
    let c = supersonic_candidates(&ss, &cfg());
    assert_eq!(c.len(), 1);
    assert!((c[0] - 0.2).abs() < 1e-3, "0.2s run, got {}", c[0]);

    // Ceiling: near the ceiling for 0.05s (< ceiling_min_duration_s 0.1).
    let z = field::CEILING_Z - 50.0;
    let cp = grid(vec![
        frame(0.0, None, vec![car(1, 0, v(0.0, 0.0, z), v(0.0, 0.0, 0.0))]),
        frame(
            0.05,
            None,
            vec![car(1, 0, v(0.0, 0.0, z), v(0.0, 0.0, 0.0))],
        ),
    ]);
    assert!(ceiling_plays(&cp, &[track(1, 0)], &cfg()).is_empty());
    let c = ceiling_candidates(&cp, &cfg());
    assert_eq!(c.len(), 1);
    assert!((c[0] - 0.05).abs() < 1e-3, "0.05s run, got {}", c[0]);

    // Ground dribble: carried for 0.2s (< dribble_min_duration_s 0.75).
    let low = |t: f32| {
        frame(
            t,
            Some(kin(v(0.0, 0.0, 140.0), v(0.0, 0.0, 0.0))),
            vec![car(1, 0, v(0.0, 0.0, 17.0), v(0.0, 0.0, 0.0))],
        )
    };
    let gd = grid(vec![low(0.0), low(0.2)]);
    assert!(ground_dribbles(&gd, &[track(1, 0)], &cfg()).is_empty());
    let c = dribble_candidates(&gd, &cfg());
    assert_eq!(c.len(), 1);
    assert!((c[0] - 0.2).abs() < 1e-3, "0.2s run, got {}", c[0]);
}

#[test]
fn flick_detected_on_upward_release_of_carried_ball() {
    // Frame 0: ball sitting low and slow on the car. Frame 1 (the touch): ball
    // shoots upward.
    let frames = vec![
        frame(
            0.9,
            Some(kin(v(0.0, 0.0, 150.0), v(0.0, 0.0, 0.0))),
            vec![car(1, 0, v(0.0, 0.0, 17.0), v(0.0, 0.0, 0.0))],
        ),
        frame(
            1.0,
            Some(kin(v(0.0, 0.0, 160.0), v(0.0, 0.0, 1200.0))),
            vec![car(1, 0, v(0.0, 0.0, 17.0), v(0.0, 0.0, 0.0))],
        ),
    ];
    let i = only(
        flicks(&grid(frames), &[touch(1.0, 1, 0)], &cfg()),
        Skill::Flick,
    );
    assert!(i.detail.contains("up_v=1200"));
}

#[test]
fn power_shot_detected_for_fast_goalward_strike() {
    // Team 0 attacks +Y (sign +1): a fast +Y ball is goalward.
    let frames = vec![frame(
        3.0,
        Some(kin(v(0.0, 0.0, 100.0), v(0.0, 2600.0, 0.0))),
        vec![car(1, 0, v(0.0, -200.0, 17.0), v(0.0, 0.0, 0.0))],
    )];
    let i = only(
        power_shots(&grid(frames), &[touch(3.0, 1, 0)], &cfg()),
        Skill::PowerShot,
    );
    assert!(i.detail.contains("speed=2600"));
}

#[test]
fn power_shot_ignores_fast_ball_away_from_goal() {
    // Same speed but pointing at our own goal (-Y) for team 0.
    let frames = vec![frame(
        3.0,
        Some(kin(v(0.0, 0.0, 100.0), v(0.0, -2600.0, 0.0))),
        vec![car(1, 0, v(0.0, 200.0, 17.0), v(0.0, 0.0, 0.0))],
    )];
    assert!(power_shots(&grid(frames), &[touch(3.0, 1, 0)], &cfg()).is_empty());
}

#[test]
fn redirect_detected_on_sharp_direction_change() {
    // Ball comes in along -X fast, leaves along +Y fast: ~90° turn, goalward.
    let frames = vec![
        frame(
            4.9,
            Some(kin(v(0.0, 0.0, 200.0), v(-1500.0, 0.0, 0.0))),
            vec![car(1, 0, v(0.0, 0.0, 180.0), v(0.0, 0.0, 0.0))],
        ),
        frame(
            5.0,
            Some(kin(v(0.0, 0.0, 200.0), v(0.0, 1500.0, 0.0))),
            vec![car(1, 0, v(0.0, 0.0, 180.0), v(0.0, 0.0, 0.0))],
        ),
    ];
    let i = only(
        redirects(&grid(frames), &[touch(5.0, 1, 0)], &cfg()),
        Skill::Redirect,
    );
    assert!(i.detail.contains("angle=90deg"));
}

#[test]
fn flick_candidates_emit_up_dv_for_carried_touches_below_the_floor() {
    // A carried touch whose up-velocity (300) is *below* flick_min_up_dv (550) —
    // the gated detector drops it, candidate-mode keeps it. A non-carried touch
    // (ball released high) is not a flick attempt and is excluded.
    let frames = vec![
        frame(
            0.9,
            Some(kin(v(0.0, 0.0, 150.0), v(0.0, 0.0, 0.0))),
            vec![car(1, 0, v(0.0, 0.0, 17.0), v(0.0, 0.0, 0.0))],
        ),
        frame(
            1.0,
            Some(kin(v(0.0, 0.0, 160.0), v(0.0, 0.0, 300.0))), // carried, weak pop
            vec![car(1, 0, v(0.0, 0.0, 17.0), v(0.0, 0.0, 0.0))],
        ),
        frame(
            1.9,
            Some(kin(v(0.0, 0.0, 150.0), v(0.0, 0.0, 0.0))),
            vec![car(1, 0, v(0.0, 0.0, 17.0), v(0.0, 0.0, 0.0))],
        ),
        frame(
            2.0,
            Some(kin(v(0.0, 0.0, 500.0), v(0.0, 0.0, 300.0))), // ball high → not carried
            vec![car(1, 0, v(0.0, 0.0, 17.0), v(0.0, 0.0, 0.0))],
        ),
    ];
    let g = grid(frames);
    let events = [touch(1.0, 1, 0), touch(2.0, 1, 0)];
    assert!(flicks(&g, &events, &cfg()).is_empty(), "300 < floor 550");
    assert_eq!(flick_candidates(&g, &events, &cfg()), vec![300.0]);
}

#[test]
fn power_shot_candidates_emit_speed_for_goalward_touches_below_the_floor() {
    // A goalward touch at 1400uu/s — below power_shot_min_speed (2000) — is dropped
    // by the gate but kept as a candidate. A fast ball away from goal is excluded.
    let frames = vec![
        frame(
            3.0,
            Some(kin(v(0.0, 0.0, 100.0), v(0.0, 1400.0, 0.0))), // goalward, sub-floor
            vec![car(1, 0, v(0.0, -200.0, 17.0), v(0.0, 0.0, 0.0))],
        ),
        frame(
            4.0,
            Some(kin(v(0.0, 0.0, 100.0), v(0.0, -1400.0, 0.0))), // away from goal
            vec![car(1, 0, v(0.0, 200.0, 17.0), v(0.0, 0.0, 0.0))],
        ),
    ];
    let g = grid(frames);
    let events = [touch(3.0, 1, 0), touch(4.0, 1, 0)];
    assert!(
        power_shots(&g, &events, &cfg()).is_empty(),
        "1400 < floor 2000"
    );
    assert_eq!(power_shot_candidates(&g, &events, &cfg()), vec![1400.0]);
}

#[test]
fn redirect_candidates_emit_angle_for_glancing_goalward_touches() {
    // A 30° turn on a fast incoming, fast-out, goalward ball — below
    // redirect_min_angle_deg (55) so the gate drops it, candidate-mode keeps it.
    // A slow-incoming touch isn't a redirect attempt and is excluded.
    let frames = vec![
        frame(
            4.9,
            Some(kin(v(0.0, 0.0, 200.0), v(750.0, 1299.0, 0.0))), // 30° off +Y, fast
            vec![car(1, 0, v(0.0, 0.0, 180.0), v(0.0, 0.0, 0.0))],
        ),
        frame(
            5.0,
            Some(kin(v(0.0, 0.0, 200.0), v(0.0, 1500.0, 0.0))), // goalward, fast
            vec![car(1, 0, v(0.0, 0.0, 180.0), v(0.0, 0.0, 0.0))],
        ),
        frame(
            5.9,
            Some(kin(v(0.0, 0.0, 200.0), v(0.0, 300.0, 0.0))), // slow incoming
            vec![car(1, 0, v(0.0, 0.0, 180.0), v(0.0, 0.0, 0.0))],
        ),
        frame(
            6.0,
            Some(kin(v(0.0, 0.0, 200.0), v(0.0, 1500.0, 0.0))),
            vec![car(1, 0, v(0.0, 0.0, 180.0), v(0.0, 0.0, 0.0))],
        ),
    ];
    let g = grid(frames);
    let events = [touch(5.0, 1, 0), touch(6.0, 1, 0)];
    assert!(redirects(&g, &events, &cfg()).is_empty(), "30° < floor 55°");
    let cands = redirect_candidates(&g, &events, &cfg());
    assert_eq!(
        cands.len(),
        1,
        "only the fast-incoming touch is a candidate"
    );
    assert!((cands[0] - 30.0).abs() < 1.0, "≈30° turn, got {}", cands[0]);
}

// Kickoff event at t=0 with cars frozen (countdown) until they release ("GO") at
// t=0.5 — the reaction metric is measured from GO, not the frozen setup frame.
fn kickoff_grid() -> Resampled {
    let frozen = |t: f32| {
        frame(
            t,
            None,
            vec![
                car(1, 0, v(-256.0, 0.0, 17.0), v(0.0, 0.0, 0.0)),
                car(2, 1, v(256.0, 0.0, 17.0), v(0.0, 0.0, 0.0)),
            ],
        )
    };
    let moving = frame(
        0.5,
        None,
        vec![
            car(1, 0, v(-256.0, 0.0, 17.0), v(900.0, 0.0, 0.0)),
            car(2, 1, v(256.0, 0.0, 17.0), v(-900.0, 0.0, 0.0)),
        ],
    );
    grid(vec![frozen(0.0), frozen(0.25), moving])
}

#[test]
fn kickoff_first_touch_credited_to_first_toucher_after_release() {
    let events = vec![
        Event::Kickoff { t: 0.0 },
        touch(1.2, 2, 1), // first touch after release
        touch(1.5, 1, 0),
    ];
    let i = only(
        kickoff_first_touches(&kickoff_grid(), &events, &cfg()),
        Skill::KickoffFirstTouch,
    );
    assert_eq!(i.pri, 2);
    // Metric is from GO (0.5s), not the frozen kickoff frame (0.0s): 1.2 - 0.5.
    assert!(
        (i.metric - 0.7).abs() < 1e-5,
        "reaction from GO, got {}",
        i.metric
    );
}

#[test]
fn kickoff_first_touch_ignores_touch_past_window() {
    // GO is at 0.5s; a touch 30s later is well past GO + window.
    let events = vec![Event::Kickoff { t: 0.0 }, touch(30.0, 1, 0)];
    assert!(kickoff_first_touches(&kickoff_grid(), &events, &cfg()).is_empty());
}

#[test]
fn kickoff_first_touch_skipped_when_cars_never_release() {
    // Cars stay frozen → no GO within the window → no instance (degenerate).
    let g = grid(vec![
        frame(
            0.0,
            None,
            vec![car(1, 0, v(0.0, 0.0, 17.0), v(0.0, 0.0, 0.0))],
        ),
        frame(
            0.5,
            None,
            vec![car(1, 0, v(0.0, 0.0, 17.0), v(0.0, 0.0, 0.0))],
        ),
    ]);
    let events = vec![Event::Kickoff { t: 0.0 }, touch(1.2, 1, 0)];
    assert!(kickoff_first_touches(&g, &events, &cfg()).is_empty());
}

#[test]
fn boost_steal_detected_for_full_pad_in_opponent_half() {
    // Team 0 attacks +Y: a full-pad pickup at +Y (opponent corner) is a steal.
    let mut t = track(1, 0);
    t.samples = vec![
        TrackSample {
            t: 10.0,
            actor_id: 1,
            p: v(3000.0, 4000.0, 17.0),
            v: v(0.0, 0.0, 0.0),
            boost: Some(80),
            rot: None,
        },
        TrackSample {
            t: 10.1,
            actor_id: 1,
            p: v(3000.0, 4000.0, 17.0),
            v: v(0.0, 0.0, 0.0),
            boost: Some(255),
            rot: None,
        },
    ];
    let i = only(boost_steals(&[t], &grid(vec![]), &cfg()), Skill::BoostSteal);
    assert_eq!(i.pri, 1);
    assert!(i.detail.contains("to 255"));
}

#[test]
fn no_boost_steal_in_own_half() {
    let mut t = track(1, 0);
    t.samples = vec![
        TrackSample {
            t: 10.0,
            actor_id: 1,
            p: v(3000.0, -4000.0, 17.0),
            v: v(0.0, 0.0, 0.0),
            boost: Some(80),
            rot: None,
        },
        TrackSample {
            t: 10.1,
            actor_id: 1,
            p: v(3000.0, -4000.0, 17.0),
            v: v(0.0, 0.0, 0.0),
            boost: Some(255),
            rot: None,
        },
    ];
    assert!(boost_steals(&[t], &grid(vec![]), &cfg()).is_empty());
}

#[test]
fn boost_steal_ignores_refill_across_respawn_gap() {
    let mut t = track(1, 0);
    t.samples = vec![
        TrackSample {
            t: 10.0,
            actor_id: 1,
            p: v(3000.0, 4000.0, 17.0),
            v: v(0.0, 0.0, 0.0),
            boost: Some(0),
            rot: None,
        },
        TrackSample {
            t: 13.0,
            actor_id: 2,
            p: v(3000.0, 4000.0, 17.0),
            v: v(0.0, 0.0, 0.0),
            boost: Some(255),
            rot: None,
        },
    ];
    t.gaps = vec![TrackGap {
        start: 10.0,
        end: 13.0,
        reason: replay_analyzer::model::GapReason::Respawn,
    }];
    assert!(boost_steals(&[t], &grid(vec![]), &cfg()).is_empty());
}

#[test]
fn demo_credited_to_attacker() {
    let events = vec![Event::Demo {
        t: 7.0,
        attacker_pri: Some(1),
        attacker: Some("p1".to_string()),
        victim_pri: Some(2),
        victim: Some("p2".to_string()),
    }];
    let i = only(demos(&events, &[track(1, 0), track(2, 1)]), Skill::Demo);
    assert_eq!(i.pri, 1);
    assert_eq!(i.team, Some(0));
    assert!(i.detail.contains("demoed p2"));
}

/// A silence guard: a rotation-only field present on cars must not perturb
/// position-based detectors (rot is unused by these detectors but part of the
/// model).
#[test]
fn rotation_field_is_ignored_by_position_detectors() {
    let hot = |t: f32| {
        let mut c = car(1, 0, v(0.0, 0.0, 17.0), v(2300.0, 0.0, 0.0));
        c.rot = Some(Rot3 {
            pitch: 1.0,
            yaw: 2.0,
            roll: 3.0,
        });
        frame(t, None, vec![c])
    };
    let frames = vec![hot(0.0), hot(0.3), hot(0.6)];
    assert_eq!(supersonic(&grid(frames), &[track(1, 0)], &cfg()).len(), 1);
}
