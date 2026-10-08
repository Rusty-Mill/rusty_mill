//! `bc-clone <file.replay> [--out doc.json]` — decode a replay and print the
//! ballchasing-shaped stats document (the `GET /replays/{id}` JSON schema).

use bc_clone::{ballchasing_document, html};
use replay_analyzer::analyze::build_canonical;
use replay_analyzer::decode::boxcars_adapter::BoxcarsParser;
use replay_analyzer::decode::ReplayParser;
use std::path::Path;
use std::process::ExitCode;

const USAGE: &str = "usage: bc-clone <file.replay> [--out <doc.json>] [--html <dash.html>]";

fn main() -> ExitCode {
    match run() {
        Ok(()) => ExitCode::SUCCESS,
        Err(e) => {
            eprintln!("error: {e}");
            ExitCode::FAILURE
        }
    }
}

fn run() -> Result<(), Box<dyn std::error::Error>> {
    let mut replay = None;
    let mut out = None;
    let mut html_out = None;
    let mut it = std::env::args().skip(1);
    while let Some(a) = it.next() {
        match a.as_str() {
            "--out" => out = Some(it.next().ok_or("--out needs a path")?),
            "--html" => html_out = Some(it.next().ok_or("--html needs a path")?),
            "-h" | "--help" => {
                println!("{USAGE}");
                return Ok(());
            }
            other if other.starts_with('-') => return Err(format!("unknown flag: {other}").into()),
            other => {
                if replay.is_some() {
                    return Err(format!("unexpected argument: {other}").into());
                }
                replay = Some(other.to_string());
            }
        }
    }
    let replay = replay.ok_or(USAGE)?;

    let data = std::fs::read(&replay)?;
    let decoded = BoxcarsParser::new().parse(&data)?;
    let id = Path::new(&replay)
        .file_stem()
        .and_then(|s| s.to_str())
        .unwrap_or("replay")
        .to_string();
    let canonical = build_canonical(&decoded, id);
    let doc = ballchasing_document(&canonical);

    if let Some(path) = &html_out {
        std::fs::write(path, html::render_html(&doc, &canonical))?;
        eprintln!("wrote ballchasing-style dashboard -> {path}");
    }

    // JSON: to --out if given; otherwise stdout *unless* an HTML dashboard was the
    // only requested output.
    let json = serde_json::to_string_pretty(&doc)?;
    match (&out, &html_out) {
        (Some(path), _) => {
            std::fs::write(path, json)?;
            eprintln!("wrote ballchasing-shaped document -> {path}");
        }
        (None, None) => println!("{json}"),
        (None, Some(_)) => {}
    }
    Ok(())
}
