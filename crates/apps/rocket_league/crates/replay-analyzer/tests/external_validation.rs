//! External-validation tests.
//!
//! Two layers:
//! * **Pure unit tests** of the comparison math and matching — always run, no
//!   replay files needed.
//! * A **corpus gate** that runs the analyzer over the committed ballchasing
//!   ground-truth fixture and asserts agreement within tolerance. It is active
//!   only where the (large, gitignored) corpus `.replay` files are present, and
//!   skips cleanly otherwise. The authoritative full-corpus gate is
//!   `cargo run --release --bin validate -- --gate`.

use std::path::Path;

use replay_analyzer::analyze::build_canonical;
use replay_analyzer::analyze::validate::{
    agreement, evaluate, pair_replay, spearman, GroundTruth, GtPlayer, GtReplay, Pair, Samples,
};
use replay_analyzer::decode::boxcars_adapter::BoxcarsParser;
use replay_analyzer::decode::ReplayParser;
use replay_analyzer::model::PlayerFeatures;

/// How many fixture replays the in-process corpus gate parses (debug-mode
/// boxcars is slow; the full 180-replay gate lives in the `validate` binary).
const CORPUS_TEST_LIMIT: usize = 16;

fn feat(pri: i32, player: &str, team: i32, sup: f32, dist: f32, boost: f32) -> PlayerFeatures {
    PlayerFeatures {
        pri,
        player: player.to_string(),
        team: Some(team),
        touches: 0,
        boost_used: boost,
        time_supersonic_s: sup,
        mean_dist_to_ball: dist,
        possession_time_s: 0.0,
    }
}

fn gt(name: &str, team: i32, sup: f32, dist: f32, bpm: f32) -> GtPlayer {
    GtPlayer {
        name: name.to_string(),
        team,
        time_supersonic_s: Some(sup),
        avg_dist_to_ball: Some(dist),
        bcpm: Some(bpm),
        bpm: Some(bpm),
    }
}

#[test]
fn spearman_is_monotone_invariant() {
    let xs = [1.0, 2.0, 3.0, 4.0, 5.0];
    let up = [10.0, 20.0, 33.0, 40.0, 55.0]; // strictly increasing
    let down = [9.0, 7.0, 5.0, 3.0, 1.0]; // strictly decreasing
    assert!((spearman(&xs, &up).unwrap() - 1.0).abs() < 1e-6);
    assert!((spearman(&xs, &down).unwrap() + 1.0).abs() < 1e-6);
    // Degenerate / too-short inputs yield None, not a panic.
    assert_eq!(spearman(&[1.0, 2.0], &[1.0, 2.0]), None);
    assert_eq!(spearman(&[1.0, 1.0, 1.0], &[1.0, 2.0, 3.0]), None);
}

#[test]
fn agreement_relative_error_and_floor() {
    // Perfect agreement → zero error, correlation 1.
    let exact = [
        Pair {
            ours: 100.0,
            theirs: 100.0,
        },
        Pair {
            ours: 200.0,
            theirs: 200.0,
        },
        Pair {
            ours: 300.0,
            theirs: 300.0,
        },
    ];
    let a = agreement(&exact, 1.0);
    assert_eq!(a.n, 3);
    assert!(a.median_rel_err < 1e-6);
    assert!((a.spearman.unwrap() - 1.0).abs() < 1e-6);

    // Every sample 10% high → median rel err 0.10.
    let off = [
        Pair {
            ours: 110.0,
            theirs: 100.0,
        },
        Pair {
            ours: 220.0,
            theirs: 200.0,
        },
        Pair {
            ours: 330.0,
            theirs: 300.0,
        },
    ];
    assert!((agreement(&off, 1.0).median_rel_err - 0.10).abs() < 1e-6);

    // The floor stops a near-zero ground truth from exploding the ratio.
    let tiny = [
        Pair {
            ours: 1.0,
            theirs: 0.0,
        },
        Pair {
            ours: 1.0,
            theirs: 0.0,
        },
    ];
    assert!((agreement(&tiny, 2.0).median_rel_err - 0.5).abs() < 1e-6); // 1 / max(0,2)
}

