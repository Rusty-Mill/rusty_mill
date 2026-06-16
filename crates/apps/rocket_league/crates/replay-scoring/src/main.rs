//! CLI: parse a `.replay` via the analyzer, score players, print a summary and
//! optionally write the report(s) as JSON.

use replay_analyzer::analyze::build_canonical;
use replay_analyzer::decode::boxcars_adapter::BoxcarsParser;
use replay_analyzer::decode::ReplayParser;
use replay_scoring::heatmap::{occupancy, render_svg, touch_points};
use replay_scoring::lobby::assemble;
use replay_scoring::render::html;
use replay_scoring::{score_all, score_by_name, Report, ScoreConfig};
use std::error::Error;
use std::path::Path;
use std::process::ExitCode;

const USAGE: &str = "usage: replay-scoring [<file.replay>] [--player <name>] \
[--config <cfg.json>] [--json <out.json>] [--html <out.html>] \
[--dump-canonical <out.json>] [--from-canonical <in.json>]";

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
    let mut html_out = None;
    let mut dump_canonical: Option<String> = None;
    let mut from_canonical: Option<String> = None;
    let mut it = std::env::args().skip(1);
    while let Some(arg) = it.next() {
        match arg.as_str() {
            "--player" => player = Some(it.next().ok_or("--player needs a name")?),
            "--json" => json_out = Some(it.next().ok_or("--json needs a path")?),
            "--config" => config_path = Some(it.next().ok_or("--config needs a path")?),
            "--html" => html_out = Some(it.next().ok_or("--html needs a path")?),
            "--dump-canonical" => {
                dump_canonical = Some(it.next().ok_or("--dump-canonical needs a path")?)
            }
            "--from-canonical" => {
                from_canonical = Some(it.next().ok_or("--from-canonical needs a path")?)
            }
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
    // Build the canonical match either by parsing a `.replay` or by loading a
    // cached canonical blob — the re-score path runs the identical pure core with
    // no re-parse (service §9). `CanonicalMatch` is serde round-trippable.
    let canonical = match &from_canonical {
        Some(path) => serde_json::from_slice(&std::fs::read(path)?)?,
        None => {
            let replay = replay.as_deref().ok_or(USAGE)?;
            let data = std::fs::read(replay)?;
            let decoded = BoxcarsParser::new().parse(&data)?;
            let replay_id = Path::new(replay)
                .file_stem()
                .and_then(|s| s.to_str())
                .unwrap_or("replay")
                .to_string();
            build_canonical(&decoded, replay_id)
        }
    };

    if let Some(path) = &dump_canonical {
        std::fs::write(path, serde_json::to_vec(&canonical)?)?;
        eprintln!("wrote canonical -> {path}");
    }

    if !replay_analyzer::field::is_standard_geometry(canonical.map.as_deref()) {
        eprintln!(
            "warning: non-standard map ({:?}) — positional metrics assume standard Soccar; reports are flagged low-confidence",
            canonical.map
        );
    }

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

    if let Some(path) = &html_out {
        // The HTML report is always a full-lobby artifact (§4.10), independent of
        // --player: every player, the comparison table, and per-player heatmaps.
        let lobby = assemble(&canonical, &cfg);
        let heatmaps: Vec<(i32, String)> = lobby
            .players
            .iter()
            .map(|p| {
                let occ = occupancy(&canonical, p.target_pri, 12, 15);
                let touches = touch_points(&canonical, p.target_pri);
                (p.target_pri, render_svg(&occ, &touches))
            })
            .collect();
        std::fs::write(path, html(&lobby, &heatmaps))?;
        eprintln!("\nwrote HTML report -> {path}");
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
