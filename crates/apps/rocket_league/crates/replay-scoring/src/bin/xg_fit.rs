//! Fit the xG model on the calibration corpus: `xg-fit [manifest.json]`
//! (default `assets/corpus/manifest.json`; the replays sit next to it).
//!
//! Every ranked-doubles replay's shots become labelled examples (goal = 1). Whole
//! matches are held out (every fifth), so the reported Brier score and reliability
//! are out-of-sample; the model is then refit on everything and written to
//! `xg_model.json` beside the manifest, where the app picks it up.

use std::error::Error;
use std::path::{Path, PathBuf};

use replay_analyzer::analyze::build_canonical;
use replay_analyzer::decode::boxcars_adapter::BoxcarsParser;
use replay_analyzer::decode::ReplayParser;
use replay_scoring::xg::{sample, N};
use replay_scoring::{extract, ScoreConfig, XgModel};
use serde::Deserialize;

type Sample = ([f32; N], bool);
const RIDGE: f64 = 1.0;
const HOLDOUT_EVERY: usize = 5;

#[derive(Deserialize)]
struct Entry {
    file: String,
    #[serde(default)]
    playlist: Option<String>,
    #[serde(default)]
    team_size: Option<i32>,
}

/// One replay's labelled shots, or why it was skipped.
fn shots_of(path: &Path, cfg: &ScoreConfig) -> Result<Vec<Sample>, Box<dyn Error>> {
    let data = std::fs::read(path)?;
    let decoded = BoxcarsParser::new().parse(&data)?;
    let m = build_canonical(&decoded, "xg");
    Ok(extract(&m, cfg).iter().filter_map(sample).collect())
}

fn base_brier(data: &[Sample]) -> f32 {
    let rate = data.iter().filter(|s| s.1).count() as f32 / data.len().max(1) as f32;
    data.iter().map(|s| (rate - f32::from(s.1)).powi(2)).sum::<f32>() / data.len().max(1) as f32
}

fn main() -> Result<(), Box<dyn Error>> {
    let manifest = PathBuf::from(
        std::env::args().nth(1).unwrap_or_else(|| "assets/corpus/manifest.json".into()),
    );
    let dir = manifest.parent().unwrap_or(Path::new(".")).to_path_buf();
    let entries: Vec<Entry> = serde_json::from_slice(&std::fs::read(&manifest)?)?;
    let entries: Vec<&Entry> = entries
        .iter()
        .filter(|e| e.team_size == Some(2) && e.playlist.as_deref() == Some("ranked-doubles"))
        .collect();
    let cfg = ScoreConfig::default();

    let (mut train, mut test): (Vec<Sample>, Vec<Sample>) = (vec![], vec![]);
    for (i, e) in entries.iter().enumerate() {
        match shots_of(&dir.join(&e.file), &cfg) {
            Ok(s) if i % HOLDOUT_EVERY == 0 => test.extend(s),
            Ok(s) => train.extend(s),
            Err(err) => eprintln!("skip {}: {err}", e.file),
        }
    }
    eprintln!("{} replays → {} train / {} held-out shots", entries.len(), train.len(), test.len());

    let prior = XgModel::default();
    let fit = XgModel::fit(&train, RIDGE).ok_or("too few shots to fit (are the corpus replays on disk? see assets/corpus/README.md)")?;
    println!("held-out Brier: base rate {:.4} | prior {:.4} | fitted {:.4}", base_brier(&test), prior.brier(&test), fit.brier(&test));
    println!("reliability on held-out shots (mean predicted → observed, n):");
    for (p, o, n) in fit.reliability(&test) {
        println!("  {p:.2} → {o:.2}  ({n})");
    }
    println!("weights [bias, dist, speed, on_target, defenders, edge]: {:?}", fit.w);

    // Ship the fit on everything once it has been validated out of sample.
    let all: Vec<Sample> = train.into_iter().chain(test).collect();
    let final_model = XgModel::fit(&all, RIDGE).ok_or("refit failed")?;
    let out = dir.join("xg_model.json");
    std::fs::write(&out, serde_json::to_vec_pretty(&final_model)?)?;
    println!("wrote {} ({})", out.display(), final_model.version);
    Ok(())
}
