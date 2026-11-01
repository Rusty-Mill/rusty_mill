//! Fidelity regression gate: the port against real Rocket League.
//!
//! Each case is a scripted tape-bot run (a scenario under
//! `tools/rb_tape_bot/{scenarios,experiments}`) whose recorded frames, trimmed
//! to the tape, are checked in under `tools/rb_tape_bot/fixtures/`
//! (`make_fixture.py`). The port is fed the input the game recorded
//! (`compare_scenario_recorded`, `RB-VERIFY-003-FR-016`) and its car and ball
//! positions must stay within the error bounds below: the measured error on
//! 2026-10-07 plus a quarter. A physics change that makes the port worse on any
//! of these real recordings fails here; one that makes it better should lower
//! the bound (`ADR-0066`). The captures are of a scripted bot, not a person,
//! so they hold no personal data.

use std::error::Error;
use std::path::PathBuf;

use rb_capture_ingest::CaptureFileSource;
use rb_domain::PhysicsStateSource;
use rb_scenario::Scenario;
use rb_verify_cli::{compare_scenario_recorded, ScenarioComparison};

/// One recording, its scenario and the most error the port may show on it
/// (mean, max; uu).
struct Case {
    scenario: &'static str,
    fixture: &'static str,
    car: (f32, f32),
    ball: Option<(f32, f32)>,
    /// The second car, for a bump scenario.
    other: Option<(f32, f32)>,
}

