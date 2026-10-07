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
}

const CASES: &[Case] = &[
    Case {
        scenario: "scenarios/half_flip.json",
        fixture: "half_flip.jsonl",
        car: (3.0, 14.8),
        ball: None,
    },
    Case {
        scenario: "scenarios/hard_landing_nose_first.json",
        fixture: "hard_landing_nose_first.jsonl",
        car: (3.3, 6.8),
        ball: None,
    },
    Case {
        scenario: "experiments/jumpgap_H3_R3.json",
        fixture: "jumpgap_H3_R3.jsonl",
        car: (0.5, 1.5),
        ball: None,
    },
    Case {
        scenario: "experiments/jumpgap_H3_R4.json",
        fixture: "jumpgap_H3_R4.jsonl",
        car: (3.4, 21.3),
        ball: None,
    },
    Case {
        scenario: "scenarios/late_dodge.json",
        fixture: "late_dodge.jsonl",
        car: (1.8, 3.7),
        ball: None,
    },
    Case {
        scenario: "scenarios/pogo.json",
        fixture: "pogo.jsonl",
        car: (3.0, 5.4),
        ball: None,
    },
    Case {
        scenario: "experiments/probe_accel_boost.json",
        fixture: "probe_accel_boost.jsonl",
        car: (0.5, 1.5),
        ball: None,
    },
    Case {
        scenario: "experiments/probe_air_roll.json",
        fixture: "probe_air_roll.jsonl",
        car: (1.7, 4.4),
        ball: None,
    },
    Case {
        scenario: "experiments/probe_ball_ceiling.json",
        fixture: "probe_ball_ceiling.jsonl",
        car: (0.5, 1.5),
        ball: Some((3.5, 8.0)),
    },
    Case {
        scenario: "experiments/probe_ball_goal.json",
        fixture: "probe_ball_goal.jsonl",
        car: (0.5, 1.5),
        ball: Some((5.3, 14.2)),
    },
    Case {
        scenario: "experiments/probe_ball_roll.json",
        fixture: "probe_ball_roll.jsonl",
        car: (0.5, 1.5),
        ball: Some((1.8, 8.5)),
    },
    Case {
        scenario: "experiments/probe_ball_wall.json",
        fixture: "probe_ball_wall.jsonl",
        car: (0.5, 1.5),
        ball: Some((2.8, 10.4)),
    },
    Case {
        scenario: "experiments/probe_boost_air_pitchup.json",
        fixture: "probe_boost_air_pitchup.jsonl",
        car: (3.0, 6.9),
        ball: None,
    },
    Case {
        scenario: "experiments/probe_brake.json",
        fixture: "probe_brake.jsonl",
        car: (0.5, 1.5),
        ball: None,
    },
    Case {
        scenario: "experiments/probe_dodge_back.json",
        fixture: "probe_dodge_back.jsonl",
        car: (0.7, 1.5),
        ball: None,
    },
    Case {
        scenario: "experiments/probe_dodge_diag.json",
        fixture: "probe_dodge_diag.jsonl",
        car: (0.5, 1.5),
        ball: None,
    },
    Case {
        scenario: "experiments/probe_hit_boost.json",
        fixture: "probe_hit_boost.jsonl",
        car: (0.5, 1.8),
        ball: Some((1.0, 3.8)),
    },
    Case {
        scenario: "experiments/probe_hit_offset.json",
        fixture: "probe_hit_offset.jsonl",
        car: (0.5, 1.5),
        ball: Some((1.0, 3.9)),
    },
    Case {
        scenario: "experiments/probe_ps_straight.json",
        fixture: "probe_ps_straight.jsonl",
        car: (0.5, 1.5),
        ball: None,
    },
    Case {
        scenario: "experiments/probe_reverse.json",
        fixture: "probe_reverse.jsonl",
        car: (0.5, 1.5),
        ball: None,
    },
    Case {
        scenario: "experiments/probe_turn_fast.json",
        fixture: "probe_turn_fast.jsonl",
        car: (3.8, 13.7),
        ball: None,
    },
    Case {
        scenario: "experiments/probe_wall_ride_45.json",
        fixture: "probe_wall_ride_45.jsonl",
        car: (0.7, 1.9),
        ball: None,
    },
    Case {
        scenario: "scenarios/speed_flip.json",
        fixture: "speed_flip.jsonl",
        car: (1.0, 9.4),
        ball: None,
    },
    Case {
        scenario: "scenarios/wavedash_mid.json",
        fixture: "wavedash_mid.jsonl",
        car: (0.5, 1.5),
        ball: None,
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
    let mut failures = Vec::new();
    for case in CASES {
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
