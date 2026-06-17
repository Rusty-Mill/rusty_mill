//! Corpus calibration harness.
//!
//! Reads a labeled corpus manifest (`{file, bucket, ranks:{name:tier}}` per
//! replay), scores every player, and joins each player's composite to their own
//! rank tier. Reports:
//!   * Spearman(composite, rank) — the make-or-break "does it track skill" check;
//!   * per-metric Spearman(raw, rank) — which signals carry rank, and whether any
//!     are inverted versus their assumed good-direction;
//!   * per-rank composite means (ordering);
//!   * Spearman after refitting every curve to the corpus distribution, and the
//!     fitted [`ScoreConfig`] written out for inspection/adoption.
//!
//! Usage: `replay-value`-style — `calibrate [manifest.json]`
//! (default `assets/corpus/manifest.json`). Parsing is parallel over cores.

use std::collections::BTreeMap;
use std::error::Error;
use std::path::{Path, PathBuf};

use replay_analyzer::analyze::build_canonical;
use replay_analyzer::analyze::validate::GroundTruth;
use replay_analyzer::decode::boxcars_adapter::BoxcarsParser;
use replay_analyzer::decode::ReplayParser;
use replay_scoring::calibrate::{
    composite_with, fit_tiers, fit_weights, fit_weights_ridge, join_ranks, raws_from_report,
    refit_config, spearman,
};
use replay_scoring::config::{Curve, Metric, ScoreConfig};
use replay_scoring::{score_all, Confidence};
use serde::Deserialize;

#[derive(Deserialize)]
struct ManifestEntry {
    /// ballchasing replay id — the key into the ground-truth stats fixture.
    #[serde(default)]
    id: String,
    file: String,
    bucket: String,
    #[serde(default)]
    ranks: BTreeMap<String, i32>,
    /// Playlist slug (backfilled by `refresh_manifest_playlist.py`). The corpus is
    /// **ranked doubles only** — anything else is excluded from calibration.
    #[serde(default)]
    playlist: Option<String>,
    /// Team size from the replay header; must be 2 for this 2v2 corpus.
    #[serde(default)]
    team_size: Option<i32>,
}

/// The only playlist the calibration corpus admits (see `manifest.json`).
const RANKED_DOUBLES: &str = "ranked-doubles";

struct Sample {
    tier: f32,
    bucket: String,
    composite_before: f32,
    confident: bool,
    raws: BTreeMap<Metric, Option<f32>>,
}

fn mean(v: &[f32]) -> f32 {
    if v.is_empty() {
        0.0
    } else {
        v.iter().sum::<f32>() / v.len() as f32
    }
}

fn show(o: Option<f32>) -> String {
    o.map(|v| format!("{v:+.3}"))
        .unwrap_or_else(|| "  n/a".into())
}

