//! Authoritative-parity regression test on `assets/replays/419a.replay`.
//!
//! Another product (Spire; see `docs/competitor/spire/ASSESSMENT.md`) reports
//! headline numbers for this same replay. The score, shots, saves, demos, length
//! and roster are the replay's own header counters and replicated events, so any
//! correct decoder must reproduce them. Derived metrics (possession losses,
//! recoveries, xG) are deliberately absent: their definitions differ between
//! systems. The reference figures came from one screenshot, so a failure means
//! "the decode changed — check which side is right", not automatically a bug.

use std::collections::BTreeMap;
use std::sync::LazyLock;

use replay_analyzer::decode::{boxcars_adapter::BoxcarsParser, ReplayParser};
use replay_analyzer::{
    model::{Event, PlayerMeta},
    CanonicalMatch,
};

static M: LazyLock<CanonicalMatch> = LazyLock::new(|| {
    let path = format!(
        "{}/../../rleval/assets/replays/419a.replay",
        env!("CARGO_MANIFEST_DIR")
    );
    let bytes = std::fs::read(&path).unwrap_or_else(|e| panic!("read {path}: {e}"));
    let decoded = BoxcarsParser::new().parse(&bytes).expect("decode 419a");
    replay_analyzer::build_canonical(&decoded, "419a")
});

/// `(name, team, goals, assists, saves, shots, score)`; team 0 = blue, 1 = orange.
const ROSTER: [(&str, i32, i32, i32, i32, i32, i32); 6] = [
    ("Aestis", 0, 1, 0, 3, 3, 420),
    ("Cytogenesis", 1, 1, 1, 0, 6, 290),
    ("Figauro", 1, 0, 1, 4, 1, 385),
    ("Nadir", 0, 1, 0, 1, 1, 290),
    ("bubbajudd", 1, 1, 0, 1, 4, 290),
    ("clockberg", 0, 1, 2, 2, 4, 445),
];

#[test]
fn match_shape_score_and_length() {
    assert_eq!(M.team_size, Some(3));
    assert!(
        (M.duration_s - 393.0).abs() <= 1.0,
        "duration {}",
        M.duration_s
    );
    assert_eq!(M.team_scores, BTreeMap::from([(0, 3), (1, 2)]));
    let mut goals: Vec<i32> = M
        .events
        .iter()
        .filter_map(|e| match e {
            Event::Goal { team, .. } => *team,
            _ => None,
        })
        .collect();
    goals.sort();
    assert_eq!(goals, [0, 0, 0, 1, 1], "goal events vs final score");
}

#[test]
fn roster_and_totals_match_the_header() {
    let mut got: Vec<_> = M
        .players
        .iter()
        .map(|p| {
            (
                p.name.as_str(),
                p.team,
                p.goals,
                p.assists,
                p.saves,
                p.shots,
                p.score,
            )
        })
        .collect();
    got.sort();
    assert_eq!(got, ROSTER);
    let sum = |f: fn(&PlayerMeta) -> i32| M.players.iter().map(f).sum::<i32>();
    assert_eq!(
        (sum(|p| p.goals), sum(|p| p.shots), sum(|p| p.saves)),
        (5, 19, 11)
    );
}

#[test]
fn exactly_three_demos_between_the_right_players() {
    let mut demos: Vec<_> = M
        .events
        .iter()
        .filter_map(|e| match e {
            Event::Demo {
                attacker: Some(a),
                victim: Some(v),
                ..
            } => Some((a.as_str(), v.as_str())),
            _ => None,
        })
        .collect();
    demos.sort();
    assert_eq!(
        demos,
        [
            ("Nadir", "Figauro"),
            ("Nadir", "bubbajudd"),
            ("clockberg", "Figauro")
        ]
    );
}
