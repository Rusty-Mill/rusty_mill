//! Feature-mining probe: compute a battery of candidate per-player features from
//! the canonical model and report each one's Spearman correlation with rank over
//! the labeled corpus. A discovery tool — the strong, defensible signals it finds
//! become new metrics. (Supersonic time-share, found here, is already +0.56.)
//!
//! Usage: `probe [manifest.json]` (default `assets/corpus/manifest.json`).

use std::collections::BTreeMap;
use std::error::Error;
use std::path::{Path, PathBuf};

use replay_analyzer::analyze::build_canonical;
use replay_analyzer::decode::boxcars_adapter::BoxcarsParser;
use replay_analyzer::decode::ReplayParser;
use replay_analyzer::field::{BACK_WALL_Y, SUPERSONIC_SPEED};
use replay_analyzer::model::{CanonicalMatch, Event, Vec3};
use replay_scoring::calibrate::spearman;
use serde::Deserialize;

#[derive(Deserialize)]
struct ManifestEntry {
    file: String,
    #[serde(default)]
    ranks: BTreeMap<String, i32>,
}

fn norm(a: Vec3) -> f32 {
    (a.x * a.x + a.y * a.y + a.z * a.z).sqrt()
}
fn dist(a: Vec3, b: Vec3) -> f32 {
    norm(Vec3 {
        x: a.x - b.x,
        y: a.y - b.y,
        z: a.z - b.z,
    })
}

const THIRD_Y: f32 = BACK_WALL_Y / 3.0;

/// Candidate features for one player: `(name, value)` pairs.
fn player_features(m: &CanonicalMatch, pri: i32, team: i32) -> Vec<(&'static str, f32)> {
    let sign = m
        .resampled
        .team_attack_sign
        .get(&team)
        .copied()
        .unwrap_or(1) as f32;

    let (mut n, mut nball, mut nmate) = (0usize, 0usize, 0usize);
    let (mut supersonic, mut airborne, mut ground) = (0usize, 0usize, 0usize);
    let (mut sum_speed, mut sum_z, mut sum_db, mut sum_dm) = (0.0f32, 0.0f32, 0.0f32, 0.0f32);
    let (mut ahead, mut offhalf, mut defthird, mut attthird) = (0usize, 0usize, 0usize, 0usize);
    let (mut path, mut prev) = (0.0f32, None::<Vec3>);

    for f in &m.resampled.frames {
        let Some(c) = f.cars.iter().find(|c| c.pri == pri) else {
            prev = None;
            continue;
        };
        n += 1;
        let sp = norm(c.v);
        sum_speed += sp;
        sum_z += c.p.z;
        if sp >= SUPERSONIC_SPEED {
            supersonic += 1;
        }
        if c.p.z > 300.0 {
            airborne += 1;
        }
        if c.p.z < 50.0 {
            ground += 1;
        }
        if let Some(p) = prev {
            path += dist(c.p, p);
        }
        prev = Some(c.p);
        let ya = c.p.y * sign;
        if ya > 0.0 {
            offhalf += 1;
        }
        if ya < -THIRD_Y {
            defthird += 1;
        }
        if ya > THIRD_Y {
            attthird += 1;
        }
        if let Some(b) = f.ball {
            nball += 1;
            sum_db += dist(c.p, b.p);
            if ya > b.p.y * sign {
                ahead += 1;
            }
        }
        if let Some(mate) = f
            .cars
            .iter()
            .filter(|o| o.team == Some(team) && o.pri != pri)
            .min_by(|a, b| dist(a.p, c.p).total_cmp(&dist(b.p, c.p)))
        {
            nmate += 1;
            sum_dm += dist(c.p, mate.p);
        }
    }
    if n == 0 {
        return Vec::new();
    }

    let dur_s = m
        .resampled
        .frames
        .last()
        .map(|f| f.t)
        .unwrap_or(1.0)
        .max(1.0);
    let dur_min = dur_s / 60.0;
    let nf = n as f32;
    let (mut touches, mut demos_for, mut demos_against) = (0usize, 0usize, 0usize);
    for e in &m.events {
        match e {
            Event::Touch { pri: tp, .. } if *tp == pri => touches += 1,
            Event::Demo {
                attacker_pri,
                victim_pri,
                ..
            } => {
                if *attacker_pri == Some(pri) {
                    demos_for += 1;
                }
                if *victim_pri == Some(pri) {
                    demos_against += 1;
                }
            }
            _ => {}
        }
    }

    let frac = |k: usize, d: usize| if d > 0 { k as f32 / d as f32 } else { 0.0 };
    vec![
        ("mean_speed", sum_speed / nf),
        ("frac_supersonic", supersonic as f32 / nf),
        ("frac_airborne", airborne as f32 / nf),
        ("frac_ground", ground as f32 / nf),
        ("mean_height", sum_z / nf),
        ("path_per_s", path / dur_s),
        ("mean_dist_ball", sum_db / nball.max(1) as f32),
        ("mean_dist_mate", sum_dm / nmate.max(1) as f32),
        ("frac_ahead_of_ball", frac(ahead, nball)),
        ("frac_offensive_half", frac(offhalf, n)),
        ("frac_def_third", frac(defthird, n)),
        ("frac_att_third", frac(attthird, n)),
        ("touches_per_min", touches as f32 / dur_min.max(1e-3)),
        ("demos_for_per_min", demos_for as f32 / dur_min.max(1e-3)),
        (
            "demos_against_per_min",
            demos_against as f32 / dur_min.max(1e-3),
        ),
        (
            "demo_diff_per_min",
            (demos_for as f32 - demos_against as f32) / dur_min.max(1e-3),
        ),
    ]
}

