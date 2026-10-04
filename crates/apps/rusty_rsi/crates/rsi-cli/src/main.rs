//! `rsi`: the composition root of the `rusty_rsi` harness (ADR-0005).
//!
//! Only the internal entry points exist so far; `run`, `calibrate` and
//! `report` arrive with the inner and outer loops (P3, P4).
//!
//! - `rsi __sandbox <helper args>`: confine this process and exec a
//!   program. Spawned by the executor; never run by hand.
//! - `rsi __grade --task DIR --split SPLIT --output FILE`: score a
//!   solution's output. The only process that reads private labels.

use std::ffi::OsString;
use std::process::ExitCode;

use rsi_runtime::{grading, sandbox};

fn main() -> ExitCode {
    let args: Vec<OsString> = std::env::args_os().skip(1).collect();
    let Some((command, rest)) = args.split_first() else {
        return usage();
    };
    match command.to_str() {
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
    eprintln!("usage: rsi __sandbox <helper arguments> | rsi __grade --task DIR --split SPLIT --output FILE  (internal)");
    ExitCode::from(2)
}
