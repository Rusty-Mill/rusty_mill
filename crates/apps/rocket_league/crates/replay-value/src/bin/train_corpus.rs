//! Train the value model on a whole replay corpus, with a **replay-level**
//! train/val split (rows from one match are correlated, so splitting by row
//! would leak). Reports held-out log-loss and AUC against the base-rate baseline
//! — the honest "does the value model generalize" check, the per-match overfit's
//! antidote.
//!
//! Usage: `train_corpus [manifest.json]` (default `assets/corpus/manifest.json`).
//! Labels are self-supervised from goals, so the manifest's ranks are unused.

use std::error::Error;
use std::path::{Path, PathBuf};

use replay_analyzer::analyze::build_canonical;
use replay_analyzer::decode::boxcars_adapter::BoxcarsParser;
use replay_analyzer::decode::ReplayParser;
use replay_value::config::TrainConfig;
use replay_value::dataset::{Dataset, Row};
use replay_value::features::N_FEATURES;
use replay_value::model::ValueModel;
use replay_value::{build_dataset, GbtConfig, GbtModel, ValueConfig, ValuePredictor};
use serde::Deserialize;

#[derive(Deserialize)]
struct ManifestEntry {
    #[serde(default)]
    id: String,
    file: String,
    /// Playlist slug (backfilled by `refresh_manifest_playlist.py`). Train only
    /// on ranked doubles.
    #[serde(default)]
    playlist: Option<String>,
    /// Team size from the replay header; must be 2 for this 2v2 corpus.
    #[serde(default)]
    team_size: Option<i32>,
}

/// The only playlist the corpus admits (see `manifest.json`).
const RANKED_DOUBLES: &str = "ranked-doubles";

fn base_rate_log_loss(rows: &[Row], p: f32) -> f32 {
    if rows.is_empty() {
        return 0.0;
    }
    let p = p.clamp(1e-7, 1.0 - 1e-7);
    let s: f32 = rows
        .iter()
        .map(|r| -(r.y * p.ln() + (1.0 - r.y) * (1.0 - p).ln()))
        .sum();
    s / rows.len() as f32
}

/// Mean binary cross-entropy of a prediction fn over rows (held-out diagnostic).
fn log_loss(predict: impl Fn(&[f32; N_FEATURES]) -> f32, rows: &[Row]) -> f32 {
    if rows.is_empty() {
        return 0.0;
    }
    let eps = 1e-7;
    let s: f32 = rows
        .iter()
        .map(|r| {
            let p = predict(&r.x).clamp(eps, 1.0 - eps);
            -(r.y * p.ln() + (1.0 - r.y) * (1.0 - p).ln())
        })
        .sum();
    s / rows.len() as f32
}

/// Mann–Whitney AUC: P(model scores a random positive above a random negative).
fn auc(predict: impl Fn(&[f32; N_FEATURES]) -> f32, rows: &[Row]) -> f32 {
    let mut scored: Vec<(f32, f32)> = rows.iter().map(|r| (predict(&r.x), r.y)).collect();
    scored.sort_by(|a, b| a.0.total_cmp(&b.0));
    let (mut rank_sum, mut npos, mut i) = (0.0f64, 0usize, 0usize);
    // Average ranks for ties, 1-based.
    while i < scored.len() {
        let mut j = i;
        while j + 1 < scored.len() && scored[j + 1].0 == scored[i].0 {
            j += 1;
        }
        let avg_rank = (i + j) as f64 / 2.0 + 1.0;
        for s in &scored[i..=j] {
            if s.1 > 0.5 {
                rank_sum += avg_rank;
                npos += 1;
            }
        }
        i = j + 1;
    }
    let nneg = scored.len() - npos;
    if npos == 0 || nneg == 0 {
        return 0.5;
    }
    ((rank_sum - (npos * (npos + 1)) as f64 / 2.0) / (npos as f64 * nneg as f64)) as f32
}

