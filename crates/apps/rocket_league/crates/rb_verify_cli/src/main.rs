//! `rb-verify`: verification pipeline entry point.
//!
//! Thin CLI over `rb_verify_cli`'s scoring functions — argument parsing and
//! human-readable output only, no logic of its own (see `lib.rs` for why
//! the actual wiring lives there instead). Three modes:
//! - `rb-verify <replay-file> <capture-file> [max-timestamp-delta-secs]`:
//!   the mechanical `PHASE-0-EXIT` pipeline check
//!   (`score_replay_against_capture`) — two unrelated recordings, no
//!   physical reason to resemble each other.
//! - `rb-verify --self <capture-file> [max-timestamp-delta-secs]`: the real
//!   fidelity check (`score_capture_against_candidate`,
//!   `RB-PHYSICS-001-FR-077`) — scores a capture's own recorded outcome
//!   against a candidate `rb_physics_bullet` actually simulated from that
//!   same capture's recorded input.
//! - `rb-verify --self-growth <capture-file> [window-secs]
//!   [max-timestamp-delta-secs]`: the divergence-growth diagnostic
//!   (`score_capture_growth`, `RB-VERIFY-003-FR-004`) — the same
//!   candidate-vs-capture comparison as `--self`, but reported per
//!   `window-secs`-wide time window instead of one whole-run number, so
//!   whether the divergence grows gradually or abruptly can be read
//!   directly from the output.

use rb_domain::divergence::DivergenceScore;
use rb_verify_cli::{
    score_capture_against_candidate, score_capture_growth, score_replay_against_capture,
    DEFAULT_GROWTH_WINDOW_SECS, DEFAULT_MAX_TIMESTAMP_DELTA_SECS,
};
use std::env;
use std::process::ExitCode;

fn print_score(score: &DivergenceScore) {
    println!("frames compared:    {}", score.frames_compared);
    println!("mean ball distance: {:.2} uu", score.mean_ball_distance);
    println!("max ball distance:  {:.2} uu", score.max_ball_distance);
    println!("car pairs compared: {}", score.cars.pairs_compared);
    println!(
        "mean car position/rotation/velocity distance: {:.2} uu / {:.2} rad / {:.2} uu/s",
        score.cars.mean_position_distance,
        score.cars.mean_rotation_distance,
        score.cars.mean_velocity_distance
    );
    println!(
        "max  car position/rotation/velocity distance: {:.2} uu / {:.2} rad / {:.2} uu/s",
        score.cars.max_position_distance,
        score.cars.max_rotation_distance,
        score.cars.max_velocity_distance
    );
}

fn print_growth(windows: &[(f32, DivergenceScore)]) {
    for (start, score) in windows {
        println!(
            "t={start:>7.2}s  frames={frames:>4}  ball mean/max={mean_ball:>8.2}/{max_ball:>8.2} uu  car mean pos/rot/vel={mean_pos:>8.2} uu / {mean_rot:.2} rad / {mean_vel:>8.2} uu/s",
            frames = score.frames_compared,
            mean_ball = score.mean_ball_distance,
            max_ball = score.max_ball_distance,
            mean_pos = score.cars.mean_position_distance,
            mean_rot = score.cars.mean_rotation_distance,
            mean_vel = score.cars.mean_velocity_distance,
        );
    }
}

fn parse_max_timestamp_delta_secs(raw: Option<String>) -> Result<f32, String> {
    match raw {
        Some(raw) => raw
            .parse::<f32>()
            .map_err(|_| format!("invalid max-timestamp-delta-secs: {raw:?}")),
        None => Ok(DEFAULT_MAX_TIMESTAMP_DELTA_SECS),
    }
}

fn parse_window_secs(raw: Option<String>) -> Result<f32, String> {
    match raw {
        Some(raw) => raw
            .parse::<f32>()
            .map_err(|_| format!("invalid window-secs: {raw:?}")),
        None => Ok(DEFAULT_GROWTH_WINDOW_SECS),
    }
}

fn usage() -> &'static str {
    "usage:\n  rb-verify <replay-file> <capture-file> [max-timestamp-delta-secs]\n  rb-verify --self <capture-file> [max-timestamp-delta-secs]\n  rb-verify --self-growth <capture-file> [window-secs] [max-timestamp-delta-secs]"
}

fn main() -> ExitCode {
    let mut args = env::args().skip(1);
    let Some(first) = args.next() else {
        eprintln!("{}", usage());
        return ExitCode::FAILURE;
    };

    if first == "--self-growth" {
        let Some(capture_path) = args.next() else {
            eprintln!("{}", usage());
            return ExitCode::FAILURE;
        };
        let window_secs = match parse_window_secs(args.next()) {
            Ok(v) => v,
            Err(e) => {
                eprintln!("{e}");
                return ExitCode::FAILURE;
            }
        };
        let max_timestamp_delta_secs = match parse_max_timestamp_delta_secs(args.next()) {
            Ok(v) => v,
            Err(e) => {
                eprintln!("{e}");
                return ExitCode::FAILURE;
            }
        };
        return match score_capture_growth(capture_path, max_timestamp_delta_secs, window_secs) {
            Err(e) => {
                eprintln!("ingestion failed: {e}");
                ExitCode::FAILURE
            }
            Ok(windows) => {
                print_growth(&windows);
                ExitCode::SUCCESS
            }
        };
    }

    let result = if first == "--self" {
        let Some(capture_path) = args.next() else {
            eprintln!("{}", usage());
            return ExitCode::FAILURE;
        };
        let max_timestamp_delta_secs = match parse_max_timestamp_delta_secs(args.next()) {
            Ok(v) => v,
            Err(e) => {
                eprintln!("{e}");
                return ExitCode::FAILURE;
            }
        };
        score_capture_against_candidate(capture_path, max_timestamp_delta_secs)
    } else {
        let Some(capture_path) = args.next() else {
            eprintln!("{}", usage());
            return ExitCode::FAILURE;
        };
        let max_timestamp_delta_secs = match parse_max_timestamp_delta_secs(args.next()) {
            Ok(v) => v,
            Err(e) => {
                eprintln!("{e}");
                return ExitCode::FAILURE;
            }
        };
        score_replay_against_capture(first, capture_path, max_timestamp_delta_secs)
    };

    match result {
        Err(e) => {
            eprintln!("ingestion failed: {e}");
            ExitCode::FAILURE
        }
        Ok(score) => {
            print_score(&score);
            ExitCode::SUCCESS
        }
    }
}
