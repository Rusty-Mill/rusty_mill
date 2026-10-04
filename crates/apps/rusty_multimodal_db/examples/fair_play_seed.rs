//! The Fair Play seed loader (`FPL-FR-006`, ADR-0137): loads the
//! supplied deck, and optionally people and splits, into a data
//! directory the `fair_play_server` binary serves.
//!
//! Run with:
//! `cargo run --example fair_play_seed -- <store_dir> <cards.csv> [--people <people.csv>] [--splits <splits.csv>]`
//!
//! Idempotent: a rerun creates nothing that is already there and never
//! overwrites a live card (see `support/fair_play_seed_lib.rs`). A
//! malformed row is reported by file and line and nothing is written.

#[path = "support/fair_play_seed_lib.rs"]
mod loader;

use loader::{seed, SeedInputs};
use std::path::PathBuf;
use std::process::ExitCode;

const USAGE: &str =
    "usage: fair_play_seed <store_dir> <cards.csv> [--people <people.csv>] [--splits <splits.csv>]";

fn parse_args() -> Result<(PathBuf, SeedInputs), String> {
    let mut args = std::env::args_os().skip(1);
    let store_dir = PathBuf::from(args.next().ok_or(USAGE)?);
    let cards = PathBuf::from(args.next().ok_or(USAGE)?);
    let mut inputs = SeedInputs {
        people: None,
        cards,
        splits: None,
    };
    while let Some(flag) = args.next() {
        let value = args.next().map(PathBuf::from).ok_or(USAGE)?;
        match flag.to_str() {
            Some("--people") => inputs.people = Some(value),
            Some("--splits") => inputs.splits = Some(value),
            _ => return Err(USAGE.to_string()),
        }
    }
    Ok((store_dir, inputs))
}

fn main() -> ExitCode {
    let (store_dir, inputs) = match parse_args() {
        Ok(parsed) => parsed,
        Err(usage) => {
            eprintln!("{usage}");
            return ExitCode::FAILURE;
        }
    };
    match seed(&store_dir, &inputs) {
        Ok(r) => {
            println!(
                "people {}+{} baselines {}+{} cards {}+{} splits {}+{} (created+existing) in {}",
                r.people.created,
                r.people.existing,
                r.card_defaults.created,
                r.card_defaults.existing,
                r.cards.created,
                r.cards.existing,
                r.splits.created,
                r.splits.existing,
                store_dir.display()
            );
            ExitCode::SUCCESS
        }
        Err(e) => {
            eprintln!("seed failed: {e}");
            ExitCode::FAILURE
        }
    }
}