const CASES: &[Case] = &[
    // A level car with its wheels off the ground hits a stopped grounded one (FR-153): the
    // victim leaves at 1.25 times its speed and up. `other` is the victim.
    Case {
        scenario: "experiments/bumpag_700_45.json",
        fixture: "bumpag_700_45.jsonl",
        car: (1.2, 1.7),
        ball: None,
        other: Some((5.3, 11.3)),
    },
    Case {
        scenario: "experiments/bumpag_1000_45.json",
        fixture: "bumpag_1000_45.jsonl",
        car: (0.6, 1.2),
        ball: None,
        other: Some((11.4, 36.8)),
    },
    // A boosting attacker demolishes a stopped enemy; the victim is back 3.000 s later at
    // the spawn point the recording shows (FR-151). `other` is the victim.
    Case {
        scenario: "experiments/demo_4.json",
        fixture: "demo_4.jsonl",
        car: (3.8, 8.9),
        ball: None,
        other: Some((0.3, 0.3)),
    },
    Case {
        scenario: "experiments/demo_9.json",
        fixture: "demo_9.jsonl",
        car: (3.0, 5.8),
        ball: None,
        other: Some((0.3, 0.3)),
    },
    // A level car dropped on a ball at rest (RB-RESEARCH-O014, FR-150): the wheels
    // hold the car up on the ball. The bounds are today's errors, not parity: the
    // off-centre drops are far from the game (ball 92 uu mean at 100 uu off centre).
    // The game's 2.0 s sleep timer (FR-037) lets the centred drops drift where the old
    // 0.5 s one froze them early, so wob_0_200's ball and wob_0_500's car bounds rose.
    Case {
        scenario: "experiments/wob_0_200.json",
        fixture: "wob_0_200.jsonl",
        car: (1.3, 7.0),
        ball: Some((23.0, 87.7)),
        other: None,
    },
    Case {
        scenario: "experiments/wob_0_500.json",
        fixture: "wob_0_500.jsonl",
        car: (9.0, 41.7),
        ball: Some((26.5, 92.0)),
        other: None,
    },
    Case {
        scenario: "experiments/wob_100_200.json",
        fixture: "wob_100_200.jsonl",
        car: (15.0, 37.0),
        ball: Some((97.0, 212.0)),
        other: None,
    },
    Case {
        scenario: "scenarios/half_flip.json",
        fixture: "half_flip.jsonl",
        car: (3.0, 14.8),
        ball: None,
        other: None,
    },
    Case {
        scenario: "scenarios/hard_landing_nose_first.json",
        fixture: "hard_landing_nose_first.jsonl",
        car: (3.3, 6.8),
        ball: None,
        other: None,
    },
    Case {
        scenario: "experiments/jumpgap_H3_R3.json",
        fixture: "jumpgap_H3_R3.jsonl",
        car: (0.5, 1.5),
        ball: None,
        other: None,
    },
    Case {
        scenario: "experiments/jumpgap_H3_R4.json",
        fixture: "jumpgap_H3_R4.jsonl",
        car: (3.4, 21.3),
        ball: None,
        other: None,
    },
    Case {
        scenario: "scenarios/late_dodge.json",
        fixture: "late_dodge.jsonl",
        car: (1.8, 3.7),
        ball: Some((0.2, 3.1)),
        other: None,
    },
    Case {
        scenario: "scenarios/pogo.json",
        fixture: "pogo.jsonl",
        car: (2.8, 10.5),
        ball: None,
        other: None,
    },
    Case {
        scenario: "experiments/probe_accel_boost.json",
        fixture: "probe_accel_boost.jsonl",
        car: (0.5, 1.5),
        ball: None,
        other: None,
    },
    Case {
        scenario: "experiments/probe_air_roll.json",
        fixture: "probe_air_roll.jsonl",
        car: (1.7, 4.4),
        ball: None,
        other: None,
    },
    Case {
        scenario: "experiments/probe_ball_ceiling.json",
        fixture: "probe_ball_ceiling.jsonl",
        car: (0.5, 1.5),
        ball: Some((3.5, 8.0)),
        other: None,
    },
    Case {
        scenario: "experiments/probe_ball_goal.json",
        fixture: "probe_ball_goal.jsonl",
        car: (0.5, 1.5),
        ball: Some((5.3, 14.2)),
        other: None,
    },
    Case {
        scenario: "experiments/probe_ball_roll.json",
        fixture: "probe_ball_roll.jsonl",
        car: (0.5, 1.5),
        ball: Some((1.5, 4.3)),
        other: None,
    },
    Case {
        scenario: "experiments/probe_ball_wall.json",
        fixture: "probe_ball_wall.jsonl",
        car: (0.5, 1.5),
        ball: Some((1.9, 5.9)),
        other: None,
    },
    // A ball rolling without sliding below the sleep thresholds is frozen 2.0 s in
    // (FR-037, board b19); 80 uu/s rolls on. rest_40 and rest_80 keep a rolling-decay
    // error (the port's rolling ball slows 2.1%/s, the game's 3.0%/s). A ball set at
    // 6500 uu/s is capped before its first tick moves it. fast_6500's max is not
    // the cap or the wall: the port slides 0.2% further along the floor (1.8958
    // against 1.8999 uu/s a tick of friction), reaches the ramp 0.3 uu ahead and
    // leaves its top facet a tick early (board #197; see
    // `from_the_games_state_the_port_rides_the_side_ramp_and_bounces_off_the_wall`).
    Case {
        scenario: "experiments/probe_ball_rest_10.json",
        fixture: "probe_ball_rest_10.jsonl",
        car: (0.5, 1.5),
        ball: Some((1.0, 1.4)),
        other: None,
    },
    Case {
        scenario: "experiments/probe_ball_rest_20.json",
        fixture: "probe_ball_rest_20.jsonl",
        car: (0.5, 1.5),
        ball: Some((1.3, 1.8)),
        other: None,
    },
    // rest_40's ball error fell when the ball's spin moved to its 91.25 uu
    // sphere (FR-161): 6.49 / 11.07 -> 0.29 / 0.36 uu. Ball bounds on roll, wall,
    // rest_80, ballfuzz_203 and nose_hit_glancing were lowered with it.
    Case {
        scenario: "experiments/probe_ball_rest_40.json",
        fixture: "probe_ball_rest_40.jsonl",
        car: (0.5, 1.5),
        ball: Some((0.5, 1.5)),
        other: None,
    },
    Case {
        scenario: "experiments/probe_ball_rest_80.json",
        fixture: "probe_ball_rest_80.jsonl",
        car: (0.5, 1.5),
        ball: Some((2.7, 8.0)),
        other: None,
    },
    Case {
        scenario: "experiments/probe_ball_fast_4500.json",
        fixture: "probe_ball_fast_4500.jsonl",
        car: (0.5, 1.5),
        ball: Some((0.5, 1.5)),
        other: None,
    },
    Case {
        scenario: "experiments/probe_ball_fast_6500.json",
        fixture: "probe_ball_fast_6500.jsonl",
        car: (0.5, 1.5),
        ball: Some((7.1, 71.1)),
        other: None,
    },
    Case {
        scenario: "experiments/probe_boost_air_pitchup.json",
        fixture: "probe_boost_air_pitchup.jsonl",
        car: (3.0, 6.9),
        ball: None,
        other: None,
    },
    Case {
        scenario: "experiments/probe_brake.json",
        fixture: "probe_brake.jsonl",
        car: (0.5, 1.5),
        ball: None,
        other: None,
    },
    Case {
        scenario: "experiments/probe_dodge_back.json",
        fixture: "probe_dodge_back.jsonl",
        car: (0.7, 1.5),
        ball: None,
        other: None,
    },
    Case {
        scenario: "experiments/probe_dodge_diag.json",
        fixture: "probe_dodge_diag.jsonl",
        car: (0.5, 1.5),
        ball: None,
        other: None,
    },
    Case {
        scenario: "experiments/probe_hit_boost.json",
        fixture: "probe_hit_boost.jsonl",
        car: (0.5, 1.8),
        ball: Some((1.0, 3.8)),
        other: None,
    },
    // The car's max rose from 0.62 to 1.50 uu when the ball's inertia moved to its
    // 91.25 uu sphere (FR-161), at the bound; raised by a quarter. Its ball error did
    // not move (3.09 -> 3.02 uu max).
    Case {
        scenario: "experiments/probe_hit_offset.json",
        fixture: "probe_hit_offset.jsonl",
        car: (0.5, 1.9),
        ball: Some((1.0, 3.9)),
        other: None,
    },
    Case {
        scenario: "experiments/probe_ps_straight.json",
        fixture: "probe_ps_straight.jsonl",
        car: (0.5, 1.5),
        ball: None,
        other: None,
    },
    Case {
        scenario: "experiments/hb_rev_1400.json",
        fixture: "hb_rev_1400.jsonl",
        car: (0.3, 0.6),
        ball: None,
        other: None,
    },
    Case {
        scenario: "experiments/hb_rev_600.json",
        fixture: "hb_rev_600.jsonl",
        car: (0.3, 0.4),
        ball: None,
        other: None,
    },
    Case {
        scenario: "experiments/hb_rev_steer.json",
        fixture: "hb_rev_steer.jsonl",
        car: (4.5, 12.4),
        ball: None,
        other: None,
    },
    Case {
        scenario: "experiments/hb_boost_rev.json",
        fixture: "hb_boost_rev.jsonl",
        car: (0.2, 0.3),
        ball: None,
        other: None,
    },
    Case {
        scenario: "experiments/probe_powerslide.json",
        fixture: "probe_powerslide.jsonl",
        car: (6.0, 14.2),
        ball: None,
        other: None,
    },
    Case {
        scenario: "experiments/probe_ps_boost.json",
        fixture: "probe_ps_boost.jsonl",
        car: (6.2, 24.9),
        ball: None,
        other: None,
    },
    Case {
        scenario: "experiments/probe_ps_slow.json",
        fixture: "probe_ps_slow.jsonl",
        car: (9.2, 29.4),
        ball: None,
        other: None,
    },
    Case {
        scenario: "experiments/ja_thr.json",
        fixture: "ja_thr.jsonl",
        car: (0.7, 1.8),
        ball: None,
        other: None,
    },
    Case {
        scenario: "experiments/probe_wall_land.json",
        fixture: "probe_wall_land.jsonl",
        car: (5.6, 11.0),
        ball: None,
        other: None,
    },
    Case {
        scenario: "experiments/dodgeth_yaw.json",
        fixture: "dodgeth_yaw.jsonl",
        car: (0.5, 1.3),
        ball: None,
        other: None,
    },
    Case {
        scenario: "experiments/dodgeth_pitch.json",
        fixture: "dodgeth_pitch.jsonl",
        car: (1.5, 7.7),
        ball: None,
        other: None,
    },
    Case {
        scenario: "experiments/bt_rev.json",
        fixture: "bt_rev.jsonl",
        car: (0.5, 1.4),
        ball: None,
        other: None,
    },
    Case {
        scenario: "experiments/bt_rev_ground.json",
        fixture: "bt_rev_ground.jsonl",
        car: (0.2, 0.3),
        ball: None,
        other: None,
    },
    Case {
        scenario: "experiments/ballfuzz_203.json",
        fixture: "ballfuzz_203.jsonl",
        car: (0.2, 0.2),
        ball: Some((9.7, 28.8)),
        other: None,
    },
    Case {
        scenario: "experiments/ballfuzz_218.json",
        fixture: "ballfuzz_218.jsonl",
        car: (0.2, 0.2),
        ball: Some((6.2, 13.7)),
        other: None,
    },
    Case {
        scenario: "experiments/probe_reverse.json",
        fixture: "probe_reverse.jsonl",
        car: (0.5, 1.5),
        ball: None,
        other: None,
    },
    Case {
        scenario: "experiments/probe_turn_fast.json",
        fixture: "probe_turn_fast.jsonl",
        car: (3.8, 13.7),
        ball: None,
        other: None,
    },
    Case {
        scenario: "experiments/probe_wall_ride_45.json",
        fixture: "probe_wall_ride_45.jsonl",
        car: (0.7, 1.9),
        ball: None,
        other: None,
    },
    Case {
        scenario: "scenarios/speed_flip.json",
        fixture: "speed_flip.jsonl",
        car: (1.0, 9.4),
        ball: None,
        other: None,
    },
    Case {
        scenario: "scenarios/wavedash_mid.json",
        fixture: "wavedash_mid.jsonl",
        car: (0.5, 1.5),
        ball: None,
        other: None,
    },
    Case {
        scenario: "experiments/probe_land_wheels.json",
        fixture: "probe_land_wheels.jsonl",
        car: (0.5, 1.5),
        ball: None,
        other: None,
    },
    Case {
        scenario: "experiments/probe_flip_cancel.json",
        fixture: "probe_flip_cancel.jsonl",
        car: (0.5, 1.5),
        ball: None,
        other: None,
    },
    Case {
        scenario: "experiments/probe_dodge_fast_side.json",
        fixture: "probe_dodge_fast_side.jsonl",
        car: (0.5, 1.5),
        ball: None,
        other: None,
    },
    Case {
        scenario: "experiments/probe_double_jump_moving.json",
        fixture: "probe_double_jump_moving.jsonl",
        car: (0.7, 1.5),
        ball: None,
        other: None,
    },
    Case {
        scenario: "experiments/probe_ball_bounce_car_side.json",
        fixture: "probe_ball_bounce_car_side.jsonl",
        car: (0.5, 1.5),
        ball: Some((1.0, 3.0)),
        other: None,
    },
    Case {
        scenario: "experiments/probe_nose_hit_glancing.json",
        fixture: "probe_nose_hit_glancing.jsonl",
        car: (0.8, 2.4),
        ball: Some((1.3, 3.2)),
        other: None,
    },
    Case {
        scenario: "experiments/bumpr_300.json",
        fixture: "bumpr_300.jsonl",
        car: (0.8, 1.5),
        ball: None,
        other: Some((0.5, 1.5)),
    },
    Case {
        scenario: "experiments/bumpr_900.json",
        fixture: "bumpr_900.jsonl",
        car: (2.0, 3.0),
        ball: None,
        other: Some((3.5, 6.2)),
    },
    Case {
        scenario: "experiments/bumpr_1500.json",
        fixture: "bumpr_1500.jsonl",
        car: (1.9, 3.0),
        ball: None,
        other: Some((5.5, 13.7)),
    },
    Case {
        scenario: "experiments/bumpr_2100.json",
        fixture: "bumpr_2100.jsonl",
        car: (0.5, 1.5),
        ball: None,
        other: Some((12.7, 35.3)),
    },
    Case {
        scenario: "experiments/bumps_600.json",
        fixture: "bumps_600.jsonl",
        car: (1.5, 4.8),
        ball: None,
        other: Some((3.4, 17.5)),
    },
    Case {
        scenario: "experiments/bumps_1800.json",
        fixture: "bumps_1800.jsonl",
        car: (1.8, 2.9),
        ball: None,
        other: Some((9.0, 26.5)),
    },
    Case {
        scenario: "experiments/bumph_500.json",
        fixture: "bumph_500.jsonl",
        car: (2.2, 10.2),
        ball: None,
        other: Some((3.3, 10.9)),
    },
    Case {
        scenario: "experiments/bumpo_900.json",
        fixture: "bumpo_900.jsonl",
        car: (2.9, 20.5),
        ball: None,
        other: Some((13.0, 46.2)),
    },
    Case {
        scenario: "experiments/bumpm_1200.json",
        fixture: "bumpm_1200.jsonl",
        car: (0.3, 0.5),
        ball: None,
        other: Some((1.7, 6.7)),
    },
    Case {
        scenario: "experiments/bumpd_1800.json",
        fixture: "bumpd_1800.jsonl",
        car: (0.9, 1.5),
        ball: None,
        other: Some((8.4, 20.0)),
    },
    Case {
        scenario: "experiments/bumpd_2300.json",
        fixture: "bumpd_2300.jsonl",
        car: (1.4, 1.9),
        ball: None,
        other: Some((0.5, 1.5)),
    },
    Case {
        scenario: "experiments/bumpd_side_2300.json",
        fixture: "bumpd_side_2300.jsonl",
        car: (1.2, 3.0),
        ball: None,
        other: Some((0.5, 1.5)),
    },
    Case {
        scenario: "experiments/bumpd_mate_2300.json",
        fixture: "bumpd_mate_2300.jsonl",
        car: (1.2, 1.8),
        ball: None,
        other: Some((16.9, 40.8)),
    },
    Case {
        scenario: "experiments/bumpv_moving_side.json",
        fixture: "bumpv_moving_side.jsonl",
        car: (5.2, 19.0),
        ball: None,
        other: Some((3.8, 11.9)),
    },
    Case {
        scenario: "experiments/bumpc_corner.json",
        fixture: "bumpc_corner.jsonl",
        car: (14.0, 42.5),
        ball: None,
        other: Some((7.9, 21.3)),
    },
    Case {
        scenario: "experiments/bumpa_air_1000.json",
        fixture: "bumpa_air_1000.jsonl",
        car: (2.2, 5.9),
        ball: None,
        other: Some((1.5, 2.4)),
    },
    Case {
        scenario: "experiments/bumpa_air_1400.json",
        fixture: "bumpa_air_1400.jsonl",
        car: (3.0, 8.5),
        ball: None,
        other: Some((2.8, 5.7)),
    },
    // The first 130 ticks of hivemind duels (`gen_duel_fuzz.py`, both cars play a tape).
    // 865: two cars at 1000 uu/s each, 70 uu off line, both bumped (FR-158); 910: an
    // airborne car bumps a grounded one and pushes it up by its climb (FR-159); 904: a
    // clean head-on; 911: a tank that grew on the first frame (a pad swept by the state
    // set, counted into the starting tank).
    Case {
        scenario: "experiments/duelg_865.json",
        fixture: "duelg_865.jsonl",
        car: (6.4, 20.0),
        ball: None,
        other: Some((6.9, 21.0)),
    },
    Case {
        scenario: "experiments/duelg_910.json",
        fixture: "duelg_910.jsonl",
        car: (2.8, 9.0),
        ball: None,
        other: Some((3.0, 10.9)),
    },
    Case {
        scenario: "experiments/duelg_904.json",
        fixture: "duelg_904.jsonl",
        car: (0.2, 0.4),
        ball: None,
        other: Some((0.3, 0.9)),
    },
    Case {
        scenario: "experiments/duelg_911.json",
        fixture: "duelg_911.jsonl",
        car: (0.2, 0.5),
        ball: None,
        other: Some((0.2, 0.2)),
    },
];

