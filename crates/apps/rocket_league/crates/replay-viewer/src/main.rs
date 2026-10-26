//! CLI: decode a `.replay`, or read a physics capture, and emit a 3D HTML viewer.
//!
//! ```text
//! replay-viewer <file.replay> [--html <out.html>] [--json <scene.json>]
//!               [--hz <rate>] [--offline] [--no-skills] [--no-roles]
//! replay-viewer <capture.jsonl> [--sim <candidate.jsonl>] [--html <out.html>]
//!               [--json <scene.json>] [--offline]
//! ```
//! A `.jsonl` input is an ADR-0005 capture; `--sim` adds the port's candidate
//! run of it (`rb-verify --self <capture> --write-candidate <file>`) as ghost
//! cars (ADR-0085). With no output flag, writes `<stem>.html`. Detected skills and 1st/2nd-man
//! roles overlay by default (`--no-skills` / `--no-roles` to omit). `--offline`
//! embeds three.js so the file needs no network; `--hz` thins the playback grid.

use std::error::Error;
use std::path::Path;
use std::process::ExitCode;

use rb_capture_ingest::CaptureFileSource;
use rb_domain::PhysicsStateSource;
use replay_analyzer::analyze::build_canonical;
use replay_analyzer::decode::boxcars_adapter::BoxcarsParser;
use replay_analyzer::decode::ReplayParser;
use replay_skills::{detect_all, SkillConfig};
use replay_viewer::{build_physics_scene, build_scene, html, Scene};

const USAGE: &str = "usage: replay-viewer <file.replay> [--html <out.html>] [--json <scene.json>] \
[--hz <rate>] [--offline] [--no-skills] [--no-roles] [--no-winprob] [--no-impact] [--no-player-stats]
       replay-viewer <capture.jsonl> [--sim <candidate.jsonl>] [--html <out.html>] [--json <scene.json>] [--offline]";

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
    let mut hz: Option<f32> = None;
    let mut skills = true;
    let mut roles = true;
    let mut winprob = true;
    let mut impact = true;
    let mut player_stats = true;
    let mut offline = false;
    let mut sim = None;
    let mut it = std::env::args().skip(1);
    while let Some(arg) = it.next() {
        match arg.as_str() {
            "--html" => html_out = Some(it.next().ok_or("--html needs a path")?),
            "--json" => json_out = Some(it.next().ok_or("--json needs a path")?),
            "--sim" => sim = Some(it.next().ok_or("--sim needs a path")?),
            "--hz" => {
                hz = Some(
                    it.next()
                        .ok_or("--hz needs a rate")?
                        .parse()
                        .map_err(|_| "--hz must be a number")?,
                )
            }
            "--no-skills" => skills = false,
            "--no-roles" => roles = false,
            "--no-winprob" => winprob = false,
            "--no-impact" => impact = false,
            "--no-player-stats" => player_stats = false,
            "--offline" => offline = true,
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
    let stem = Path::new(&replay)
        .file_stem()
        .and_then(|s| s.to_str())
        .unwrap_or("replay")
        .to_string();
    let outputs = Outputs {
        stem: stem.clone(),
        html: html_out,
        json: json_out,
        offline,
    };

    if replay.ends_with(".jsonl") {
        return capture_scene(&replay, sim.as_deref(), &stem).and_then(|s| outputs.write(&s));
    }
    if sim.is_some() {
        return Err("--sim needs a capture (.jsonl) input".into());
    }

    let data = std::fs::read(&replay)?;
    let decoded = BoxcarsParser::new().parse(&data)?;
    let canonical = build_canonical(&decoded, stem);
    if !replay_analyzer::field::is_standard_geometry(canonical.map.as_deref()) {
        eprintln!(
            "warning: non-standard map ({:?}) — the drawn field and positional overlays are approximate",
            canonical.map
        );
    }

    let instances = if skills {
        detect_all(&canonical, &SkillConfig::default()).instances
    } else {
        Vec::new()
    };
    let mut scene = build_scene(&canonical, &instances);
    // Roles map 1:1 to the full grid, so attach before any downsample.
    if roles {
        replay_viewer::attach_roles(
            &mut scene,
            &canonical,
            &replay_scoring::ScoreConfig::default(),
        );
    }
    if winprob {
        replay_viewer::attach_winprob(
            &mut scene,
            &canonical,
            &replay_value::ValueConfig::default(),
        );
    }
    if impact {
        replay_viewer::attach_impact(
            &mut scene,
            &canonical,
            &replay_value::ValueConfig::default(),
        );
    }
    if player_stats {
        replay_viewer::attach_player_stats(&mut scene, &canonical);
    }
    if let Some(target) = hz {
        replay_viewer::scene::downsample(&mut scene, target);
    }

    eprintln!(
        "scene: {} frames @ {:.0}Hz, {} players, {} timeline events (~{:.0}s)",
        scene.frames.len(),
        scene.hz,
        scene.players.len(),
        scene.events.len(),
        scene.duration_s,
    );

    outputs.write(&scene)
}

/// The scene for a capture and, with `--sim`, the port's candidate run of it.
fn capture_scene(capture: &str, sim: Option<&str>, stem: &str) -> Result<Scene, Box<dyn Error>> {
    let read = |path: &str| {
        CaptureFileSource::new(path)
            .frames()
            .map_err(|e| format!("{path}: {e}"))
    };
    let recorded = read(capture)?;
    let candidate = sim.map(read).transpose()?;
    let scene = build_physics_scene(stem, &recorded, candidate.as_deref())?;
    eprintln!(
        "scene: {} frames @ {:.0}Hz, {} cars{} (~{:.1}s)",
        scene.frames.len(),
        scene.hz,
        scene.players.len(),
        if candidate.is_some() {
            ", sim cars as ghosts"
        } else {
            ""
        },
        scene.duration_s,
    );
    Ok(scene)
}

/// Where a scene goes: `--json`, `--html`, or `<stem>.html` when neither is given.
struct Outputs {
    stem: String,
    html: Option<String>,
    json: Option<String>,
    offline: bool,
}

impl Outputs {
    fn write(self, scene: &Scene) -> Result<(), Box<dyn Error>> {
        if let Some(path) = &self.json {
            std::fs::write(path, serde_json::to_vec_pretty(scene)?)?;
            eprintln!("wrote scene JSON -> {path}");
        }
        // Write HTML if requested, or by default when no JSON-only output was asked.
        let default_html = self.json.is_none().then(|| format!("{}.html", self.stem));
        let Some(path) = self.html.or(default_html) else {
            return Ok(());
        };
        let doc = if self.offline {
            replay_viewer::html_offline(scene)
        } else {
            html(scene)
        };
        std::fs::write(&path, doc)?;
        let note = if self.offline {
            " (self-contained, no network)"
        } else {
            " (open in a browser)"
        };
        eprintln!("wrote 3D viewer -> {path}{note}");
        Ok(())
    }
}