#[test]
fn pair_replay_matches_by_team_when_name_is_mangled() {
    // Ours preserves the true UTF-8 name; ballchasing strips the accent on Bób.
    let features = vec![
        feat(1, "Alice", 0, 10.0, 2000.0, 30.0),
        feat(2, "Bób", 0, 12.0, 2200.0, 35.0),
        feat(3, "Carol", 1, 8.0, 1800.0, 28.0),
        feat(4, "Dave", 1, 6.0, 2500.0, 40.0),
    ];
    let replay = GtReplay {
        duration_s: Some(60.0), // 1 minute → bpm == boost-units total
        players: vec![
            gt("Alice", 0, 10.0, 2000.0, 30.0),
            gt("Bob", 0, 12.0, 2200.0, 35.0), // accent stripped → no exact match
            gt("Carol", 1, 8.0, 1800.0, 28.0),
            gt("Dave", 1, 6.0, 2500.0, 40.0),
        ],
    };
    let mut acc = Samples::default();
    let mc = pair_replay(&features, &replay, 60.0, &mut acc);

    assert_eq!(mc.total, 4);
    assert_eq!(
        mc.matched, 4,
        "Bób should pair to Bob via the unique team-0 residual"
    );
    assert_eq!(acc.supersonic.len(), 4);
    // The team-forced pair carried Bób's stats against Bob's ground truth.
    let bob = acc
        .supersonic
        .iter()
        .find(|p| (p.ours - 12.0).abs() < 1e-6)
        .expect("Bób's supersonic pair present");
    assert!((bob.theirs - 12.0).abs() < 1e-6);
}

#[test]
fn pair_replay_skips_unknown_and_ambiguous_residual() {
    // Spurious unnamed track must never be force-matched; and a team with two
    // mangled names (2 residual ours vs 2 theirs) is left unpaired, not guessed.
    let features = vec![
        feat(1, "Alice", 0, 10.0, 2000.0, 30.0),
        feat(-1, "<unknown>", 0, 0.0, 0.0, 0.0),
        feat(3, "Çarol", 1, 8.0, 1800.0, 28.0),
        feat(4, "Davé", 1, 6.0, 2500.0, 40.0),
    ];
    let replay = GtReplay {
        duration_s: Some(60.0),
        players: vec![
            gt("Alice", 0, 10.0, 2000.0, 30.0),
            gt("Bob", 0, 12.0, 2200.0, 35.0), // no team-0 counterpart but <unknown> excluded
            gt("Carol", 1, 8.0, 1800.0, 28.0), // both team-1 names mangled → ambiguous
            gt("Dave", 1, 6.0, 2500.0, 40.0),
        ],
    };
    let mut acc = Samples::default();
    let mc = pair_replay(&features, &replay, 60.0, &mut acc);
    // Only Alice matches exactly; <unknown> not forced, team-1 pair is ambiguous.
    assert_eq!(mc.matched, 1);
}

#[test]
fn evaluate_passes_strong_agreement_and_flags_drift() {
    // Strongly-correlated, low-error samples across all channels → no failures.
    let mut good = Samples::default();
    // bpm is a perfect match; bcpm is deliberately noisier so the better mapping
    // (bpm) is the one boost() selects — mirroring the real corpus finding.
    let bcpm_noise = [3.0, 1.0, 4.0, 1.0, 5.0, 9.0, 2.0, 6.0, 5.0, 3.0, 5.0, 8.0];
    for (i, &noise) in bcpm_noise.iter().enumerate() {
        let x = i as f32;
        good.supersonic.push(Pair {
            ours: x,
            theirs: x + 0.1,
        });
        good.dist_to_ball.push(Pair {
            ours: 1000.0 + 50.0 * x,
            theirs: 1000.0 + 50.0 * x,
        });
        good.boost_bpm.push(Pair {
            ours: 20.0 * x,
            theirs: 20.0 * x,
        });
        good.boost_bcpm.push(Pair {
            ours: 20.0 * noise,
            theirs: 20.0 * x,
        });
    }
    let report = evaluate(&good, 48, 48);
    assert!((report.coverage - 1.0).abs() < 1e-6);
    assert_eq!(
        report.boost().0,
        "bpm",
        "the better-correlating boost mapping wins"
    );
    assert!(
        report.failures().is_empty(),
        "unexpected: {:?}",
        report.failures()
    );

    // Scramble the distance channel → low Spearman → a reported failure.
    let mut bad = good.clone();
    bad.dist_to_ball = (0..12)
        .map(|i| Pair {
            ours: ((i * 7) % 12) as f32,
            theirs: i as f32,
        })
        .collect();
    let fails = evaluate(&bad, 48, 48).failures();
    assert!(
        fails.iter().any(|f| f.contains("dist_to_ball")),
        "got {fails:?}"
    );

    // Low coverage is itself a failure.
    let fails = evaluate(&good, 40, 48).failures();
    assert!(
        fails.iter().any(|f| f.contains("coverage")),
        "got {fails:?}"
    );
}

