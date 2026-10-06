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
use replay_scoring::{extract, level_of, ScoreConfig, XgModel};
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
    /// Rank bracket (`gold`, …): the lobby level the shots are fitted at.
    #[serde(default)]
    bucket: Option<String>,
}

/// One replay's labelled shots, or why it was skipped.
fn shots_of(path: &Path, level: f32, cfg: &ScoreConfig) -> Result<Vec<Sample>, Box<dyn Error>> {
    let data = std::fs::read(path)?;
    let decoded = BoxcarsParser::new().parse(&data)?;
    let m = build_canonical(&decoded, "xg");
    Ok(extract(&m, cfg)
        .iter()
        .filter_map(|e| sample(e, level))
        .collect())
}

fn base_brier(data: &[Sample]) -> f32 {
    let rate = data.iter().filter(|s| s.1).count() as f32 / data.len().max(1) as f32;
    data.iter()
        .map(|s| (rate - f32::from(s.1)).powi(2))
        .sum::<f32>()
        / data.len().max(1) as f32
}

fn main() -> Result<(), Box<dyn Error>> {
    let manifest = PathBuf::from(
        std::env::args()
            .nth(1)
            .unwrap_or_else(|| "assets/corpus/manifest.json".into()),
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
        let level = e.bucket.as_deref().and_then(level_of).unwrap_or(0.0);
        match shots_of(&dir.join(&e.file), level, &cfg) {
            Ok(s) if i % HOLDOUT_EVERY == 0 => test.extend(s),
            Ok(s) => train.extend(s),
            Err(err) => eprintln!("skip {}: {err}", e.file),
        }
    }
    eprintln!(
        "{} replays → {} train / {} held-out shots",
        entries.len(),
        train.len(),
        test.len()
    );

    let prior = XgModel::default();
    let fit = XgModel::fit(&train, RIDGE).ok_or(
        "too few shots to fit (are the corpus replays on disk? see assets/corpus/README.md)",
    )?;
    // The same fit without the rank feature, to show what it buys on held-out shots.
    let blind = |d: &[Sample]| -> Vec<Sample> {
        d.iter()
            .map(|(x, y)| {
                (
                    {
                        let mut x = *x;
                        x[N - 1] = 0.0;
                        x
                    },
                    *y,
                )
            })
            .collect()
    };
    let no_rank = XgModel::fit(&blind(&train), RIDGE).ok_or("too few shots to fit")?;
    println!(
        "held-out Brier: base rate {:.4} | prior {:.4} | fitted without rank {:.4} | fitted {:.4}",
        base_brier(&test),
        prior.brier(&test),
        no_rank.brier(&blind(&test)),
        fit.brier(&test)
    );
    println!("reliability on held-out shots (mean predicted → observed, n):");
    for (p, o, n) in fit.reliability(&test) {
        println!("  {p:.2} → {o:.2}  ({n})");
    }
    println!(
        "weights [bias, dist, speed, on_target, defenders, edge, rank]: {:?}",
        fit.w
    );

    // Ship the fit on everything once it has been validated out of sample.
    let all: Vec<Sample> = train.into_iter().chain(test).collect();
    let final_model = XgModel::fit(&all, RIDGE).ok_or("refit failed")?;
    let out = dir.join("xg_model.json");
    std::fs::write(&out, serde_json::to_vec_pretty(&final_model)?)?;
    println!("wrote {} ({})", out.display(), final_model.version);
    Ok(())
}