/// The game spins a sliding ball up at 0.05190 rad/s a tick
/// (`probe_ball_fast_6500`, from 0.05191 at tick 1 to 5.19001 at tick 100):
/// friction about RocketSim's 91.25 uu ball sphere. From the game's tick-1
/// state the port's spin stays within 0.02 rad/s of it for 100 ticks; about
/// the 93.15 uu contact sphere it fell 0.10 rad/s behind.
#[test]
fn a_sliding_ball_spins_up_as_the_games_does() -> Result<(), Box<dyn Error>> {
    const FROM: usize = 1;
    const TICKS: usize = 100;
    let frames =
        CaptureFileSource::new(bot_dir().join("fixtures/probe_ball_fast_6500.jsonl")).frames()?;
    let window = frames
        .get(FROM..=FROM + TICKS)
        .ok_or("the capture is shorter than the window")?;
    let mut env = rb_env::Env::new();
    env.reset(&window[0]);
    let mut worst = 0.0f32;
    for pair in window.windows(2) {
        let (before, recorded) = (&pair[0], &pair[1]);
        let gap = recorded.timestamp_secs - before.timestamp_secs;
        assert!((gap - rb_env::TICK_SECS).abs() < 1e-4, "gap {gap} s");
        let inputs: Vec<_> = recorded
            .cars
            .iter()
            .map(|car| car.input.unwrap_or_default())
            .collect();
        let predicted = env.step(&inputs);
        let spin = (predicted.ball.angular_velocity - recorded.ball.angular_velocity).length();
        worst = worst.max(spin);
    }
    assert!(worst < 0.02, "worst ball spin error {worst:.4} rad/s");
    Ok(())
}