fn main() -> Result<(), Box<dyn Error>> {
    let manifest_path = PathBuf::from(
        std::env::args()
            .nth(1)
            .unwrap_or_else(|| "assets/corpus/manifest.json".into()),
    );
    let dir = manifest_path
        .parent()
        .unwrap_or(Path::new("."))
        .to_path_buf();
    let entries: Vec<ManifestEntry> = serde_json::from_slice(&std::fs::read(&manifest_path)?)?;
    eprintln!("probing {} replays", entries.len());

    let nthreads = std::thread::available_parallelism()
        .map(|n| n.get())
        .unwrap_or(4)
        .min(entries.len().max(1));
    let chunk = entries.len().div_ceil(nthreads);

    // (feature value, rank tier) samples, accumulated per feature name.
    let samples: Vec<(&'static str, f32, f32)> = std::thread::scope(|s| {
        let handles: Vec<_> = entries
            .chunks(chunk)
            .map(|slice| {
                let dir = &dir;
                s.spawn(move || {
                    let mut out: Vec<(&'static str, f32, f32)> = Vec::new();
                    for e in slice {
                        let Ok(data) = std::fs::read(dir.join(&e.file)) else {
                            continue;
                        };
                        let Ok(decoded) = BoxcarsParser::new().parse(&data) else {
                            continue;
                        };
                        let m = build_canonical(&decoded, "x");
                        for t in &m.tracks {
                            let Some(&tier) = e.ranks.get(&t.player) else {
                                continue;
                            };
                            let Some(team) = t.team else { continue };
                            for (name, val) in player_features(&m, t.pri, team) {
                                if val.is_finite() {
                                    out.push((name, val, tier as f32));
                                }
                            }
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

    // Group by feature and correlate with rank.
    let mut by_feat: BTreeMap<&str, (Vec<f32>, Vec<f32>)> = BTreeMap::new();
    for (name, val, tier) in samples {
        let e = by_feat.entry(name).or_default();
        e.0.push(val);
        e.1.push(tier);
    }
    let mut rows: Vec<(&str, f32, usize)> = by_feat
        .iter()
        .map(|(name, (xs, ys))| (*name, spearman(xs, ys).unwrap_or(0.0), xs.len()))
        .collect();
    rows.sort_by(|a, b| b.1.abs().total_cmp(&a.1.abs()));

    println!("candidate features, by |Spearman(feature, rank)|:");
    for (name, rho, n) in rows {
        println!("  {name:<24} {rho:>+7.3}   n={n}");
    }
    Ok(())
}
