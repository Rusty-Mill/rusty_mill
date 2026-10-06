//! `rusty-bot`: the sandbox helper and the fleet runner in one binary.
//!
//! - `rusty-bot __sandbox <helper args>`: confine this process and exec a
//!   bot's program. Started by the executor, never by hand; it must be
//!   single-threaded from birth, which a fresh process is.
//! - `rusty-bot run <bots.json> <state dir>`: start every bot in the file
//!   under its own sandbox, print each one's end, and exit when the last
//!   has ended. The state directory holds the executor's status files and
//!   must be outside every workspace.

use std::ffi::OsString;
use std::process::ExitCode;
use std::time::Duration;

fn main() -> ExitCode {
    let args: Vec<OsString> = std::env::args_os().skip(1).collect();
    match args.first().and_then(|a| a.to_str()) {
        Some("__sandbox") => {
            let error = rusty_sandbox::run_helper(&args[1..]);
            eprintln!("rusty-bot __sandbox: {error}");
            ExitCode::from(rusty_sandbox::SETUP_FAILED)
        }
        Some("run") if args.len() == 3 => match run(&args[1], &args[2]) {
            Ok(()) => ExitCode::SUCCESS,
            Err(e) => {
                eprintln!("rusty-bot: {e}");
                ExitCode::FAILURE
            }
        },
        _ => {
            eprintln!("usage: rusty-bot run <bots.json> <state dir>");
            ExitCode::FAILURE
        }
    }
}

fn run(bots: &OsString, state: &OsString) -> Result<(), String> {
    let json =
        std::fs::read_to_string(bots).map_err(|e| format!("{}: {e}", bots.to_string_lossy()))?;
    let specs = rusty_bot::load(&json).map_err(|e| e.to_string())?;
    let me = std::env::current_exe().map_err(|e| format!("locating this binary: {e}"))?;
    let executor = rusty_bot::executor(&me, std::path::Path::new(state));
    let mut fleet = rusty_bot::Fleet::start(&executor, &specs).map_err(|e| e.to_string())?;
    for bot in &fleet.bots {
        println!("[{}] started", bot.name);
    }
    let mut reported = vec![false; fleet.bots.len()];
    while fleet.any_running() {
        report(&mut fleet, &mut reported);
        std::thread::sleep(Duration::from_millis(200));
    }
    report(&mut fleet, &mut reported);
    Ok(())
}

fn report(fleet: &mut rusty_bot::Fleet, reported: &mut [bool]) {
    for (bot, done) in fleet.bots.iter_mut().zip(reported.iter_mut()) {
        if *done {
            continue;
        }
        let name = bot.name.clone();
        let Some(outcome) = bot.outcome() else {
            continue;
        };
        *done = true;
        match outcome {
            Ok(outcome) => println!(
                "[{}] ended: {:?} after {:.1}s; stderr: {}",
                name,
                outcome.termination,
                outcome.wall.as_secs_f64(),
                String::from_utf8_lossy(&outcome.stderr).trim()
            ),
            Err(e) => println!("[{name}] failed: {e}"),
        }
    }
}