fn bot_dir() -> PathBuf {
    PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("../../rusty_bullet/tools/rb_tape_bot")
}

fn compare(
    case: &Case,
    perturb: impl Fn(usize, &mut rb_domain::PhysicsFrame),
) -> Result<ScenarioComparison, Box<dyn Error>> {
    let bot = bot_dir();
    let scenario = Scenario::from_json(&std::fs::read_to_string(bot.join(case.scenario))?)?;
    let mut frames = CaptureFileSource::new(bot.join("fixtures").join(case.fixture)).frames()?;
    for (i, frame) in frames.iter_mut().enumerate() {
        perturb(i, frame);
    }
    Ok(compare_scenario_recorded(&scenario, &frames)?)
}

#[test]
fn the_port_stays_within_its_error_bounds_on_every_golden_capture() -> Result<(), Box<dyn Error>> {
    // One thread per case: each is an independent simulation, and the debug
    // build is slow.
    let results: Vec<Result<Vec<String>, String>> = std::thread::scope(|scope| {
        let handles: Vec<_> = CASES
            .iter()
            .map(|case| {
                scope.spawn(move || check(case).map_err(|e| format!("{}: {e}", case.fixture)))
            })
            .collect();
        handles
            .into_iter()
            .map(|handle| {
                handle
                    .join()
                    .unwrap_or_else(|_| Err("a case panicked".to_string()))
            })
            .collect()
    });
    let mut failures = Vec::new();
    for result in results {
        match result {
            Ok(mut found) => failures.append(&mut found),
            Err(e) => failures.push(e),
        }
    }
    assert!(
        failures.is_empty(),
        "
{}",
        failures.join(
            "
"
        )
    );
    Ok(())
}