fn main() -> Result<(), Box<dyn Error>> {
    let manifest = PathBuf::from(
        std::env::args()
            .nth(1)
            .unwrap_or_else(|| "assets/corpus/manifest.json".into()),
    );
    let dir = manifest.parent().unwrap_or(Path::new(".")).to_path_buf();
    let all_entries: Vec<ManifestEntry> = serde_json::from_slice(&std::fs::read(&manifest)?)?;
    // Train only on ranked 2v2 — exclude (loudly) anything else so a stray
    // refresh can't pollute the value model.
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
        "corpus: {}/{} replays are ranked-doubles 2v2",
        entries.len(),
        total
    );
    if entries.is_empty() {
        return Err("no ranked-doubles 2v2 replays in manifest (run \
             assets/corpus/refresh_manifest_playlist.py to backfill playlist/team_size)"
            .into());
    }

    let cfg = ValueConfig {
        sample_stride: 30, // 1 s; plenty of states across a corpus, keeps it fast
        train: TrainConfig {
            epochs: 2500,
            ..TrainConfig::default()
        },
        ..ValueConfig::default()
    };

    let nthreads = std::thread::available_parallelism()
        .map(|n| n.get())
        .unwrap_or(4)
        .min(entries.len().max(1));
    let chunk = entries.len().div_ceil(nthreads);

    // Parse + build each replay's rows in parallel, preserving replay grouping.
    let per_replay: Vec<Vec<Row>> = std::thread::scope(|s| {
        let handles: Vec<_> = entries
            .chunks(chunk)
            .map(|slice| {
                let dir = &dir;
                let cfg = &cfg;
                s.spawn(move || {
                    let mut out = Vec::new();
                    for e in slice {
                        let Ok(data) = std::fs::read(dir.join(&e.file)) else {
                            continue;
                        };
                        let Ok(decoded) = BoxcarsParser::new().parse(&data) else {
                            continue;
                        };
                        let m = build_canonical(&decoded, "x");
                        // Defense-in-depth: drop anything not actually 2v2.
                        if m.team_size != Some(2) {
                            eprintln!("skip {}: decoded team_size {:?} != 2", e.file, m.team_size);
                            continue;
                        }
                        out.push(build_dataset(&m, cfg).rows);
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

    // Replay-level split: every 5th replay is held out.
    let (mut train, mut val) = (Vec::new(), Vec::new());
    for (i, rows) in per_replay.iter().enumerate() {
        let dst = if i % 5 == 0 { &mut val } else { &mut train };
        dst.extend(rows.iter().cloned());
    }
    let train_ds = Dataset {
        rows: train,
        horizon_s: cfg.horizon_s,
    };
    let val_ds = Dataset {
        rows: val,
        horizon_s: cfg.horizon_s,
    };

    let base = if train_ds.rows.is_empty() {
        0.0
    } else {
        train_ds.rows.iter().map(|r| r.y).sum::<f32>() / train_ds.rows.len() as f32
    };
    let model = ValueModel::train(&train_ds, &cfg.train);

    eprintln!(
        "corpus value model (horizon={:.0}s, stride={}):",
        cfg.horizon_s, cfg.sample_stride
    );
    eprintln!(
        "  replays={}  train_rows={}  val_rows={}  base_rate={:.3}",
        per_replay.len(),
        train_ds.rows.len(),
        val_ds.rows.len(),
        base
    );
    eprintln!(
        "  train log_loss = {:.4}   (base {:.4})",
        model.log_loss(&train_ds),
        base_rate_log_loss(&train_ds.rows, base)
    );

    // Head-to-head on the SAME held-out split: logistic vs gradient-boosted trees.
    // The GBT clearly wins (VAL AUC ~0.715 → ~0.741), so it's the shipped model.
    // The production config is more regularized than the library default (more
    // corpus data warrants it): besides matching the default's held-out AUC, the
    // smoother per-touch ΔV surface keeps `reconcile` green (the default GBT flipped
    // the near-zero `first_touch_value` sign and tripped that gate).
    let gbt_cfg = GbtConfig {
        rounds: 100,
        max_depth: 3,
        learning_rate: 0.1,
        min_leaf: 250,
        lambda: 3.0,
    };
    let gbt = GbtModel::train(&train_ds, &gbt_cfg);
    let base_ll = base_rate_log_loss(&val_ds.rows, base);
    eprintln!("  VAL base-rate log_loss = {base_ll:.4}   [held-out: model < base ⇒ generalizes]");
    eprintln!("  {:<10} {:>10} {:>10}", "model", "VAL_logloss", "VAL_AUC");
    eprintln!(
        "  {:<10} {:>10.4} {:>10.4}",
        "logistic",
        log_loss(|x| model.predict(x), &val_ds.rows),
        auc(|x| model.predict(x), &val_ds.rows),
    );
    eprintln!(
        "  {:<10} {:>10.4} {:>10.4}   (rounds={}, depth={}, lr={}, min_leaf={}, lambda={})",
        "gbt",
        log_loss(|x| gbt.predict(x), &val_ds.rows),
        auc(|x| gbt.predict(x), &val_ds.rows),
        gbt_cfg.rounds,
        gbt_cfg.max_depth,
        gbt_cfg.learning_rate,
        gbt_cfg.min_leaf,
        gbt_cfg.lambda,
    );

    // Retrain on ALL replays for the shipped model (max data), and persist it.
    let all_rows: Vec<Row> = per_replay.iter().flat_map(|r| r.iter().cloned()).collect();
    let full = Dataset {
        rows: all_rows,
        horizon_s: cfg.horizon_s,
    };
    // Ship the gradient-boosted model — it wins the held-out head-to-head above.
    let final_model = ValuePredictor::Gbt(GbtModel::train(&full, &gbt_cfg));
    let out = dir.join("value_model.json");
    std::fs::write(&out, serde_json::to_vec_pretty(&final_model)?)?;
    eprintln!(
        "  wrote gbt model ({} rows) -> {}",
        full.rows.len(),
        out.display()
    );
    Ok(())
}
