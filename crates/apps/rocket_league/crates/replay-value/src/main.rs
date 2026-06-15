//! CLI: parse a `.replay`, build the labeled value dataset, fit the value model,
//! and print per-player ΔV. Optionally dump the dataset (JSONL) for an external
//! trainer and/or the evaluation summary (JSON).

use replay_analyzer::analyze::build_canonical;
use replay_analyzer::decode::boxcars_adapter::BoxcarsParser;
use replay_analyzer::decode::ReplayParser;
use replay_value::dataset::to_jsonl;
use replay_value::{
    build_dataset, evaluate, per_player_delta_v, Evaluation, ValueConfig, ValueModel,
};
use std::error::Error;
use std::path::Path;
use std::process::ExitCode;

const USAGE: &str = "usage: replay-value <file.replay> [--horizon <s>] [--model <model.json>] [--dump <data.jsonl>] [--json <out.json>]";

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
    let mut dump = None;
    let mut json_out = None;
    let mut model_path = None;
    let mut cfg = ValueConfig::default();
    let mut it = std::env::args().skip(1);
    while let Some(arg) = it.next() {
        match arg.as_str() {
            "--horizon" => {
                cfg.horizon_s = it
                    .next()
                    .ok_or("--horizon needs seconds")?
                    .parse()
                    .map_err(|_| "--horizon must be a number")?;
            }
            "--dump" => dump = Some(it.next().ok_or("--dump needs a path")?),
            "--json" => json_out = Some(it.next().ok_or("--json needs a path")?),
            "--model" => model_path = Some(it.next().ok_or("--model needs a path")?),
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

    let data = std::fs::read(&replay)?;
    let decoded = BoxcarsParser::new().parse(&data)?;
    let replay_id = Path::new(&replay)
        .file_stem()
        .and_then(|s| s.to_str())
        .unwrap_or("replay")
        .to_string();
    let canonical = build_canonical(&decoded, replay_id);

    if let Some(path) = &dump {
        let ds = build_dataset(&canonical, &cfg);
        std::fs::write(path, to_jsonl(&ds))?;
        eprintln!("wrote {} labeled rows -> {path}", ds.rows.len());
    }

    let eval = match &model_path {
        Some(p) => {
            let model: ValueModel = serde_json::from_slice(&std::fs::read(p)?)?;
            let ds = build_dataset(&canonical, &cfg);
            let base = if ds.rows.is_empty() {
                0.0
            } else {
                ds.rows.iter().map(|r| r.y).sum::<f32>() / ds.rows.len() as f32
            };
            let log_loss = model.log_loss(&ds);
            let players = per_player_delta_v(&canonical, &model, &cfg);
            Evaluation {
                dataset_rows: ds.rows.len(),
                base_rate: base,
                log_loss,
                players,
                model,
            }
        }
        None => evaluate(&canonical, &cfg),
    };

    eprintln!(
        "value model (vcfg={} horizon={:.0}s): {} rows, base_rate={:.3}, train log_loss={:.4}",
        cfg.version, cfg.horizon_s, eval.dataset_rows, eval.base_rate, eval.log_loss
    );
    eprintln!("per-player ΔV (scoring-probability swing credited to each touch):");
    eprintln!(
        "  {:<22} {:<6} {:>7}  {:>9}  {:>9}",
        "player", "team", "touches", "sum_dV", "mean_dV"
    );
    for p in &eval.players {
        let name = p.player.clone().unwrap_or_else(|| format!("pri {}", p.pri));
        eprintln!(
            "  {:<22} {:<6} {:>7}  {:>+9.3}  {:>+9.4}",
            name,
            p.team.map(|t| t.to_string()).unwrap_or_else(|| "?".into()),
            p.touches,
            p.sum_dv,
            p.mean_dv
        );
    }

    if let Some(path) = &json_out {
        let payload = serde_json::json!({
            "replay_id": canonical.replay_id,
            "value_config_version": cfg.version,
            "parser_version": canonical.parser_version,
            "analyzer_version": canonical.analyzer_version,
            "horizon_s": cfg.horizon_s,
            "dataset_rows": eval.dataset_rows,
            "base_rate": eval.base_rate,
            "train_log_loss": eval.log_loss,
            "model": eval.model,
            "players": eval.players,
        });
        std::fs::write(path, serde_json::to_vec_pretty(&payload)?)?;
        eprintln!("\nwrote evaluation -> {path}");
    }
    Ok(())
}
