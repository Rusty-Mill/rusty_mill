//! Cross-track reconciliation harness: the rule-based rubric vs the
//! outcome-grounded value model, over the labeled corpus.
//!
//! For every player it joins three numbers computed on the *same* match — the
//! rubric raws/composite (this crate), the player's ΔV (the `replay-value`
//! model loaded from `value_model.json`), and their rank tier (the manifest) —
//! then reports how the rubric agrees with each independent ground truth. See
//! [`replay_scoring::reconcile`] for what the correlations mean.
//!
//! Usage: `reconcile [manifest.json] [--gate] [--promote] [--min-rho <ρ>]`
//! (default manifest `assets/corpus/manifest.json`). Reads `fitted_config.json`
//! and `value_model.json` from the manifest's dir. With `--gate` it exits
//! non-zero on any rank-vs-impact **sign disagreement** — a regression gate that
//! catches a future metric change that tracks the ranked cohort but not in-match
//! value, mirroring `validate --gate`.
//!
//! With `--promote` it graduates experimental **candidate** metrics that the
//! value model vindicates ([`replay_scoring::reconcile::promote_candidates`]): any
//! candidate whose ΔV correlation clears `--min-rho` (default 0.10) and matches
//! its curve's good-direction has its `experimental` flag cleared and its weight
//! set to ρ², then the updated config is written back to `fitted_config.json`.
//! This is the impact-driven counterpart to `calibrate`'s rank fit; run it after
//! `calibrate` (which writes the rank-fitted config this reads).

use std::collections::BTreeMap;
use std::error::Error;
use std::path::{Path, PathBuf};
use std::process::ExitCode;

use replay_analyzer::analyze::build_canonical;
use replay_analyzer::decode::boxcars_adapter::BoxcarsParser;
use replay_analyzer::decode::ReplayParser;
use replay_scoring::calibrate::raws_from_report;
use replay_scoring::config::ScoreConfig;
use replay_scoring::reconcile::{promote_candidates, reconcile, CrossSample};
use replay_scoring::score_all;
use replay_value::{per_player_delta_v, ValueConfig, ValuePredictor};
use serde::Deserialize;

#[derive(Deserialize)]
struct ManifestEntry {
    file: String,
    #[serde(default)]
    ranks: BTreeMap<String, i32>,
}

fn show(o: Option<f32>) -> String {
    o.map(|v| format!("{v:+.3}"))
        .unwrap_or_else(|| "  n/a".into())
}

fn main() -> ExitCode {
    match run() {
        Ok(true) => ExitCode::SUCCESS,
        Ok(false) => ExitCode::FAILURE,
        Err(e) => {
            eprintln!("error: {e}");
            ExitCode::FAILURE
        }
    }
}