/// The bound violations of one case (empty when it passes).
fn check(case: &Case) -> Result<Vec<String>, Box<dyn Error>> {
    let mut failures = Vec::new();
    let comparison = compare(case, |_, _| {})?;
    let (mean, max) = (
        comparison.mean_position_error(),
        comparison.max_position_error(),
    );
    if mean > case.car.0 || max > case.car.1 {
        failures.push(format!(
            "{}: car mean {mean:.2} / max {max:.2} uu over the bound {:?}",
            case.fixture, case.car
        ));
    }
    if let Some(bound) = case.ball {
        let (mean, max) = (comparison.mean_ball_error(), comparison.max_ball_error());
        if mean > bound.0 || max > bound.1 {
            failures.push(format!(
                "{}: ball mean {mean:.2} / max {max:.2} uu over the bound {bound:?}",
                case.fixture
            ));
        }
    }
    if let Some(bound) = case.other {
        let (mean, max) = (
            comparison.mean_other_error(0),
            comparison.max_other_error(0),
        );
        if mean > bound.0 || max > bound.1 {
            failures.push(format!(
                "{}: second car mean {mean:.2} / max {max:.2} uu over the bound {bound:?}",
                case.fixture
            ));
        }
    }
    Ok(failures)
}

/// A recording the port cannot match (the car shifted 50 uu) must fail the
/// bound, or the gate above would pass anything.
#[test]
fn a_recording_the_port_cannot_match_exceeds_the_bound() -> Result<(), Box<dyn Error>> {
    let case = &CASES[0];
    // From the second frame on, so the start frame still aligns.
    let shifted = compare(case, |i, frame| {
        if i > 0 {
            for car in &mut frame.cars {
                car.position.x += 50.0;
            }
        }
    })?;
    assert!(
        shifted.mean_position_error() > case.car.0,
        "{}",
        shifted.mean_position_error()
    );
    Ok(())
}

