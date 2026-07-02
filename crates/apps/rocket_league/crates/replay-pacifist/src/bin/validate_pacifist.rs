//! Corpus validation harness for the Pacifist score.
//!
//! Mirrors the scoring crate's `calibrate` shape: read the labeled corpus
//! manifest, score every player with the default [`Analyzer`], join each to
//! their rank tier (team-anchored, recovering ballchasing-mangled names), and
//! report whether the Pacifist score carries rank — overall Spearman,
//! per-dimension Spearman, per-bucket means, and within-bucket Spearman (the
//! GC row tests the rank series' prediction that textbook-shaped discipline
//! metrics stop describing play at the top).
//!
//! Reuses the scoring crate's rank-join, Spearman, and lobby-completeness
//! gate rather than re-deriving them; replays failing the gate are excluded
//! exactly like the rank-assessment series excluded them.
//!
//! Usage: `cargo run --release -p replay-pacifist --features corpus-validate
//! --bin validate_pacifist [manifest.json]`

use std::collections::BTreeMap;
use std::error::Error;
use std::path::{Path, PathBuf};

use replay_analyzer::analyze::build_canonical;
use replay_analyzer::analyze::validate::GroundTruth;
use replay_analyzer::decode::boxcars_adapter::BoxcarsParser;
use replay_analyzer::decode::ReplayParser;
use replay_pacifist::bridge::timeline_from_canonical;
use replay_pacifist::context::MatchContext;
use replay_pacifist::metrics::DimensionId;
use replay_pacifist::scoring::Analyzer;
use replay_pacifist::{roster, team_sizes, Team};
use replay_scoring::calibrate::{join_ranks, spearman};
use replay_scoring::coverage::{lobby_fully_present, CoverageConfig};
use serde::Deserialize;

#[derive(Deserialize)]
struct ManifestEntry {
    #[serde(default)]
    id: String,
    file: String,
    bucket: String,
    #[serde(default)]
    ranks: BTreeMap<String, i32>,
    #[serde(default)]
    playlist: Option<String>,
    #[serde(default)]
    team_size: Option<i32>,
}

/// One scored, rank-joined player.
struct Row {
    bucket: String,
    tier: f32,
    value: f32,
    dimensions: Vec<(DimensionId, f32, f32)>, // (id, value, confidence)
}

const BUCKETS: [&str; 7] = [
    "bronze",
    "silver",
    "gold",
    "platinum",
    "diamond",
    "champion",
    "grand-champion",
];

