//! Skill-threshold calibration harness.
//!
//! Reads a labeled corpus manifest (`{file, ranks:{name:tier}, playlist,
//! team_size}` per replay), runs `detect_all` on every replay, and reports, per
//! skill:
//!   * the observed metric distribution (count, p10/p50/p90 of the evidence
//!     magnitude — aerial peak height, power-shot ball speed, …);
//!   * Spearman(per-player mean metric, rank tier) — the detector-precision check:
//!     do better-ranked players show bigger/faster mechanics?
//!
//! Then it refits the calibratable confidence-ramp anchors (see
//! [`replay_skills::calibrate::refit_skill_config`]) and writes the fitted
//! [`SkillConfig`] to `assets/corpus/fitted_skill_config.json`, which
//! `replay-skills --config <path>` consumes.
//!
//! Usage: `calibrate-skills [manifest.json]` (default `assets/corpus/manifest.json`).
//! The corpus `.replay` files are gitignored — fetch with
//! `assets/corpus/refresh_corpus_replays.py` (needs `BALLCHASING_API_KEY`). Parsing
//! is parallel over cores; missing files are skipped, so it runs (reporting little)
//! without the corpus.

use std::collections::BTreeMap;
use std::error::Error;
use std::path::{Path, PathBuf};

use replay_analyzer::analyze::build_canonical;
use replay_analyzer::analyze::roster_match::{team_anchored_pairs, RosterSlot};
use replay_analyzer::decode::boxcars_adapter::BoxcarsParser;
use replay_analyzer::decode::ReplayParser;
use replay_skills::calibrate::{pctl, refit_skill_config, spearman};
use replay_skills::{candidate_metrics, detect_all, profiles, Skill, SkillConfig};
use serde::Deserialize;

#[derive(Deserialize)]
struct ManifestEntry {
    #[serde(default)]
    id: String,
    file: String,
    #[serde(default)]
    ranks: BTreeMap<String, i32>,
    #[serde(default)]
    playlist: Option<String>,
    #[serde(default)]
    team_size: Option<i32>,
}

/// The only playlist the calibration corpus admits (see `manifest.json`).
const RANKED_DOUBLES: &str = "ranked-doubles";

/// One detected-skill observation joined to its player's rank tier.
struct Obs {
    skill: Skill,
    metric: f32, // per-player mean of the skill's evidence magnitude
    tier: f32,
}

/// Per-replay collection: every instance's raw (gated) metric, every candidate
/// (pre-gate) metric, and the per-player mean-metric ↔ tier pairs.
struct Collected {
    raw: Vec<(Skill, f32)>,
    cand: Vec<(Skill, f32)>,
    obs: Vec<Obs>,
}

