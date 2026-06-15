//! CLI: parse a `.replay` via the analyzer, score players, print a summary and
//! optionally write the report(s) as JSON.

use replay_analyzer::analyze::build_canonical;
use replay_analyzer::decode::boxcars_adapter::BoxcarsParser;
use replay_analyzer::decode::ReplayParser;
use replay_scoring::{score_all, score_by_name, Report, ScoreConfig};
use std::error::Error;
use std::path::Path;
use std::process::ExitCode;

const USAGE: &str =
    "usage: replay-scoring <file.replay> [--player <name>] [--config <cfg.json>] [--json <out.json>]";

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
    let mut replay = None;
    let mut player = None;
    let mut json_out = None;
    let mut config_path = None;
    let mut it = std::env::args().skip(1);
    while let Some(arg) = it.next() {
        match arg.as_str() {
            "--player" => player = Some(it.next().ok_or("--player needs a name")?),
            "--json" => json_out = Some(it.next().ok_or("--json needs a path")?),
            "--config" => config_path = Some(it.next().ok_or("--config needs a path")?),
            "-h" | "--help" => {
                println!("{USAGE}");
                return Ok(());
            }
            other if other.starts_with('-') => {
                return Err(format!("unknown flag {other}\n{USAGE}").into())
            }
            other if replay.is_none() => replay = Some(other.to_string()),
            other => return Err(format!("unexpected arg {other}\n{USAGE}").into()),
        }
    }
    let replay = replay.ok_or(USAGE)?;

    let data = std::fs::read(&replay)?;
    let decoded = BoxcarsParser::new().parse(&data)?;
    let replay_id = Path::new(&replay)
        .file_stem()
        .and_then(|s| s.to_str())
        .unwrap_or("replay")
        .to_string();
    let canonical = build_canonical(&decoded, replay_id);

    let cfg = match &config_path {
        Some(p) => serde_json::from_slice(&std::fs::read(p)?)?,
        None => ScoreConfig::default(),
    };
    let reports: Vec<Report> = match &player {
        Some(name) => vec![score_by_name(&canonical, name, &cfg)
            .ok_or_else(|| format!("player '{name}' not found in replay"))?],
        None => score_all(&canonical, &cfg),
    };

    for r in &reports {
        print_report(r);
    }

    if let Some(path) = &json_out {
        let payload = if reports.len() == 1 {
            serde_json::to_vec_pretty(&reports[0])?
        } else {
            serde_json::to_vec_pretty(&reports)?
        };
        std::fs::write(path, payload)?;
        eprintln!("\nwrote report(s) -> {path}");
    }
    Ok(())
}

fn print_report(r: &Report) {
    eprintln!(
        "\n== {} (pri {}, team {:?}) ==",
        r.target_player, r.target_pri, r.target_team
    );
    eprintln!(
        "config={} parser={}",
        r.score_config_version, r.parser_version
    );
    eprintln!(
        "composite={:.1}  [1st={:.1} 2nd={:.1} gen={:.1}]  {}  type={}  conf={:?}",
        r.composite, r.first_man, r.second_man, r.general, r.licence, r.player_type, r.confidence
    );
    eprintln!("main leak: {} -> {}", r.main_leak, r.focus_chapter);
    eprintln!("metrics:");
    for m in &r.metrics {
        let role = format!("{:?}", m.role);
        match m.raw {
            Some(raw) => eprintln!(
                "  {:<26} {:<8} raw={:>8.3} norm={:>5.1} w={:.2}",
                m.key, role, raw, m.normalized, m.effective_weight
            ),
            None => eprintln!("  {:<26} {:<8} raw=   n/a   norm=  -", m.key, role),
        }
    }
}
