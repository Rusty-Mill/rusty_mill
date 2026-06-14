//! CLI front-end: decode a `.replay`, build the canonical match model, print a
//! human-readable summary to stderr, and emit the model as JSON (stdout, or a
//! file via `--json <path>`).

use replay_analyzer::analyze::{self, reconstruct};
use replay_analyzer::decode::boxcars_adapter::BoxcarsParser;
use replay_analyzer::decode::ReplayParser;
use replay_analyzer::model::{CanonicalMatch, Event};
use std::error::Error;
use std::path::Path;
use std::process::ExitCode;

const USAGE: &str = "usage: replay-analyzer <file.replay> [--json <out.json>]";

struct Args {
    replay: String,
    json_out: Option<String>,
}

fn parse_args() -> Result<Args, String> {
    let mut replay = None;
    let mut json_out = None;
    let mut it = std::env::args().skip(1);
    while let Some(arg) = it.next() {
        match arg.as_str() {
            "--json" => {
                json_out = Some(it.next().ok_or("--json requires a path argument")?);
            }
            "-h" | "--help" => return Err(USAGE.to_string()),
            other if other.starts_with('-') => {
                return Err(format!("unknown flag: {other}\n{USAGE}"));
            }
            other => {
                if replay.is_some() {
                    return Err(format!("unexpected extra argument: {other}\n{USAGE}"));
                }
                replay = Some(other.to_string());
            }
        }
    }
    Ok(Args {
        replay: replay.ok_or(USAGE)?,
        json_out,
    })
}

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
    let args = parse_args()?;

    let data = std::fs::read(&args.replay)?;
    let decoded = BoxcarsParser::new().parse(&data)?;

    let replay_id = Path::new(&args.replay)
        .file_stem()
        .and_then(|s| s.to_str())
        .unwrap_or("replay")
        .to_string();

    let canonical = analyze::build_canonical(&decoded, replay_id);

    print_summary(&canonical);

    match &args.json_out {
        Some(path) => {
            std::fs::write(path, serde_json::to_vec_pretty(&canonical)?)?;
            eprintln!("\nwrote canonical model -> {path}");
        }
        None => println!("{}", serde_json::to_string_pretty(&canonical)?),
    }
    Ok(())
}

fn print_summary(c: &CanonicalMatch) {
    eprintln!("== DECODE/ANALYZE SUMMARY ==");
    eprintln!("replay_id      : {}", c.replay_id);
    eprintln!("parser_version : {}", c.parser_version);
    eprintln!("analyzer       : {}", c.analyzer_version);
    eprintln!("map            : {:?}", c.map);
    eprintln!("team_size      : {:?}", c.team_size);
    eprintln!("record_fps     : {:?}", c.record_fps);
    eprintln!(
        "frames         : {}  (~{:.1}s)",
        c.num_frames, c.duration_s
    );
    eprintln!("team_scores    : {:?}", c.team_scores);

    eprintln!("players (header stats):");
    for p in &c.players {
        eprintln!(
            "  - {:<20} team={} score={} G={} A={} Sv={} Sh={}",
            p.name, p.team, p.score, p.goals, p.assists, p.saves, p.shots
        );
    }

    if let Some((min, max)) = reconstruct::ball_bounds(&c.frames) {
        eprintln!(
            "ball bounds    : x[{:.0},{:.0}] y[{:.0},{:.0}] z[{:.0},{:.0}]  \
             (expect ~x +/-4096, y +/-5120(+goal), z 0..2044)",
            min[0], max[0], min[1], max[1], min[2], max[2]
        );
    }

    eprintln!("coalesced tracks (T1):");
    for t in &c.tracks {
        let span = t
            .samples
            .first()
            .zip(t.samples.last())
            .map(|(a, b)| b.t - a.t)
            .unwrap_or(0.0);
        let boosts: Vec<u8> = t.samples.iter().filter_map(|s| s.boost).collect();
        let mean_boost = if boosts.is_empty() {
            f32::NAN
        } else {
            boosts.iter().map(|&b| b as f32).sum::<f32>() / boosts.len() as f32
        };
        eprintln!(
            "  - {:<20} pri={:<3} team={:?} segments={:<3} samples={:<6} gaps={:<3} span={:.1}s mean_boost={:.0}%",
            t.player,
            t.pri,
            t.team,
            t.num_segments,
            t.samples.len(),
            t.gaps.len(),
            span,
            replay_analyzer::field::boost_percent(mean_boost as u8),
        );
    }

    let mut counts: std::collections::BTreeMap<&str, usize> = std::collections::BTreeMap::new();
    for e in &c.events {
        let k = match e {
            Event::Kickoff { .. } => "kickoff",
            Event::Touch { .. } => "touch",
            Event::Possession { .. } => "possession",
            Event::Demo { .. } => "demo",
            Event::Goal { .. } => "goal",
        };
        *counts.entry(k).or_default() += 1;
    }
    eprintln!("events (T4)    : {counts:?}");
}