/// Returns `Ok(false)` when `--gate` is set and a sign disagreement is found.
fn run() -> Result<bool, Box<dyn Error>> {
    let mut gate = false;
    let mut promote = false;
    let mut min_rho = 0.10f32;
    let mut manifest_arg: Option<String> = None;
    let mut args = std::env::args().skip(1);
    while let Some(a) = args.next() {
        match a.as_str() {
            "--gate" => gate = true,
            "--promote" => promote = true,
            "--min-rho" => {
                min_rho = args
                    .next()
                    .ok_or("--min-rho needs a value")?
                    .parse()
                    .map_err(|_| "--min-rho needs a number")?;
            }
            other if other.starts_with('-') => return Err(format!("unknown flag {other}").into()),
            other => manifest_arg = Some(other.to_string()),
        }
    }
    let manifest_path =
        PathBuf::from(manifest_arg.unwrap_or_else(|| "assets/corpus/manifest.json".into()));
    let dir = manifest_path
        .parent()
        .unwrap_or(Path::new("."))
        .to_path_buf();
    let entries: Vec<ManifestEntry> = serde_json::from_slice(&std::fs::read(&manifest_path)?)?;

    // Fitted rubric (fall back to defaults) + the corpus-trained value model.
    let cfg: ScoreConfig = match std::fs::read(dir.join("fitted_config.json")) {
        Ok(bytes) => serde_json::from_slice(&bytes)?,
        Err(_) => {
            eprintln!("no fitted_config.json; using default ScoreConfig");
            ScoreConfig::default()
        }
    };
    let model: ValuePredictor =
        serde_json::from_slice(&std::fs::read(dir.join("value_model.json"))?)?;
    let vcfg = ValueConfig::default();
    eprintln!(
        "corpus: {} replays   rubric={}   value model n_train={}",
        entries.len(),
        cfg.version,
        model.n_train()
    );

    let nthreads = std::thread::available_parallelism()
        .map(|n| n.get())
        .unwrap_or(4)
        .min(entries.len().max(1));
    let chunk = entries.len().div_ceil(nthreads);

    let samples: Vec<CrossSample> = std::thread::scope(|s| {
        let handles: Vec<_> = entries
            .chunks(chunk)
            .map(|slice| {
                let (cfg, model, vcfg, dir) = (&cfg, &model, &vcfg, &dir);
                s.spawn(move || {
                    let mut out = Vec::new();
                    for e in slice {
                        let Ok(data) = std::fs::read(dir.join(&e.file)) else {
                            continue;
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
                        let m = build_canonical(&decoded, stem);

                        // ΔV per pri from the corpus value model.
                        let dv: BTreeMap<i32, (f32, f32)> = per_player_delta_v(&m, model, vcfg)
                            .into_iter()
                            .map(|p| (p.pri, (p.sum_dv, p.mean_dv)))
                            .collect();

                        for r in score_all(&m, cfg) {
                            // Require presence in *both* tracks: drop spurious
                            // unnamed tracks and players with no credited ΔV.
                            if r.target_player == "<unknown>" {
                                continue;
                            }
                            let Some(&(dv_sum, dv_mean)) = dv.get(&r.target_pri) else {
                                continue;
                            };
                            out.push(CrossSample {
                                rank: e.ranks.get(&r.target_player).map(|t| *t as f32),
                                composite: r.composite,
                                raws: raws_from_report(cfg, &r),
                                dv_sum,
                                dv_mean,
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

    let rec = reconcile(&cfg, &samples);
    eprintln!("joined {} players ({} with a rank)\n", rec.n, rec.n_ranked);

    println!("=== do the two ground truths agree? Spearman(rank, ΔV) ===");
    println!("  rank vs ΔV_sum   : {}", show(rec.rho_rank_dv_sum));
    println!("  rank vs ΔV_mean  : {}", show(rec.rho_rank_dv_mean));

    println!("\n=== rubric report cards (Spearman) ===");
    println!("  composite vs rank    : {}", show(rec.rho_composite_rank));
    println!(
        "  composite vs ΔV_sum  : {}",
        show(rec.rho_composite_dv_sum)
    );
    println!(
        "  composite vs ΔV_mean : {}",
        show(rec.rho_composite_dv_mean)
    );

    println!("\n=== per-metric: raw signal vs each ground truth ===");
    println!(
        "  {:<26} {:>7}  {:>9} {:>9}  agree",
        "metric", "[dir]", "rho_rank", "rho_dV"
    );
    for m in &rec.metrics {
        let agree = match m.agrees() {
            Some(true) => "yes",
            Some(false) => "NO",
            None => "-",
        };
        println!(
            "  {:<26} {:>7}  {:>9} {:>9}  {}",
            m.metric.key(),
            format!("[{}]", m.good_dir),
            show(m.rho_rank),
            show(m.rho_value),
            agree
        );
    }

    let dis: Vec<_> = rec.disagreements().collect();
    if dis.is_empty() {
        println!(
            "\nNo sign disagreements: every signal that carries rank also carries in-match value."
        );
    } else {
        println!("\nSign disagreements (track ranked cohort but not in-match impact):");
        for m in &dis {
            println!("  - {}", m.metric.key());
        }
    }

    // Impact-driven promotion of experimental candidate metrics.
    if promote {
        let candidates: Vec<_> = cfg.metrics.iter().filter(|s| s.experimental).collect();
        if candidates.is_empty() {
            println!("\n--promote: no experimental candidate metrics in the config.");
        } else {
            let promoted = promote_candidates(&cfg, &rec, min_rho);
            println!("\n=== candidate promotion (min |ρ_ΔV| = {min_rho:.2}) ===");
            for spec in &candidates {
                let ma = rec.metrics.iter().find(|m| m.metric == spec.metric);
                let rv = ma.and_then(|m| m.rho_value);
                let dir = ma.map(|m| m.good_dir).unwrap_or("?");
                let graduated = promoted
                    .metrics
                    .iter()
                    .find(|s| s.metric == spec.metric)
                    .map(|s| !s.experimental)
                    .unwrap_or(false);
                let verdict = if graduated {
                    format!("PROMOTED  weight={:.3}", rv.map(|r| r * r).unwrap_or(0.0))
                } else if rv.map(|r| r.abs() < min_rho).unwrap_or(true) {
                    "kept (no ΔV signal)".to_string()
                } else {
                    format!("kept (wrong sign for [{dir}])")
                };
                println!(
                    "  {:<26} ρ_ΔV={:>7}  [{}]  -> {}",
                    spec.metric.key(),
                    show(rv),
                    dir,
                    verdict
                );
            }
            let n_promoted =
                candidates.len() - promoted.metrics.iter().filter(|s| s.experimental).count();
            let mut out = promoted;
            if n_promoted > 0 && !out.version.ends_with("+promoted") {
                out.version = format!("{}+promoted", out.version);
            }
            let path = dir.join("fitted_config.json");
            std::fs::write(&path, serde_json::to_vec_pretty(&out)?)?;
            println!(
                "\npromoted {n_promoted} candidate(s); wrote {} (version {})",
                path.display(),
                out.version
            );
        }
    }

    if gate && !dis.is_empty() {
        eprintln!(
            "\nGATE FAIL: {} metric(s) track the ranked cohort but disagree on in-match value.",
            dis.len()
        );
        return Ok(false);
    }
    Ok(true)
}