/// Every checked-in fixture has a case, so a new one cannot go unchecked.
#[test]
fn every_fixture_has_a_case() -> Result<(), Box<dyn Error>> {
    let mut on_disk: Vec<String> = std::fs::read_dir(bot_dir().join("fixtures"))?
        .filter_map(|entry| entry.ok())
        .map(|entry| entry.file_name().to_string_lossy().into_owned())
        .filter(|name| name.ends_with(".jsonl"))
        .collect();
    let mut listed: Vec<String> = CASES.iter().map(|case| case.fixture.to_string()).collect();
    on_disk.sort();
    listed.sort();
    assert_eq!(on_disk, listed);
    Ok(())
}

/// From the game's own state just before the side ramp (`probe_ball_fast_6500`
/// tick 141), the port rides the ramp and bounces off the side wall the way
/// the game does: the ball leaves the ramp at about (1500, 3200) uu/s and the
/// wall returns it at (-210, 2855). So the scenario's remaining ball error (board
/// #197) is not the ramp, the wall or the seam between them. It is a 0.3 uu lead
/// built up while the ball slides along the floor (the port's sliding friction
/// takes 1.8958 uu/s a tick, the game's 1.8999), which decides whether the ball
/// still touches the ramp's top facet at tick 148. A guard: it passed on its
/// base. Combining the ball's restitution with the arena's by the smaller value
/// instead of the larger turns it red at the wall bounce.
#[test]
fn from_the_games_state_the_port_rides_the_side_ramp_and_bounces_off_the_wall(
) -> Result<(), Box<dyn Error>> {
    const FROM: usize = 141;
    const TICKS: usize = 20;
    let frames =
        CaptureFileSource::new(bot_dir().join("fixtures/probe_ball_fast_6500.jsonl")).frames()?;
    let window = frames
        .get(FROM..=FROM + TICKS)
        .ok_or("the capture is shorter than the window")?;
    let mut env = rb_env::Env::new();
    env.reset(&window[0]);
    let mut worst = (0.0f32, 0.0f32);
    for pair in window.windows(2) {
        let (before, recorded) = (&pair[0], &pair[1]);
        // Compared by time, one tick apart: no hole in this window.
        let gap = recorded.timestamp_secs - before.timestamp_secs;
        assert!((gap - rb_env::TICK_SECS).abs() < 1e-4, "gap {gap} s");
        let inputs: Vec<_> = recorded
            .cars
            .iter()
            .map(|car| car.input.unwrap_or_default())
            .collect();
        let predicted = env.step(&inputs);
        let position = (predicted.ball.position - recorded.ball.position).length();
        let velocity = (predicted.ball.velocity - recorded.ball.velocity).length();
        worst = (worst.0.max(position), worst.1.max(velocity));
    }
    assert!(
        worst.0 < 0.5 && worst.1 < 5.0,
        "worst ball error {:.2} uu, {:.2} uu/s",
        worst.0,
        worst.1
    );
    Ok(())
}