#[test]
fn bcstats_agrees_with_ballchasing() {
    use replay_analyzer::analyze::bcstats::ballchasing_stats;
    use replay_analyzer::analyze::roster_match::{team_anchored_pairs, RosterSlot};

    let fixture = Path::new("assets/corpus/ballchasing_stats.json");
    if !fixture.exists() {
        eprintln!("skip bcstats gate: no ground-truth fixture");
        return;
    }
    let corpus = Path::new("assets/corpus");
    let truth: GroundTruth =
        serde_json::from_slice(&std::fs::read(fixture).expect("read fixture")).expect("parse");
    let mut ids: Vec<&String> = truth.replays.keys().collect();
    ids.sort();

    let (mut sup, mut dist, mut bpm, mut bcpm) = (Vec::new(), Vec::new(), Vec::new(), Vec::new());
    let mut present = 0usize;
    for id in ids.iter().take(CORPUS_TEST_LIMIT) {
        let path = corpus.join(format!("{id}.replay"));
        if !path.exists() {
            continue;
        }
        present += 1;
        let decoded = BoxcarsParser::new()
            .parse(&std::fs::read(&path).expect("read"))
            .expect("decode");
        let canonical = build_canonical(&decoded, (*id).clone());
        let stats = ballchasing_stats(&canonical);
        let entry = &truth.replays[*id];
        let ours: Vec<RosterSlot> = stats
            .iter()
            .map(|s| RosterSlot::new(s.team, &s.player))
            .collect();
        let theirs: Vec<RosterSlot> = entry
            .players
            .iter()
            .map(|g| RosterSlot::new(Some(g.team), &g.name))
            .collect();
        for (oi, gi) in team_anchored_pairs(&ours, &theirs) {
            let (s, g) = (&stats[oi], &entry.players[gi]);
            if let Some(t) = g.time_supersonic_s {
                sup.push(Pair {
                    ours: s.movement.time_supersonic_s,
                    theirs: t,
                });
            }
            if let Some(d) = g.avg_dist_to_ball {
                dist.push(Pair {
                    ours: s.positioning.avg_dist_to_ball,
                    theirs: d,
                });
            }
            if let Some(b) = g.bpm {
                bpm.push(Pair {
                    ours: s.boost.bpm,
                    theirs: b,
                });
            }
            if let Some(b) = g.bcpm {
                bcpm.push(Pair {
                    ours: s.boost.bcpm,
                    theirs: b,
                });
            }
        }
    }
    if present == 0 {
        eprintln!("skip bcstats gate: corpus .replay files not present (gitignored)");
        return;
    }

    let (sa, da, pa, ca) = (
        agreement(&sup, 2.0),
        agreement(&dist, 100.0),
        agreement(&bpm, 50.0),
        agreement(&bcpm, 50.0),
    );
    eprintln!(
        "bcstats vs ballchasing ({present} replays): \
         supersonic ρ={:?} rel={:.3} | dist ρ={:?} rel={:.3} | \
         bpm ρ={:?} rel={:.3} | bcpm ρ={:?} rel={:.3}",
        sa.spearman,
        sa.median_rel_err,
        da.spearman,
        da.median_rel_err,
        pa.spearman,
        pa.median_rel_err,
        ca.spearman,
        ca.median_rel_err,
    );

    // Thresholds locked from observed corpus agreement, with margin. bpm/bcpm are
    // a rate-vs-rate comparison of the inferred pad model (ours runs a little low),
    // so the relative-error bound is looser than the position/velocity channels.
    let ok = |a: &replay_analyzer::analyze::validate::Agreement, rho: f32, rel: f32| {
        a.spearman.unwrap_or(0.0) >= rho && a.median_rel_err <= rel
    };
    // Observed on the first 16 corpus replays: supersonic ρ≈0.999/2.5%,
    // dist ρ≈0.989/2.9%, bpm ρ≈0.94/7.1%, bcpm ρ≈0.92/8.4%. Bounds keep ~2–3×
    // margin so the gate guards against regressions without flaking.
    assert!(
        ok(&sa, 0.97, 0.08),
        "supersonic: ρ={:?} rel={:.3}",
        sa.spearman,
        sa.median_rel_err
    );
    assert!(
        ok(&da, 0.95, 0.08),
        "dist: ρ={:?} rel={:.3}",
        da.spearman,
        da.median_rel_err
    );
    assert!(
        ok(&pa, 0.88, 0.15),
        "bpm: ρ={:?} rel={:.3}",
        pa.spearman,
        pa.median_rel_err
    );
    assert!(
        ok(&ca, 0.85, 0.15),
        "bcpm: ρ={:?} rel={:.3}",
        ca.spearman,
        ca.median_rel_err
    );
}

