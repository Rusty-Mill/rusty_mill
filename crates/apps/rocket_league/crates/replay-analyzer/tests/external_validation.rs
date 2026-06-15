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
