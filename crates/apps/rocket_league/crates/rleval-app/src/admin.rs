//! Model/config introspection for the admin page (`/admin`).
//!
//! A read-only snapshot of the *models the workspace ships*: the decision-
//! discipline scoring rubric, the value model, the skills catalog, and the
//! calibration corpus. It reports both what the app runs **in-process** (the
//! versioned defaults) and the **calibrated artifacts on disk** (fitted configs,
//! the corpus value model) with their versions — so an operator can see at a
//! glance whether the rubric is running defaults or a corpus fit, and how big the
//! corpus is. Purely informational; nothing here mutates state.

use std::path::Path;

use replay_scoring::config::{ScoreConfig, SCORE_CONFIG_VERSION};
use replay_skills::skill::Skill;
use replay_skills::SKILL_CONFIG_VERSION;
use replay_value::features::FEATURE_NAMES;
use replay_value::{ValuePredictor, VALUE_CONFIG_VERSION};
use serde::Serialize;

/// The full admin snapshot.
#[derive(Serialize)]
pub struct ConfigReport {
    pub scoring: ScoringInfo,
    pub value: ValueInfo,
    pub skills: SkillsInfo,
    pub corpus: CorpusInfo,
    /// Whether the maintenance run endpoint is enabled (server started with
    /// `--enable-admin-run`). The UI greys out the buttons when false.
    pub run_enabled: bool,
}

#[derive(Serialize)]
pub struct ScoringInfo {
    /// The config version the app actually runs (the in-process default).
    pub active_version: String,
    /// The full default config — metric table, curves, weights, tiers, centroids.
    pub config: ScoreConfig,
    /// The corpus-fitted config on disk, if present (used by the CLI tools, not
    /// the app). Surfaces the gap between "calibrated" and "what runs here".
    pub fitted: Option<OnDisk>,
}

#[derive(Serialize)]
pub struct ValueInfo {
    pub config_version: String,
    /// The input features the model consumes, in order.
    pub feature_names: Vec<String>,
    /// How the app scores ΔV at runtime (independent of the shipped model).
    pub runtime: String,
    /// The committed corpus model on disk, if present.
    pub shipped: Option<ShippedModel>,
}

#[derive(Serialize)]
pub struct SkillsInfo {
    pub active_version: String,
    /// The mechanical-skill catalog (display names).
    pub catalog: Vec<String>,
    pub fitted: Option<OnDisk>,
}

#[derive(Serialize)]
pub struct CorpusInfo {
    pub manifest_path: String,
    pub present: bool,
    pub total: usize,
    /// `(bucket, count)` rank-tier distribution.
    pub per_bucket: Vec<(String, usize)>,
}

/// A calibrated artifact found on disk: its `version` string, path, and last-
/// modified time (seconds since the Unix epoch) so the UI can flag staleness.
#[derive(Serialize)]
pub struct OnDisk {
    pub version: String,
    pub path: String,
    pub modified_epoch: Option<u64>,
}

#[derive(Serialize)]
pub struct ShippedModel {
    pub kind: String,
    pub n_train: usize,
    pub path: String,
    pub modified_epoch: Option<u64>,
    /// Per-feature importance, index-aligned with `feature_names` (gradient-boosted
    /// split frequency); `None` for a logistic model.
    pub importance: Option<Vec<f32>>,
}

/// File mtime as whole seconds since the Unix epoch, if available.
fn mtime_epoch(path: &Path) -> Option<u64> {
    std::fs::metadata(path)
        .ok()?
        .modified()
        .ok()?
        .duration_since(std::time::UNIX_EPOCH)
        .ok()
        .map(|d| d.as_secs())
}

/// Read the `"version"` field out of a JSON config on disk (without binding to its
/// full schema), returning an [`OnDisk`] if the file parses.
fn on_disk_version(path: &Path) -> Option<OnDisk> {
    let bytes = std::fs::read(path).ok()?;
    let v: serde_json::Value = serde_json::from_slice(&bytes).ok()?;
    Some(OnDisk {
        version: v.get("version")?.as_str()?.to_string(),
        path: path.display().to_string(),
        modified_epoch: mtime_epoch(path),
    })
}