#[test]
fn bcstats_full_agrees_with_ballchasing() {
    use replay_analyzer::analyze::bcstats::{ballchasing_stats, BcPlayerStats};
    use replay_analyzer::analyze::roster_match::{team_anchored_pairs, RosterSlot};
    use std::collections::HashMap;

    #[derive(serde::Deserialize)]
    struct FullGt {
        replays: HashMap<String, FullReplay>,
    }
    #[derive(serde::Deserialize)]
    struct FullReplay {
        players: Vec<FullPlayer>,
    }
    #[derive(serde::Deserialize)]
    struct FullPlayer {
        name: String,
        team: i32,
        boost: HashMap<String, f64>,
        movement: HashMap<String, f64>,
        positioning: HashMap<String, f64>,
    }

    let fixture = Path::new("assets/corpus/ballchasing_bcstats.json");
    if !fixture.exists() {
        eprintln!("skip bcstats-full gate: no fixture");
        return;
    }
    let corpus = Path::new("assets/corpus");
    let truth: FullGt =
        serde_json::from_slice(&std::fs::read(fixture).expect("read")).expect("parse");

    // (channel, group, ballchasing key, extractor) — our bcstats value vs theirs.
    type Ex = fn(&BcPlayerStats) -> f32;
    let channels: Vec<(&str, &str, &str, Ex)> = vec![
        ("bpm", "boost", "bpm", |s| s.boost.bpm),
        ("bcpm", "boost", "bcpm", |s| s.boost.bcpm),
        ("avg_boost", "boost", "avg_amount", |s| s.boost.avg_amount),
        ("pct_zero", "boost", "percent_zero_boost", |s| {
            s.boost.percent_zero
        }),
        ("pct_full", "boost", "percent_full_boost", |s| {
            s.boost.percent_full
        }),
        ("pct_b0_25", "boost", "percent_boost_0_25", |s| {
            s.boost.percent_0_25
        }),
        ("pct_b25_50", "boost", "percent_boost_25_50", |s| {
            s.boost.percent_25_50
        }),
        ("pct_b50_75", "boost", "percent_boost_50_75", |s| {
            s.boost.percent_50_75
        }),
        ("pct_b75_100", "boost", "percent_boost_75_100", |s| {
            s.boost.percent_75_100
        }),
        ("collected", "boost", "amount_collected", |s| {
            s.boost.amount_collected
        }),
        ("stolen", "boost", "amount_stolen", |s| {
            s.boost.amount_stolen
        }),
        ("cnt_big", "boost", "count_collected_big", |s| {
            s.boost.count_collected_big as f32
        }),
        ("cnt_small", "boost", "count_collected_small", |s| {
            s.boost.count_collected_small as f32
        }),
        ("overfill", "boost", "amount_overfill", |s| {
            s.boost.amount_overfill
        }),
        ("avg_speed", "movement", "avg_speed", |s| {
            s.movement.avg_speed
        }),
        ("total_dist", "movement", "total_distance", |s| {
            s.movement.total_distance
        }),
        ("pct_slow", "movement", "percent_slow_speed", |s| {
            s.movement.percent_slow
        }),
        ("pct_boostspd", "movement", "percent_boost_speed", |s| {
            s.movement.percent_boost_speed
        }),
        ("pct_super", "movement", "percent_supersonic_speed", |s| {
            s.movement.percent_supersonic
        }),
        ("pct_ground", "movement", "percent_ground", |s| {
            s.movement.percent_ground
        }),
        ("pct_lowair", "movement", "percent_low_air", |s| {
            s.movement.percent_low_air
        }),
        ("pct_highair", "movement", "percent_high_air", |s| {
            s.movement.percent_high_air
        }),
        ("dist_ball", "positioning", "avg_distance_to_ball", |s| {
            s.positioning.avg_dist_to_ball
        }),
        (
            "dist_poss",
            "positioning",
            "avg_distance_to_ball_possession",
            |s| s.positioning.avg_dist_to_ball_possession,
        ),
        (
            "dist_nopos",
            "positioning",
            "avg_distance_to_ball_no_possession",
            |s| s.positioning.avg_dist_to_ball_no_possession,
        ),
        ("dist_mates", "positioning", "avg_distance_to_mates", |s| {
            s.positioning.avg_dist_to_mates
        }),
        ("pct_def3", "positioning", "percent_defensive_third", |s| {
            s.positioning.percent_defensive_third
        }),
        ("pct_neu3", "positioning", "percent_neutral_third", |s| {
            s.positioning.percent_neutral_third
        }),
        ("pct_off3", "positioning", "percent_offensive_third", |s| {
            s.positioning.percent_offensive_third
        }),
        (
            "pct_defhalf",
            "positioning",
            "percent_defensive_half",
            |s| s.positioning.percent_defensive_half,
        ),
        (
            "pct_offhalf",
            "positioning",
            "percent_offensive_half",
            |s| s.positioning.percent_offensive_half,
        ),
        ("pct_behind", "positioning", "percent_behind_ball", |s| {
            s.positioning.percent_behind_ball
        }),
        ("pct_infront", "positioning", "percent_infront_ball", |s| {
            s.positioning.percent_infront_ball
        }),
        ("pct_mostback", "positioning", "percent_most_back", |s| {
            s.positioning.percent_most_back
        }),
        ("pct_mostfwd", "positioning", "percent_most_forward", |s| {
            s.positioning.percent_most_forward
        }),
        (
            "ga_lastdef",
            "positioning",
            "goals_against_while_last_defender",
            |s| s.positioning.goals_against_while_last_defender as f32,
        ),
    ];

    let mut acc: HashMap<&str, Vec<Pair>> = channels.iter().map(|c| (c.0, Vec::new())).collect();
    let mut present = 0usize;
    let mut ids: Vec<&String> = truth.replays.keys().collect();
    ids.sort();
    for id in ids {
        let path = corpus.join(format!("{id}.replay"));
        if !path.exists() {
            continue;
        }
        present += 1;
        let decoded = BoxcarsParser::new()
            .parse(&std::fs::read(&path).expect("read"))
            .expect("decode");
        let stats = ballchasing_stats(&build_canonical(&decoded, id.clone()));
        let gt = &truth.replays[id];
        let ours: Vec<RosterSlot> = stats
            .iter()
            .map(|s| RosterSlot::new(s.team, &s.player))
            .collect();
        let theirs: Vec<RosterSlot> = gt
            .players
            .iter()
            .map(|g| RosterSlot::new(Some(g.team), &g.name))
            .collect();
        for (oi, gi) in team_anchored_pairs(&ours, &theirs) {
            let (s, g) = (&stats[oi], &gt.players[gi]);
            for (name, group, key, ex) in &channels {
                let map = match *group {
                    "boost" => &g.boost,
                    "movement" => &g.movement,
                    _ => &g.positioning,
                };
                if let Some(&their) = map.get(*key) {
                    acc.get_mut(name).unwrap().push(Pair {
                        ours: ex(s),
                        theirs: their as f32,
                    });
                }
            }
        }
    }
    if present == 0 {
        eprintln!("skip bcstats-full gate: corpus .replay files not present");
        return;
    }

    // The strong core: channels that should be near-exact (the pad model, speed
    // buckets, thirds/halves, distances). The rest still must rank-correlate, but
    // are inherently noisier: the middle boost quartiles have little cross-player
    // variance; the low/high-air *magnitudes* depend on our approximate z-band
    // thresholds (rank stays high, magnitude drifts); possession-split distance
    // depends on our derived possession model.
    let core: &[&str] = &[
        "bpm",
        "collected",
        "stolen",
        "cnt_big",
        "cnt_small",
        "pct_zero",
        "pct_full",
        "avg_speed",
        "total_dist",
        "pct_slow",
        "pct_boostspd",
        "pct_super",
        "pct_ground",
        "dist_ball",
        "dist_mates",
        "pct_def3",
        "pct_neu3",
        "pct_off3",
        "pct_defhalf",
        "pct_offhalf",
        "pct_behind",
        "pct_infront",
        "pct_mostfwd",
    ];

    // The near-exact core thresholds (ρ≥0.90, rel≤0.12) were calibrated on the
    // full fixture — 96 players across every rank bucket. On a small subset
    // (e.g. only one bucket's replays downloaded) the cross-player variance
    // that carries ρ is range-restricted and the calibrated thresholds don't
    // apply; the ρ≥0.50 broken-reducer tripwire below still runs regardless.
    let core_gate = 2 * present >= truth.replays.len();
    if !core_gate {
        eprintln!(
            "note: only {present}/{} fixture replays present — near-exact core \
             gate skipped (range-restricted subset); broken-reducer tripwire still on",
            truth.replays.len()
        );
    }

    eprintln!("bcstats full block vs ballchasing ({present} replays):");
    let mut fails = Vec::new();
    for (name, ..) in &channels {
        let a = agreement(&acc[name], 1.0);
        let rho = a.spearman.unwrap_or(0.0);
        eprintln!(
            "  {:<14} n={:>3} ρ={:>6} rel={:.3}{}",
            name,
            a.n,
            a.spearman
                .map(|v| format!("{v:.3}"))
                .unwrap_or_else(|| "—".into()),
            a.median_rel_err,
            if core.contains(name) { "  [core]" } else { "" },
        );
        // Every channel must rank-correlate — a broken reducer (sign flip, wrong
        // field) tanks Spearman.
        if a.n >= 10 && rho < 0.50 {
            fails.push(format!("{name}: ρ={rho:.3} < 0.50"));
        }
        // Core channels must be near-exact (only judged at calibrated coverage).
        if core_gate && core.contains(name) && (rho < 0.90 || a.median_rel_err > 0.12) {
            fails.push(format!(
                "core {name}: ρ={rho:.3} rel={:.3}",
                a.median_rel_err
            ));
        }
    }
    assert!(fails.is_empty(), "bcstats-full gate failures: {fails:?}");
}