/// k-fold cross-validated Spearman(composite, rank): fit curves (and optionally
/// weights) on the train folds, score the held-out fold, pool all held-out
/// predictions. This is the *honest* number — no metric sees its own test row.
fn cv_rho(samples: &[Sample], base: &ScoreConfig, k: usize, fit: u8) -> Option<f32> {
    let (mut comps, mut tiers) = (Vec::new(), Vec::new());
    for fold in 0..k {
        let mut pooled: BTreeMap<Metric, Vec<f32>> = BTreeMap::new();
        for (i, s) in samples.iter().enumerate() {
            if i % k == fold {
                continue; // held out
            }
            for (m, raw) in &s.raws {
                if let Some(v) = raw {
                    pooled.entry(*m).or_default().push(*v);
                }
            }
        }
        let mut cfg = refit_config(base, &pooled);
        if fit > 0 {
            let train: Vec<_> = samples
                .iter()
                .enumerate()
                .filter(|(i, _)| i % k != fold)
                .map(|(_, s)| (s.raws.clone(), s.tier))
                .collect();
            cfg = match fit {
                2 => fit_weights_ridge(&cfg, &train, 5.0),
                _ => fit_weights(&cfg, &train),
            };
        }
        for (i, s) in samples.iter().enumerate() {
            if i % k == fold {
                comps.push(composite_with(&cfg, &s.raws).0);
                tiers.push(s.tier);
            }
        }
    }
    spearman(&comps, &tiers)
}

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
    let all_entries: Vec<ManifestEntry> = serde_json::from_slice(&std::fs::read(&manifest_path)?)?;
    // Train only on ranked 2v2 — exclude (loudly) anything the manifest doesn't
    // tag as ranked-doubles / team_size 2 so a stray refresh can't pollute the fit.
    let total = all_entries.len();
    let entries: Vec<ManifestEntry> = all_entries
        .into_iter()
        .filter(|e| {
            let ok = e.team_size == Some(2) && e.playlist.as_deref() == Some(RANKED_DOUBLES);
            if !ok {
                eprintln!(
                    "skip {}: not ranked 2v2 (playlist={:?}, team_size={:?})",
                    e.id, e.playlist, e.team_size
                );
            }
            ok
        })
        .collect();
    eprintln!(
        "corpus: {}/{} replays are ranked-doubles 2v2 from {}",
        entries.len(),
        total,
        manifest_path.display()
    );
    if entries.is_empty() {
        return Err("no ranked-doubles 2v2 replays in manifest (run \
             assets/corpus/refresh_manifest_playlist.py to backfill playlist/team_size)"
            .into());
    }

    // Ground-truth stats give each ballchasing-mangled rank key its team, which is
    // what lets the rank join team-anchor a mangled name to the analyzer's true
    // name (recovering players an exact-name lookup drops). Optional: absent → the
    // join degrades to exact-name matching.
    let ground_truth: Option<GroundTruth> =
        std::fs::read(corpus_dir.join("ballchasing_stats.json"))
            .ok()
            .and_then(|b| serde_json::from_slice(&b).ok());
    if ground_truth.is_none() {
        eprintln!("note: no ballchasing_stats.json — rank join uses exact names only");
    }

    let cfg = ScoreConfig::default();
    let nthreads = std::thread::available_parallelism()
        .map(|n| n.get())
        .unwrap_or(4)
        .min(entries.len().max(1));
    let chunk = entries.len().div_ceil(nthreads);

    // Parse + score the corpus in parallel; collect per-player samples.
    let samples: Vec<Sample> = std::thread::scope(|s| {
        let handles: Vec<_> = entries
            .chunks(chunk)
            .map(|slice| {
                let cfg = &cfg;
                let dir = &corpus_dir;
                let gt = &ground_truth;
                s.spawn(move || {
                    let mut out = Vec::new();
                    for e in slice {
                        let path = dir.join(&e.file);
                        let data = match std::fs::read(&path) {
                            Ok(d) => d,
                            Err(err) => {
                                eprintln!("read {}: {err}", path.display());
                                continue;
                            }
                        };
                        let decoded = match BoxcarsParser::new().parse(&data) {
                            Ok(d) => d,
                            Err(err) => {
                                eprintln!("decode {}: {err}", e.file);
                                continue;
                            }
                        };
                        let stem = Path::new(&e.file)
                            .file_stem()
                            .and_then(|x| x.to_str())
                            .unwrap_or("replay");
                        let canonical = build_canonical(&decoded, stem);
                        // Defense-in-depth: trust the decoded header over the
                        // manifest tag — drop anything that isn't actually 2v2.
                        if canonical.team_size != Some(2) {
                            eprintln!(
                                "skip {}: decoded team_size {:?} != 2",
                                e.file, canonical.team_size
                            );
                            continue;
                        }
                        let reports = score_all(&canonical, cfg);
                        // Team-anchored rank join: a mangled ballchasing name still
                        // resolves to the analyzer's player (vs. dropping it).
                        let players: Vec<(String, Option<i32>)> = reports
                            .iter()
                            .map(|r| (r.target_player.clone(), r.target_team))
                            .collect();
                        let gt_players: Option<Vec<(String, i32)>> =
                            gt.as_ref().and_then(|g| g.replays.get(&e.id)).map(|rep| {
                                rep.players
                                    .iter()
                                    .map(|p| (p.name.clone(), p.team))
                                    .collect()
                            });
                        let tiers = join_ranks(&players, &e.ranks, gt_players.as_deref());
                        for (r, tier) in reports.iter().zip(&tiers) {
                            let Some(tier) = tier else { continue };
                            out.push(Sample {
                                tier: *tier as f32,
                                bucket: e.bucket.clone(),
                                composite_before: r.composite,
                                confident: r.confidence == Confidence::Ok,
                                raws: raws_from_report(cfg, r),
                            });
                        }
                    }
                    out
                })
            })
            .collect();
        handles
            .into_iter()
            .flat_map(|h| h.join().unwrap())
            .collect()
    });

    let confident: Vec<&Sample> = samples.iter().filter(|s| s.confident).collect();
    eprintln!(
        "scored {} players ({} confident / full 2v2)\n",
        samples.len(),
        confident.len()
    );

    let tiers: Vec<f32> = samples.iter().map(|s| s.tier).collect();
    let comp_before: Vec<f32> = samples.iter().map(|s| s.composite_before).collect();
    let ct: Vec<f32> = confident.iter().map(|s| s.tier).collect();
    let cc: Vec<f32> = confident.iter().map(|s| s.composite_before).collect();

    // Per-metric rank correlation: does each signal carry skill, in the right way?
    eprintln!("per-metric Spearman(raw, rank)   [good dir]   n");
    for spec in &cfg.metrics {
        let mut xs = Vec::new();
        let mut ys = Vec::new();
        for sm in &samples {
            if let Some(v) = sm.raws.get(&spec.metric).and_then(|o| *o) {
                xs.push(v);
                ys.push(sm.tier);
            }
        }
        let rho = spearman(&xs, &ys);
        let (dir, ok) = match spec.curve {
            Curve::Higher { .. } => ("higher", rho.map(|r| r >= 0.0)),
            Curve::Lower { .. } => ("lower", rho.map(|r| r <= 0.0)),
            Curve::Band { .. } => ("band", None),
        };
        let flag = match ok {
            Some(false) => "  <-- INVERTED vs assumed direction",
            _ => "",
        };
        eprintln!(
            "  {:<26} {:>7}   [{:<6}]  {:>4}{}",
            spec.metric.key(),
            show(rho),
            dir,
            xs.len(),
            flag
        );
    }

    // Refit every curve to the corpus distribution, then re-score from cached raws.
    let mut pooled: BTreeMap<Metric, Vec<f32>> = BTreeMap::new();
    for sm in &samples {
        for (m, raw) in &sm.raws {
            if let Some(v) = raw {
                pooled.entry(*m).or_default().push(*v);
            }
        }
    }
    let fitted = refit_config(&cfg, &pooled);
    let comp_after: Vec<f32> = samples
        .iter()
        .map(|s| composite_with(&fitted, &s.raws).0)
        .collect();

    // Per-rank composite means (ordering should rise with rank if it tracks skill).
    eprintln!("\nper-rank composite (mean ± over players)   default -> fitted");
    for b in [
        "bronze",
        "silver",
        "gold",
        "platinum",
        "diamond",
        "champion",
        "grand-champion",
    ] {
        let before: Vec<f32> = samples
            .iter()
            .filter(|s| s.bucket == b)
            .map(|s| s.composite_before)
            .collect();
        if before.is_empty() {
            continue;
        }
        let after: Vec<f32> = samples
            .iter()
            .zip(&comp_after)
            .filter(|(s, _)| s.bucket == b)
            .map(|(_, &c)| c)
            .collect();
        eprintln!(
            "  {:<16} n={:<3}  {:>5.1}  ->  {:>5.1}",
            b,
            before.len(),
            mean(&before),
            mean(&after)
        );
    }

    eprintln!("\n=== headline: does composite track rank? (Spearman) ===");
    eprintln!(
        "  default config,  all players      : {}",
        show(spearman(&comp_before, &tiers))
    );
    eprintln!(
        "  default config,  confident only   : {}",
        show(spearman(&cc, &ct))
    );
    eprintln!(
        "  refitted curves, all players      : {}",
        show(spearman(&comp_after, &tiers))
    );
    eprintln!("  --- 5-fold cross-validated (honest, held-out) ---");
    eprintln!(
        "  refit curves              (CV)    : {}",
        show(cv_rho(&samples, &cfg, 5, 0))
    );
    eprintln!(
        "  refit curves + ρ²-weights (CV)    : {}",
        show(cv_rho(&samples, &cfg, 5, 1))
    );
    eprintln!(
        "  refit curves + ridge-wts  (CV)    : {}",
        show(cv_rho(&samples, &cfg, 5, 2))
    );

    // The adoptable config: curves refit + ridge weights fit on the full corpus
    // (ridge generalized marginally better than ρ² in CV, and shares collinear
    // weight correctly).
    let all: Vec<_> = samples.iter().map(|s| (s.raws.clone(), s.tier)).collect();
    let mut final_cfg = fit_weights_ridge(&fitted, &all, 5.0);
    final_cfg.version = format!("{}-fitted", final_cfg.version);
    let comps_fit: Vec<f32> = samples
        .iter()
        .map(|s| composite_with(&final_cfg, &s.raws).0)
        .collect();
    final_cfg.tiers = fit_tiers(&final_cfg, &comps_fit);
    let out_path = corpus_dir.join("fitted_config.json");
    std::fs::write(&out_path, serde_json::to_vec_pretty(&final_cfg)?)?;
    eprintln!(
        "\nwrote fitted config (curves+weights) -> {}",
        out_path.display()
    );
    Ok(())
}