fn join_tiers(
    players: &[(String, Option<i32>)],
    ranks: &BTreeMap<String, i32>,
) -> Vec<Option<i32>> {
    // Exact-name match, then team-anchor the residuals against the ranked roster
    // (the manifest's rank keys carry no team, so this only recovers exact names
    // here; the scoring calibrator's ground-truth fixture is score-specific).
    let mut tiers: Vec<Option<i32>> = players
        .iter()
        .map(|(name, _)| ranks.get(name).copied())
        .collect();
    let left: Vec<RosterSlot> = players
        .iter()
        .map(|(name, team)| RosterSlot::new(*team, name))
        .collect();
    let right: Vec<RosterSlot> = ranks
        .keys()
        .map(|name| RosterSlot::new(None, name))
        .collect();
    let rank_names: Vec<&String> = ranks.keys().collect();
    for (li, ri) in team_anchored_pairs(&left, &right) {
        if tiers[li].is_none() {
            tiers[li] = ranks.get(rank_names[ri].as_str()).copied();
        }
    }
    tiers
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

    // Calibrate only on ranked 2v2 (the corpus contract); skip anything else loudly.
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

    let cfg = SkillConfig::default();
    let nthreads = std::thread::available_parallelism()
        .map(|n| n.get())
        .unwrap_or(4)
        .min(entries.len().max(1));
    let chunk = entries.len().div_ceil(nthreads).max(1);

    // Detect across the corpus in parallel; gather raw metrics + per-player obs.
    let collected: Vec<Collected> = std::thread::scope(|s| {
        let handles: Vec<_> = entries
            .chunks(chunk)
            .map(|slice| {
                let cfg = &cfg;
                let dir = &corpus_dir;
                s.spawn(move || {
                    let mut out = Vec::new();
                    for e in slice {
                        let data = match std::fs::read(dir.join(&e.file)) {
                            Ok(d) => d,
                            Err(_) => continue, // gitignored corpus absent → skip
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
                        if canonical.team_size != Some(2) {
                            continue; // defense-in-depth vs a mistagged manifest
                        }
                        let report = detect_all(&canonical, cfg);
                        let raw: Vec<(Skill, f32)> = report
                            .instances
                            .iter()
                            .map(|i| (i.skill, i.metric))
                            .collect();
                        // Candidate-mode: pre-gate metric for every candidate, so
                        // the calibrator can fit floors, not just ramp tops.
                        let cand: Vec<(Skill, f32)> = candidate_metrics(&canonical, cfg)
                            .into_iter()
                            .flat_map(|(skill, ms)| ms.into_iter().map(move |m| (skill, m)))
                            .collect();

                        // Per-player mean metric per skill, joined to rank tier.
                        let profs = profiles(&report, canonical.duration_s);
                        let players: Vec<(String, Option<i32>)> =
                            profs.iter().map(|p| (p.player.clone(), p.team)).collect();
                        let tiers = join_tiers(&players, &e.ranks);
                        let mut obs = Vec::new();
                        for (p, tier) in profs.iter().zip(&tiers) {
                            let Some(tier) = tier else { continue };
                            for (skill, stat) in &p.skills {
                                if stat.count > 0 {
                                    obs.push(Obs {
                                        skill: *skill,
                                        metric: stat.mean_metric,
                                        tier: *tier as f32,
                                    });
                                }
                            }
                        }
                        out.push(Collected { raw, cand, obs });
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

    // Pool: gated metrics per skill (distribution + fit), candidate (pre-gate)
    // metrics per skill (floor fit), and (mean, tier) per skill.
    let mut raw_by_skill: BTreeMap<Skill, Vec<f32>> = BTreeMap::new();
    let mut cand_by_skill: BTreeMap<Skill, Vec<f32>> = BTreeMap::new();
    let mut pairs_by_skill: BTreeMap<Skill, (Vec<f32>, Vec<f32>)> = BTreeMap::new();
    for c in &collected {
        for (skill, m) in &c.raw {
            raw_by_skill.entry(*skill).or_default().push(*m);
        }
        for (skill, m) in &c.cand {
            cand_by_skill.entry(*skill).or_default().push(*m);
        }
        for o in &c.obs {
            let e = pairs_by_skill.entry(o.skill).or_default();
            e.0.push(o.metric);
            e.1.push(o.tier);
        }
    }

    let replays_run: usize = collected.len();
    eprintln!("\nran {replays_run} replays\n");
    println!(
        "{:<20} {:>6}  {:>8} {:>8} {:>8}   {:>8} label",
        "skill", "count", "p10", "p50", "p90", "rho(rank)"
    );
    for skill in Skill::ALL {
        let Some(raws) = raw_by_skill.get(&skill) else {
            continue;
        };
        let label = skill.metric_label();
        if label.is_empty() {
            // No continuous magnitude (e.g. demo) — report the count only.
            println!("{:<20} {:>6}", skill.key(), raws.len());
            continue;
        }
        let rho = pairs_by_skill.get(&skill).and_then(|(m, t)| spearman(m, t));
        println!(
            "{:<20} {:>6}  {:>8.1} {:>8.1} {:>8.1}   {:>8}   {} ({})",
            skill.key(),
            raws.len(),
            pctl(raws, 0.10),
            pctl(raws, 0.50),
            pctl(raws, 0.90),
            rho.map(|v| format!("{v:+.3}"))
                .unwrap_or_else(|| "  n/a".into()),
            label,
            skill.metric_unit(),
        );
        // Candidate-mode: the pre-gate population the floor is fit from. The gated
        // row above is its upper tail; this shows how far below the floor sits.
        if let Some(c) = cand_by_skill.get(&skill).filter(|c| !c.is_empty()) {
            println!(
                "{:<20} {:>6}  {:>8.1} {:>8.1} {:>8.1}   {:>8}   candidates (pre-gate p50/p90/p99)",
                "  └ candidates",
                c.len(),
                pctl(c, 0.50),
                pctl(c, 0.90),
                pctl(c, 0.99),
                "",
            );
        }
    }

    // Refit the available ramp anchors (floor + top from candidates where present)
    // and write the fitted config.
    let fitted = refit_skill_config(&cfg, &raw_by_skill, &cand_by_skill);
    let out_path = corpus_dir.join("fitted_skill_config.json");
    std::fs::write(&out_path, serde_json::to_vec_pretty(&fitted)?)?;
    eprintln!(
        "\nwrote {} (version {}) — aerial_min_height {:.0} -> {:.0}, high_aerial_height {:.0} -> {:.0}",
        out_path.display(),
        fitted.version,
        cfg.aerial_min_height,
        fitted.aerial_min_height,
        cfg.high_aerial_height,
        fitted.high_aerial_height,
    );
    if replays_run == 0 {
        eprintln!(
            "note: no corpus replays found under {} — fetch with \
             assets/corpus/refresh_corpus_replays.py to calibrate for real",
            corpus_dir.display()
        );
    }
    Ok(())
}
