//! The Fair Play seed loader CLI (`FPL-FR-006`, ADR-0137): loads the
//! deck — the one embedded in the library unless `--cards` names a file —
//! and optionally people and splits, into a data directory the
//! `fair_play_server` and `rusty_fair_play` binaries serve.
//!
//! Run with:
//! `cargo run --example fair_play_seed -- <store_dir> [--cards <cards.csv>] [--people <people.csv>] [--splits <splits.csv>]`
//!
//! Idempotent: a rerun creates nothing that is already there and never
//! overwrites a live card (see `generic::fair_play::seed`). A malformed
//! row is reported by file and line and nothing is written.

use rusty_fair_play_domain::seed::{deck, parse_cards, parse_people, parse_splits, seed, SeedData};
use std::path::PathBuf;
use std::process::ExitCode;

const USAGE: &str =
    "usage: fair_play_seed <store_dir> [--cards <cards.csv>] [--people <people.csv>] [--splits <splits.csv>]";

struct Args {
    store_dir: PathBuf,
    cards: Option<PathBuf>,
    people: Option<PathBuf>,
    splits: Option<PathBuf>,
}

fn parse_args() -> Result<Args, String> {
    let mut args = std::env::args_os().skip(1);
    let mut parsed = Args {
        store_dir: PathBuf::from(args.next().ok_or(USAGE)?),
        cards: None,
        people: None,
        splits: None,
    };
    while let Some(flag) = args.next() {
        let value = args.next().map(PathBuf::from).ok_or(USAGE)?;
        match flag.to_str() {
            Some("--cards") => parsed.cards = Some(value),
            Some("--people") => parsed.people = Some(value),
            Some("--splits") => parsed.splits = Some(value),
            _ => return Err(USAGE.to_string()),
        }
    }
    Ok(parsed)
}

fn run() -> Result<String, String> {
    let args = parse_args()?;
    let data = SeedData {
        people: args
            .people
            .as_deref()
            .map(parse_people)
            .transpose()
            .map_err(|e| e.to_string())?
            .unwrap_or_default(),
        deck: match &args.cards {
            Some(path) => parse_cards(path),
            None => deck(),
        }
        .map_err(|e| e.to_string())?,
        splits: args
            .splits
            .as_deref()
            .map(parse_splits)
            .transpose()
            .map_err(|e| e.to_string())?
            .unwrap_or_default(),
        splits_label: args
            .splits
            .as_ref()
            .map(|p| p.display().to_string())
            .unwrap_or_default(),
    };
    let r = seed(&args.store_dir, &data).map_err(|e| e.to_string())?;
    Ok(format!(
        "people {}+{} baselines {}+{} cards {}+{} splits {}+{} (created+existing) in {}",
        r.people.created,
        r.people.existing,
        r.card_defaults.created,
        r.card_defaults.existing,
        r.cards.created,
        r.cards.existing,
        r.splits.created,
        r.splits.existing,
        args.store_dir.display()
    ))
}

fn main() -> ExitCode {
    match run() {
        Ok(report) => {
            println!("{report}");
            ExitCode::SUCCESS
        }
        Err(e) => {
            eprintln!("seed failed: {e}");
            ExitCode::FAILURE
        }
    }
}
