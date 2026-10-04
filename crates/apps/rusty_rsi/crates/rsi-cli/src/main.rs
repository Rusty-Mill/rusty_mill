//! `rsi`: the composition root of the `rusty_rsi` harness (ADR-0005).
//!
//! - `rsi calibrate`, `rsi run`, `rsi report`: the outer loop (see
//!   [`outer`]).
//! - `rsi inner ...`: one inner run of a harness on one task with a live
//!   model (see [`inner`]).
//! - `rsi __sandbox <helper args>`: confine this process and exec a
//!   program. Spawned by the executor; never run by hand.
//! - `rsi __grade --task DIR --split SPLIT --output FILE`: score a
//!   solution's output. The only process that reads private labels.

use std::ffi::OsString;
use std::process::ExitCode;

use rsi_runtime::{grading, sandbox};

mod config;
mod flags;
mod inner;
mod outer;

fn main() -> ExitCode {
    let args: Vec<OsString> = std::env::args_os().skip(1).collect();
    let Some((command, rest)) = args.split_first() else {
        return usage();
    };
    let command = command.to_str().unwrap_or_default();
    let report = match command {
        "inner" => inner::main(rest),
        "calibrate" => outer::calibrate_main(rest),
        "run" => outer::run_main(rest),
        "report" => outer::report_main(rest),
        _ => return internal(command, rest),
    };
    match report {
        Ok(report) => {
            print!("{report}");
            ExitCode::SUCCESS
        }
        Err(error) => {
            eprintln!("rsi {command}: {error}");
            ExitCode::FAILURE
        }
    }
}

/// The internal entry points, spawned by the runtime itself.
fn internal(command: &str, rest: &[OsString]) -> ExitCode {
    match command {
        "__sandbox" => {
            let error = sandbox::run_helper(rest);
            eprintln!("rsi-sandbox: {error}");
            ExitCode::from(sandbox::SETUP_FAILED)
        }
        "__grade" => match grading::grade_main(rest) {
            Ok(score) => {
                println!("{}", grading::score_line(score));
                ExitCode::SUCCESS
            }
            Err(error) => {
                eprintln!("rsi-grade: {error}");
                ExitCode::FAILURE
            }
        },
        _ => usage(),
    }
}

fn usage() -> ExitCode {
    eprintln!(
        "usage:\n\
         rsi calibrate --repo DIR --tasks DIR --tokens N --wall-secs N --out FILE [--rounds 5] [--z 1.645]\n\
         rsi run --repo DIR --tasks DIR --run-dir DIR --steps N --tokens N --wall-secs N (--calibration FILE | --margin X)\n\
         rsi report --run-dir DIR [--replay --repo DIR --tasks DIR]\n\
         rsi inner --harness DIR --task DIR --tokens N --wall-secs N [--seed N] [--transcript FILE]"
    );
    ExitCode::from(2)
}
