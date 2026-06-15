//! CLI: decode a `.replay` and emit a self-contained 3D HTML viewer.
//!
//! ```text
//! replay-viewer <file.replay> [--html <out.html>] [--json <scene.json>] [--no-skills]
//! ```
//! With no output flag, writes `<stem>.html` next to the working directory. The
//! viewer overlays detected skills on the timeline by default (`--no-skills` to
//! show only match events).

use std::error::Error;
use std::path::Path;
use std::process::ExitCode;

use replay_analyzer::analyze::build_canonical;
use replay_analyzer::decode::boxcars_adapter::BoxcarsParser;
use replay_analyzer::decode::ReplayParser;
use replay_skills::{detect_all, SkillConfig};
use replay_viewer::{build_scene, html};

const USAGE: &str =
    "usage: replay-viewer <file.replay> [--html <out.html>] [--json <scene.json>] [--no-skills]";

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
    let mut html_out = None;
    let mut json_out = None;
    let mut skills = true;
    let mut it = std::env::args().skip(1);
    while let Some(arg) = it.next() {
        match arg.as_str() {
            "--html" => html_out = Some(it.next().ok_or("--html needs a path")?),
            "--json" => json_out = Some(it.next().ok_or("--json needs a path")?),
            "--no-skills" => skills = false,
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
    let stem = Path::new(&replay)
        .file_stem()
        .and_then(|s| s.to_str())
        .unwrap_or("replay")
        .to_string();
    let canonical = build_canonical(&decoded, stem.clone());

    let instances = if skills {
        detect_all(&canonical, &SkillConfig::default()).instances
    } else {
        Vec::new()
    };
    let scene = build_scene(&canonical, &instances);

    eprintln!(
        "scene: {} frames @ {:.0}Hz, {} players, {} timeline events (~{:.0}s)",
        scene.frames.len(),
        scene.hz,
        scene.players.len(),
        scene.events.len(),
        scene.duration_s,
    );

    if let Some(path) = &json_out {
        std::fs::write(path, serde_json::to_vec_pretty(&scene)?)?;
        eprintln!("wrote scene JSON -> {path}");
    }
    // Write HTML if requested, or by default when no JSON-only output was asked.
    let html_path = html_out.or_else(|| (json_out.is_none()).then(|| format!("{stem}.html")));
    if let Some(path) = html_path {
        std::fs::write(&path, html(&scene))?;
        eprintln!("wrote 3D viewer -> {path}  (open in a browser)");
    }
    Ok(())
}