fn main() -> Result<(), Box<dyn Error>> {
    let manifest_path = PathBuf::from(
        std::env::args()
            .nth(1)
            .unwrap_or_else(|| "assets/corpus/manifest.json".into()),
    );
    let corpus_dir = manifest_path
        .parent()
        .unwrap_or(Path::new("."))
        .to_path_buf();
    let all: Vec<ManifestEntry> = serde_json::from_slice(&std::fs::read(&manifest_path)?)?;
    let entries: Vec<ManifestEntry> = all
        .into_iter()
        .filter(|e| e.team_size == Some(2) && e.playlist.as_deref() == Some("ranked-doubles"))
        .collect();
    eprintln!(
        "corpus: {} ranked-doubles 2v2 entries from {}",
        entries.len(),
        manifest_path.display()
    );

    let ground_truth: Option<GroundTruth> =
        std::fs::read(corpus_dir.join("ballchasing_stats.json"))
            .ok()
            .and_then(|b| serde_json::from_slice(&b).ok());

    let nthreads = std::thread::available_parallelism()
        .map(|n| n.get())
        .unwrap_or(4)
        .min(entries.len().max(1));
    let chunk = entries.len().div_ceil(nthreads);
    let coverage = CoverageConfig::default();

    let mut gated = 0usize;
    let mut missing = 0usize;
    let mut rows: Vec<Row> = Vec::new();
    let results: Vec<(Vec<Row>, usize, usize)> = std::thread::scope(|s| {
        let handles: Vec<_> = entries
            .chunks(chunk)
            .map(|slice| {
                let dir = &corpus_dir;
                let gt = &ground_truth;
                let coverage = &coverage;
                s.spawn(move || {
                    let analyzer = Analyzer::default();
                    let mut out = Vec::new();
                    let (mut gated, mut missing) = (0usize, 0usize);
                    for e in slice {
                        let Ok(data) = std::fs::read(dir.join(&e.file)) else {
                            missing += 1;
                            continue;
                        };
                        let Ok(decoded) = BoxcarsParser::new().parse(&data) else {
                            continue;
                        };
                        let stem = Path::new(&e.file)
                            .file_stem()
                            .and_then(|x| x.to_str())
                            .unwrap_or("replay");
                        let canonical = build_canonical(&decoded, stem);
                        if canonical.team_size != Some(2) {
                            continue;
                        }
                        // The same lobby-completeness gate the rank series used.
                        if !lobby_fully_present(
                            &canonical.tracks,
                            &canonical.events,
                            canonical.duration_s,
                            coverage,
                        ) {
                            gated += 1;
                            continue;
                        }

                        let timeline = timeline_from_canonical(&canonical);
                        if !team_sizes(&timeline).is_two_v_two() {
                            continue;
                        }
                        let players = roster(&timeline);
                        let joinable: Vec<(String, Option<i32>)> = players
                            .iter()
                            .map(|p| {
                                let team = match p.team {
                                    Team::Blue => 0,
                                    Team::Orange => 1,
                                };
                                (p.name.clone().unwrap_or_default(), Some(team))
                            })
                            .collect();
                        let gt_players: Option<Vec<(String, i32)>> =
                            gt.as_ref().and_then(|g| g.replays.get(&e.id)).map(|rep| {
                                rep.players
                                    .iter()
                                    .map(|p| (p.name.clone(), p.team))
                                    .collect()
                            });
                        let tiers = join_ranks(&joinable, &e.ranks, gt_players.as_deref());

                        let ctx = MatchContext::derive(&timeline, analyzer.context_config());
                        for (entry, tier) in players.iter().zip(&tiers) {
                            let Some(tier) = tier else { continue };
                            let score = analyzer.score_player_in(&ctx, entry.player);
                            let Some(value) = score.value else { continue };
                            out.push(Row {
                                bucket: e.bucket.clone(),
                                tier: *tier as f32,
                                value: value.get(),
                                dimensions: score
                                    .breakdown
                                    .iter()
                                    .map(|d| (d.dimension, d.value.get(), d.confidence.get()))
                                    .collect(),
                            });
                        }
                    }
                    (out, gated, missing)
                })
            })
            .collect();
        handles.into_iter().map(|h| h.join().unwrap()).collect()
    });
    for (out, g, m) in results {
        rows.extend(out);
        gated += g;
        missing += m;
    }

    eprintln!(
        "scored {} rank-joined players ({} replays gated by lobby completeness, {} files missing)\n",
        rows.len(),
        gated,
        missing
    );

    // Headline: does the Pacifist score track rank?
    let values: Vec<f32> = rows.iter().map(|r| r.value).collect();
    let tiers: Vec<f32> = rows.iter().map(|r| r.tier).collect();
    eprintln!("=== does the Pacifist score track rank? (Spearman) ===");
    eprintln!(
        "  pacifist value vs tier : {}",
        show(spearman(&values, &tiers))
    );

    // Per-dimension rank correlation (players where the dimension applied).
    eprintln!("\nper-dimension Spearman(value, rank)   n");
    for dim in [
        DimensionId::OverExtension,
        DimensionId::CommitmentDiscipline,
        DimensionId::BoostEconomy,
    ] {
        let (mut xs, mut ys) = (Vec::new(), Vec::new());
        for r in &rows {
            if let Some((_, v, c)) = r.dimensions.iter().find(|(d, ..)| *d == dim) {
                if *c > 0.0 {
                    xs.push(*v);
                    ys.push(r.tier);
                }
            }
        }
        eprintln!(
            "  {:<24} {:>7}   {:>5}",
            dim.label(),
            show(spearman(&xs, &ys)),
            xs.len()
        );
    }

    // Per-bucket means + within-bucket Spearman.
    eprintln!("\nper-bucket   n     mean   p50    within-bucket rho(value, tier)");
    for b in BUCKETS {
        let sub: Vec<&Row> = rows.iter().filter(|r| r.bucket == b).collect();
        if sub.is_empty() {
            continue;
        }
        let mut vals: Vec<f32> = sub.iter().map(|r| r.value).collect();
        vals.sort_by(f32::total_cmp);
        let mean = vals.iter().sum::<f32>() / vals.len() as f32;
        let p50 = vals[vals.len() / 2];
        let within = spearman(
            &sub.iter().map(|r| r.value).collect::<Vec<_>>(),
            &sub.iter().map(|r| r.tier).collect::<Vec<_>>(),
        );
        eprintln!(
            "  {:<16} {:<5} {:>5.1}  {:>5.1}   {}",
            b,
            sub.len(),
            mean,
            p50,
            show(within)
        );
    }
    Ok(())
}

fn show(o: Option<f32>) -> String {
    o.map(|v| format!("{v:+.3}"))
        .unwrap_or_else(|| "  n/a".into())
}
