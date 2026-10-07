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
        ball: None,
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
        ball: Some((1.8, 8.5)),
        other: None,
    },
    Case {
        scenario: "experiments/probe_ball_wall.json",
        fixture: "probe_ball_wall.jsonl",
        car: (0.5, 1.5),
        ball: Some((2.8, 10.4)),
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
    Case {
        scenario: "experiments/probe_hit_offset.json",
        fixture: "probe_hit_offset.jsonl",
        car: (0.5, 1.5),
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
        ball: Some((1.8, 5.7)),
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
        car: (6.8, 22.8),
        ball: None,
        other: Some((6.8, 20.5)),
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
        car: (1.8, 4.5),
        ball: None,
        other: Some((3.4, 7.3)),
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
];

fn bot_dir() -> PathBuf {
    PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("../../tools/rb_tape_bot")
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
