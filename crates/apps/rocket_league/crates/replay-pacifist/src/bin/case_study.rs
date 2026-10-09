//! Per-player, per-match case study: reads a `fetch_player_history.py`
//! case-study manifest (`assets/case-studies/<slug>/manifest.json` — one
//! named player's full ranked-doubles history, chronological, ballchasing's
//! own rank per match) and prints, in date order, both the scoring crate's
//! decision-discipline composite and the Pacifist score for that player —
//! so a real, dated rank change can be checked against what either engine
//! says moved.
//!
//! Usage: `cargo run --release -p replay-pacifist --features corpus-validate
//! --bin case_study -- <manifest.json> <player-name>`
//!
//! Replays failing the same lobby-completeness gate the rank-assessment
//! series and `validate_pacifist` use are printed but excluded from the
//! bucket-mean summary, exactly like a gated corpus replay would be.

use std::collections::BTreeMap;
use std::error::Error;
use std::path::{Path, PathBuf};

use replay_analyzer::analyze::build_canonical;
use replay_analyzer::decode::boxcars_adapter::BoxcarsParser;
use replay_analyzer::decode::ReplayParser;
use replay_pacifist::bridge::{possession_spans, shots, timeline_from_canonical, touches};
use replay_pacifist::context::MatchContext;
use replay_pacifist::roster;
use replay_pacifist::scoring::Analyzer as PacifistAnalyzer;
use replay_scoring::coverage::{lobby_fully_present, CoverageConfig};
use replay_scoring::{score_all, ScoreConfig};
use serde::Deserialize;

const USAGE: &str = "usage: case_study <manifest.json> <player-name>";

#[derive(Deserialize)]
struct Entry {
    id: String,
    file: String,
    date: String,
    tier: i32,
    bucket: String,
}

/// One player's readout for one match.
struct Row {
    date: String,
    tier: i32,
    bucket: String,
    gated: bool,
    composite: Option<f32>,
    pacifist: Option<f32>,
    pacifist_confidence: f32,
}

fn main() -> Result<(), Box<dyn Error>> {
    let mut args = std::env::args().skip(1);
    let manifest_path = PathBuf::from(args.next().ok_or(USAGE)?);
    let player_name = args.next().ok_or(USAGE)?;
    let dir = manifest_path
        .parent()
        .unwrap_or(Path::new("."))
        .to_path_buf();

    let entries: Vec<Entry> = serde_json::from_slice(&std::fs::read(&manifest_path)?)?;
    eprintln!(
        "{} matches for {player_name:?} in {}",
        entries.len(),
        manifest_path.display()
    );

    let score_cfg = ScoreConfig::default();
    let pacifist = PacifistAnalyzer::default();
    let coverage = CoverageConfig::default();

    let mut rows = Vec::new();
    for e in &entries {
        let Ok(data) = std::fs::read(dir.join(&e.file)) else {
            eprintln!("skip {}: file missing", e.id);
            continue;
        };
        let Ok(decoded) = BoxcarsParser::new().parse(&data) else {
            eprintln!("skip {}: could not decode", e.id);
            continue;
        };
        let canonical = build_canonical(&decoded, e.id.clone());
        if canonical.team_size != Some(2) {
            continue;
        }
        let gated = !lobby_fully_present(
            &canonical.tracks,
            &canonical.events,
            canonical.duration_s,
            &coverage,
        );

        let composite = score_all(&canonical, &score_cfg)
            .into_iter()
            .find(|r| r.target_player == player_name)
            .map(|r| r.composite);

        let timeline = timeline_from_canonical(&canonical);
        let target = roster(&timeline)
            .into_iter()
            .find(|p| p.name.as_deref() == Some(player_name.as_str()));
        let (pv, pc) = match target {
            Some(p) => {
                let spans = possession_spans(&canonical);
                let ctx = MatchContext::derive_with_possession(
                    &timeline,
                    pacifist.context_config(),
                    &spans,
                )
                .with_shots(shots(&canonical))
                .with_touches(touches(&canonical));
                let s = pacifist.score_player_in(&ctx, p.player);
                (s.value.map(|v| v.get()), s.confidence.get())
            }
            None => (None, 0.0),
        };

        rows.push(Row {
            date: e.date.clone(),
            tier: e.tier,
            bucket: e.bucket.clone(),
            gated,
            composite,
            pacifist: pv,
            pacifist_confidence: pc,
        });
    }

    eprintln!(
        "\n{:<26} {:>4}  {:<14} {:>6} {:>9} {:>9}   pconf",
        "date", "tier", "bucket", "gate", "composite", "pacifist"
    );
    for r in &rows {
        eprintln!(
            "{:<26} {:>4}  {:<14} {:>6} {:>9} {:>9.1}   {:.2}",
            r.date,
            r.tier,
            r.bucket,
            if r.gated { "GATED" } else { "ok" },
            r.composite
                .map(|v| format!("{v:.1}"))
                .unwrap_or_else(|| "  n/a".into()),
            r.pacifist.unwrap_or(0.0),
            r.pacifist_confidence,
        );
    }

    // Bucket means, ungated only — the before/after summary a real rank
    // change is judged by.
    let mut by_bucket: BTreeMap<String, (Vec<f32>, Vec<f32>)> = BTreeMap::new();
    for r in rows.iter().filter(|r| !r.gated) {
        let entry = by_bucket.entry(r.bucket.clone()).or_default();
        if let Some(c) = r.composite {
            entry.0.push(c);
        }
        if let Some(p) = r.pacifist {
            entry.1.push(p);
        }
    }
    eprintln!("\n=== bucket means (ungated matches only) ===");
    for (bucket, (composites, pacifists)) in &by_bucket {
        let mean = |v: &[f32]| v.iter().sum::<f32>() / v.len().max(1) as f32;
        eprintln!(
            "  {:<14} n={:<4} composite mean={:.1}   pacifist mean={:.1}",
            bucket,
            composites.len().max(pacifists.len()),
            mean(composites),
            mean(pacifists),
        );
    }

    Ok(())
}
