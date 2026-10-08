//! CLI: score one named player across several `.replay` files and print their
//! aggregated Pacifist history — the multi-match rollup
//! (`replay_pacifist::history`) motivated by the corpus validation's finding
//! that a single match's per-dimension resolution is near a hard ceiling.
//!
//! Usage: `pacifist_history <player-name> <file1.replay> [file2.replay ...]`
//!
//! Files should be passed **oldest first** — the trend line and "most recent
//! Major" reporting both read that order directly (this tool has no notion of
//! match dates; see `replay_pacifist::history`'s module docs). The player
//! name is matched exactly against each replay's roster; a file where the
//! name doesn't appear is skipped with a warning, not an error, so one typo'd
//! or wrong-lobby file doesn't kill the whole run.

use replay_analyzer::analyze::build_canonical;
use replay_analyzer::decode::boxcars_adapter::BoxcarsParser;
use replay_analyzer::decode::ReplayParser;
use replay_pacifist::bridge::{possession_spans, shots, timeline_from_canonical, touches};
use replay_pacifist::context::MatchContext;
use replay_pacifist::history::{aggregate_history, HistoryConfig, MatchRecord};
use replay_pacifist::scoring::{Analyzer, ScoringConfig};
use replay_pacifist::{roster, PACIFIST_CONFIG_VERSION};
use std::error::Error;
use std::path::Path;
use std::process::ExitCode;

const USAGE: &str =
    "usage: pacifist_history <player-name> <file1.replay> [file2.replay ...]  (oldest first)";

fn main() -> ExitCode {
    match run() {
        Ok(()) => ExitCode::SUCCESS,
        Err(e) => {
            eprintln!("error: {e}");
            ExitCode::FAILURE
        }
    }
}

fn run() -> Result<(), Box<dyn Error>> {
    let mut args = std::env::args().skip(1);
    let player_name = args.next().ok_or(USAGE)?;
    let paths: Vec<String> = args.collect();
    if paths.is_empty() {
        return Err(USAGE.into());
    }

    let analyzer = Analyzer::default();
    let scoring = ScoringConfig::default();
    let mut records = Vec::new();

    for path in &paths {
        let label = Path::new(path)
            .file_stem()
            .and_then(|s| s.to_str())
            .unwrap_or(path)
            .to_string();
        let data = std::fs::read(path)?;
        let Ok(decoded) = BoxcarsParser::new().parse(&data) else {
            eprintln!("skip {label}: could not decode");
            continue;
        };
        let canonical = build_canonical(&decoded, label.clone());
        let timeline = timeline_from_canonical(&canonical);

        let Some(entry) = roster(&timeline)
            .into_iter()
            .find(|e| e.name.as_deref() == Some(player_name.as_str()))
        else {
            eprintln!("skip {label}: {player_name:?} not in this replay's roster");
            continue;
        };

        let spans = possession_spans(&canonical);
        let ctx =
            MatchContext::derive_with_possession(&timeline, analyzer.context_config(), &spans)
                .with_shots(shots(&canonical))
                .with_touches(touches(&canonical));
        let score = analyzer.score_player_in(&ctx, entry.player);
        records.push(MatchRecord { label, score });
    }

    if records.is_empty() {
        return Err(format!(
            "{player_name:?} was not found in any of the {} files",
            paths.len()
        )
        .into());
    }

    let hist = aggregate_history(&records, &scoring, &HistoryConfig::default());

    eprintln!(
        "config={PACIFIST_CONFIG_VERSION}  player={player_name:?}  matches={}",
        hist.matches
    );
    eprintln!(
        "trend: {}",
        records
            .iter()
            .zip(&hist.trend)
            .map(|(r, v)| match v {
                Some(v) => format!("{}={:.1}", r.label, v.get()),
                None => format!("{}=—", r.label),
            })
            .collect::<Vec<_>>()
            .join("  ")
    );

    match hist.value {
        Some(v) => eprintln!(
            "\n== history ==  pacifist score {:.1}  (confidence {:.2})  [average technique — not Major-capped; see faults below]",
            v.get(),
            hist.confidence.get()
        ),
        None => eprintln!("\n== history ==  no scoreable evidence across any match"),
    }

    eprintln!(
        "faults: {} minors, {} majors across {} matches — {}/{} matches passed FM-1 ({:.0}%)",
        hist.faults.minors_total,
        hist.faults.majors_total,
        hist.faults.matches,
        hist.faults.matches_passed,
        hist.faults.matches,
        hist.faults.pass_rate * 100.0,
    );
    for (label, f) in &hist.faults.recent_majors {
        eprintln!(
            "  MAJOR  {label} @{:>6.1}s  [{}] {}",
            f.t, f.criterion, f.detail
        );
    }

    for d in &hist.dimensions {
        let value = d
            .value
            .map(|v| format!("{:>5.1}", v.get()))
            .unwrap_or_else(|| " n/a ".into());
        eprintln!(
            "  {:<22} value={value}  conf={:.2}  influence={:>4.1}%  opportunities={:<4} matches={}",
            d.dimension.label(),
            d.confidence.get(),
            d.influence * 100.0,
            d.opportunities,
            d.matches_with_data,
        );
    }

    Ok(())
}
