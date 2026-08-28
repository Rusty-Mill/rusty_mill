//! `rb-verify`: Phase 0 verification pipeline entry point.
//!
//! Thin CLI over `rb_verify_cli::score_replay_against_capture` — argument
//! parsing and human-readable output only, no logic of its own (see
//! `lib.rs` for why the actual wiring lives there instead).

use rb_verify_cli::score_replay_against_capture;
use std::env;
use std::process::ExitCode;

fn main() -> ExitCode {
    let mut args = env::args().skip(1);
    let (Some(replay_path), Some(capture_path)) = (args.next(), args.next()) else {
        eprintln!("usage: rb-verify <replay-file> <capture-file>");
        return ExitCode::FAILURE;
    };

    match score_replay_against_capture(replay_path, capture_path) {
        Err(e) => {
            eprintln!("ingestion failed: {e}");
            ExitCode::FAILURE
        }
        Ok(score) => {
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
            ExitCode::SUCCESS
        }
    }
}
