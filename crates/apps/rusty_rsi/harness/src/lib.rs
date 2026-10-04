//! a0: the baseline inner agent of `rusty_rsi` (ADR-0005 §3).
//!
//! a0 ports AIDE0: it drafts five solutions, then repeatedly either debugs
//! a random buggy leaf (probability 0.5, debug depth at most 3) or improves
//! the best solution so far, showing the model the full history every time.
//! It submits each new best solution as soon as it scores.
//!
//! **This crate is the outer loop's mutable surface.** Candidates rewrite
//! `src/**`; nothing else. The runtime builds a candidate with plain rustc,
//! compiling this file as the binary's root:
//!
//! ```text
//! rustc --edition 2021 -O --crate-type bin --crate-name rsi_harness src/lib.rs
//! ```
//!
//! So the agent is std-only, and [`main`] is its entry point. Its contract
//! with the runtime:
//!
//! - The working directory holds `task.md`, the task description.
//! - Standard input is the broker socket; it is the only way out. The
//!   sandbox refuses every new socket, file outside the working directory
//!   and process outside the job.
//! - Arguments are `--seed N`. Given the same seed and the same broker
//!   responses, the agent must make the same requests: that is what lets a
//!   run be replayed.

mod agent;
mod broker;
mod prompt;
mod rng;

use std::process::ExitCode;

/// The agent's entry point.
pub fn main() -> ExitCode {
    match run() {
        Ok(()) => ExitCode::SUCCESS,
        Err(error) => {
            eprintln!("a0: {error}");
            ExitCode::FAILURE
        }
    }
}

fn run() -> Result<(), String> {
    let seed = parse_seed(std::env::args().skip(1))?;
    let task = std::fs::read_to_string("task.md").map_err(|e| format!("reading task.md: {e}"))?;
    let mut broker = connect()?;
    agent::Agent::new(task, seed)
        .run(&mut broker)
        .map_err(|e| format!("broker: {e}"))
}

#[cfg(unix)]
fn connect() -> Result<broker::Wire<std::os::unix::net::UnixStream>, String> {
    use std::os::fd::AsFd;
    let socket = std::io::stdin()
        .as_fd()
        .try_clone_to_owned()
        .map_err(|e| format!("taking the broker socket from stdin: {e}"))?;
    Ok(broker::Wire::new(socket.into()))
}

#[cfg(not(unix))]
fn connect() -> Result<broker::Wire<std::fs::File>, String> {
    Err("the broker socket is a Unix socket".to_owned())
}

fn parse_seed(mut args: impl Iterator<Item = String>) -> Result<u64, String> {
    match (args.next().as_deref(), args.next(), args.next()) {
        (Some("--seed"), Some(seed), None) => seed
            .parse()
            .map_err(|_| format!("--seed {seed} is not a number")),
        _ => Err("usage: rsi-harness --seed N".to_owned()),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parses_the_seed() {
        let args = |items: &[&str]| items.iter().map(|s| (*s).to_owned()).collect::<Vec<_>>();
        assert_eq!(parse_seed(args(&["--seed", "42"]).into_iter()), Ok(42));
        assert!(parse_seed(args(&["--seed"]).into_iter()).is_err());
        assert!(parse_seed(args(&["--seed", "x"]).into_iter()).is_err());
        assert!(parse_seed(args(&["--seed", "1", "2"]).into_iter()).is_err());
    }
}