#[test]
fn corpus_agrees_with_ballchasing() {
    let fixture = Path::new("assets/corpus/ballchasing_stats.json");
    if !fixture.exists() {
        eprintln!(
            "skip corpus gate: no ground-truth fixture at {}",
            fixture.display()
        );
        return;
    }
    let corpus = Path::new("assets/corpus");
    let raw = std::fs::read(fixture).expect("read fixture");
    let truth: GroundTruth = serde_json::from_slice(&raw).expect("parse fixture");

    let mut ids: Vec<&String> = truth.replays.keys().collect();
    ids.sort();

    let mut acc = Samples::default();
    let (mut matched, mut total, mut present) = (0usize, 0usize, 0usize);
    for id in ids.iter().take(CORPUS_TEST_LIMIT) {
        let path = corpus.join(format!("{id}.replay"));
        if !path.exists() {
            continue;
        }
        present += 1;
        let data = std::fs::read(&path).expect("read replay");
        let decoded = BoxcarsParser::new().parse(&data).expect("decode replay");
        let canonical = build_canonical(&decoded, (*id).clone());
        let entry = &truth.replays[*id];
        let dur = entry.duration_s.unwrap_or(canonical.duration_s);
        let mc = pair_replay(&canonical.features, entry, dur, &mut acc);
        matched += mc.matched;
        total += mc.total;
    }

    if present == 0 {
        eprintln!("skip corpus gate: corpus .replay files not present (gitignored)");
        return;
    }

    let report = evaluate(&acc, matched, total);
    let fails = report.failures();
    assert!(
        fails.is_empty(),
        "external validation gate failed on {present} replays ({}/{} players): {:?}",
        report.matched,
        report.total,
        fails
    );
}
