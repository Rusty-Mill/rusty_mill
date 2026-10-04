//! `rsi`: the composition root of the `rusty_rsi` harness (ADR-0005).
//!
//! `run`, `calibrate` and `report` arrive with the outer loop (P4).
//!
//! - `rsi inner ...`: one inner run of a harness on one task with a live
//!   model (see [`inner`]).
//! - `rsi __sandbox <helper args>`: confine this process and exec a
//!   program. Spawned by the executor; never run by hand.
//! - `rsi __grade --task DIR --split SPLIT --output FILE`: score a
//!   solution's output. The only process that reads private labels.

use std::ffi::OsString;
use std::process::ExitCode;

use rsi_runtime::{grading, sandbox};

mod inner;

fn main() -> ExitCode {
    let args: Vec<OsString> = std::env::args_os().skip(1).collect();
    let Some((command, rest)) = args.split_first() else {
        return usage();
    };
    match command.to_str() {
        Some("inner") => match inner::main(rest) {
            Ok(report) => {
                print!("{report}");
                ExitCode::SUCCESS
            }
            Err(error) => {
                eprintln!("rsi inner: {error}");
                ExitCode::FAILURE
            }
        },
        Some("__sandbox") => {
            let error = sandbox::run_helper(rest);
            eprintln!("rsi-sandbox: {error}");
            ExitCode::from(sandbox::SETUP_FAILED)
        }
        Some("__grade") => match grading::grade_main(rest) {
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
        "usage: rsi inner --harness DIR --task DIR --tokens N --wall-secs N [--seed N] [--work DIR] [--transcript FILE]"
    );
    ExitCode::from(2)
}
