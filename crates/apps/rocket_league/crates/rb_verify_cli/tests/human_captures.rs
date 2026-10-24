//! Fidelity regression gate on human-driven recordings.
//!
//! Each fixture under `rb_capture_ingest/fixtures/human/` is an excerpt of
//! the owner's own BakkesMod capture sessions (September 2026, keyboard and
//! mouse), with the input the game recorded on every tick. The port is seeded
//! from the first grounded, neutral frame and fed that input
//! (`score_capture_against_candidate`); its car and ball positions must stay
//! within the bounds below: the error measured on 2026-10-24 plus a quarter,
//! as in `golden_captures.rs`. Unlike the tape-bot captures, nothing here was
//! scripted, so these cover driving a script would not try. The files hold
//! poses and inputs only: no names or account ids.

use std::error::Error;
use std::path::PathBuf;

use rb_verify_cli::{score_capture_against_candidate, SeedFrame, DEFAULT_MAX_TIMESTAMP_DELTA_SECS};

/// One recording and the most error the port may show on it (mean, max; uu).
struct Case {
    fixture: &'static str,
    car: (f32, f32),
    /// `None` when the ball is never touched.
    ball: Option<(f32, f32)>,
}

const CASES: &[Case] = &[
    Case {
        fixture: "airborne-hit.capture.jsonl",
        car: (2.0, 10.8),
        ball: Some((1.9, 10.7)),
    },
    Case {
        fixture: "boost-wall-entry.capture.jsonl",
        car: (1.2, 11.9),
        ball: None,
    },
    Case {
        fixture: "clean-dodge.capture.jsonl",
        car: (2.2, 6.0),
        ball: None,
    },
    Case {
        fixture: "dodge-derailment.capture.jsonl",
        car: (8.7, 30.2),
        ball: Some((23.2, 104.9)),
    },
    Case {
        fixture: "dodge-free-landing.capture.jsonl",
        car: (1.4, 4.0),
        ball: None,
    },
    Case {
        fixture: "goal-shot-net-entry.capture.jsonl",
        car: (1.9, 2.5),
        ball: Some((7.6, 26.2)),
    },
    // The ball bound is today's error, not parity: after the wheels-down hit-tick jump the
    // ball's flight leaves the recording by 1 000 uu at worst (open).
    Case {
        fixture: "hit-tick-jump.capture.jsonl",
        car: (2.0, 9.4),
        ball: Some((235.8, 1279.8)),
    },
    Case {
        fixture: "throttle-jump.capture.jsonl",
        car: (2.5, 7.0),
        ball: None,
    },
    Case {
        fixture: "wall-climb-crest.capture.jsonl",
        car: (21.2, 113.8),
        ball: Some((15.2, 35.1)),
    },
];

fn fixture_dir() -> PathBuf {
    PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("../rb_capture_ingest/fixtures/human")
}

/// The bound violations of one case (empty when it passes).
fn check(case: &Case) -> Result<Vec<String>, Box<dyn Error>> {
    let score = score_capture_against_candidate(
        fixture_dir().join(case.fixture),
        SeedFrame::default(),
        DEFAULT_MAX_TIMESTAMP_DELTA_SECS,
    )?;
    let (mean, max) = (
        score.cars.mean_position_distance,
        score.cars.max_position_distance,
    );
    let mut failures = Vec::new();
    if mean > case.car.0 || max > case.car.1 {
        failures.push(format!(
            "{}: car mean {mean:.2} / max {max:.2} uu over the bound {:?}",
            case.fixture, case.car
        ));
    }
    if let Some(bound) = case.ball {
        let (mean, max) = (score.mean_ball_distance, score.max_ball_distance);
        if mean > bound.0 || max > bound.1 {
            failures.push(format!(
                "{}: ball mean {mean:.2} / max {max:.2} uu over the bound {bound:?}",
                case.fixture
            ));
        }
    }
    Ok(failures)
}

#[test]
fn the_port_stays_within_its_bound_on_every_human_capture() -> Result<(), Box<dyn Error>> {
    let mut failures = Vec::new();
    for case in CASES {
        failures.extend(check(case)?);
    }
    assert!(failures.is_empty(), "{}", failures.join("\n"));
    Ok(())
}

/// Every checked-in fixture has a case, so a new one cannot go unchecked.
#[test]
fn every_human_fixture_has_a_case() -> Result<(), Box<dyn Error>> {
    let mut on_disk: Vec<String> = std::fs::read_dir(fixture_dir())?
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