/// Build the admin snapshot, reading any calibrated artifacts from `corpus_dir`.
/// `run_enabled` reflects whether the maintenance endpoint is turned on.
pub fn config_report(corpus_dir: &Path, run_enabled: bool) -> ConfigReport {
    // Value model on disk (kind + training size).
    let model_path = corpus_dir.join("value_model.json");
    let shipped = std::fs::read(&model_path)
        .ok()
        .and_then(|b| serde_json::from_slice::<ValuePredictor>(&b).ok())
        .map(|m| ShippedModel {
            kind: match m {
                ValuePredictor::Logistic(_) => "logistic regression".into(),
                ValuePredictor::Gbt(_) => "gradient-boosted trees".into(),
            },
            n_train: m.n_train(),
            path: model_path.display().to_string(),
            modified_epoch: mtime_epoch(&model_path),
            importance: m.feature_importance().map(|imp| imp.to_vec()),
        });

    // Corpus manifest: total + per-bucket counts.
    let manifest_path = corpus_dir.join("manifest.json");
    let (present, total, per_bucket) = match std::fs::read(&manifest_path)
        .ok()
        .and_then(|b| serde_json::from_slice::<Vec<serde_json::Value>>(&b).ok())
    {
        Some(entries) => {
            let mut counts: std::collections::BTreeMap<String, usize> = Default::default();
            for e in &entries {
                if let Some(b) = e.get("bucket").and_then(|x| x.as_str()) {
                    *counts.entry(b.to_string()).or_default() += 1;
                }
            }
            (true, entries.len(), counts.into_iter().collect())
        }
        None => (false, 0, Vec::new()),
    };

    ConfigReport {
        scoring: ScoringInfo {
            active_version: SCORE_CONFIG_VERSION.to_string(),
            config: ScoreConfig::default(),
            fitted: on_disk_version(&corpus_dir.join("fitted_config.json")),
        },
        value: ValueInfo {
            config_version: VALUE_CONFIG_VERSION.to_string(),
            feature_names: FEATURE_NAMES.iter().map(|s| s.to_string()).collect(),
            runtime: "per-match logistic, trained in-process on the open replay \
                      (the shipped GBT below is used by the CLI tools)"
                .to_string(),
            shipped,
        },
        skills: SkillsInfo {
            active_version: SKILL_CONFIG_VERSION.to_string(),
            catalog: Skill::ALL.iter().map(|s| s.display_name().to_string()).collect(),
            fitted: on_disk_version(&corpus_dir.join("fitted_skill_config.json")),
        },
        corpus: CorpusInfo {
            manifest_path: manifest_path.display().to_string(),
            present,
            total,
            per_bucket,
        },
        run_enabled,
    }
}

/// Result of a whitelisted maintenance command.
#[derive(Serialize)]
pub struct RunResult {
    pub action: String,
    pub command: String,
    pub ok: bool,
    pub output: String,
}

/// Run one **whitelisted** maintenance action by invoking the cargo binaries
/// directly (cross-platform — no shell wrappers). No caller-supplied arguments
/// reach the command line; the only inputs are the fixed action key and the
/// server's corpus dir. Localhost dev convenience; the caller gates this behind
/// `--enable-admin-run`. Returns `None` for an unknown action.
pub fn run_action(action: &str, corpus_dir: &Path) -> Option<RunResult> {
    let manifest = corpus_dir.join("manifest.json").to_string_lossy().to_string();
    // (cargo package, args) steps run in sequence; stop on the first failure.
    let steps: Vec<(&str, Vec<String>)> = match action {
        "retrain-value" => vec![(
            "replay-value",
            vec!["--bin".into(), "train_corpus".into(), "--".into(), manifest.clone()],
        )],
        "calibrate-scoring" => vec![
            (
                "replay-scoring",
                vec!["--bin".into(), "calibrate".into(), "--".into(), manifest.clone()],
            ),
            (
                "replay-scoring",
                vec![
                    "--bin".into(),
                    "reconcile".into(),
                    "--".into(),
                    manifest.clone(),
                    "--promote".into(),
                ],
            ),
        ],
        _ => return None,
    };

    let mut out = String::new();
    let mut cmds = Vec::new();
    let mut ok = true;
    for (pkg, args) in steps {
        cmds.push(format!("cargo run --release -q -p {pkg} {}", args.join(" ")));
        let mut cmd = std::process::Command::new("cargo");
        cmd.args(["run", "--release", "-q", "-p", pkg]).args(&args);
        match cmd.output() {
            Ok(o) => {
                out.push_str(&String::from_utf8_lossy(&o.stdout));
                out.push_str(&String::from_utf8_lossy(&o.stderr));
                if !o.status.success() {
                    ok = false;
                    break;
                }
            }
            Err(e) => {
                out.push_str(&format!("failed to spawn cargo: {e}\n"));
                ok = false;
                break;
            }
        }
    }
    // Calibration output is verbose — keep the tail.
    const CAP: usize = 16_384;
    if out.len() > CAP {
        let tail = out.split_at(out.len() - CAP).1.to_string();
        out = format!("…(truncated to last {CAP} bytes)…\n{tail}");
    }
    Some(RunResult {
        action: action.to_string(),
        command: cmds.join(" && "),
        ok,
        output: out,
    })
}
