//! Golden-file test: score every player in the validated 2v2 sample and pin a
//! compact, deterministic report digest. Regenerate with
//! `UPDATE_GOLDEN=1 cargo test -p replay-scoring --test golden`.

use replay_analyzer::analyze::build_canonical;
use replay_analyzer::decode::boxcars_adapter::BoxcarsParser;
use replay_analyzer::decode::ReplayParser;
use replay_scoring::{score_all, ScoreConfig};
use serde::Serialize;
use std::path::PathBuf;

const SAMPLE: &str = "42f2";

#[derive(Serialize)]
struct ReportDigest {
    player: String,
    pri: i32,
    team: Option<i32>,
    composite_x10: i64,
    first_x10: i64,
    second_x10: i64,
    general_x10: i64,
    licence: String,
    player_type: String,
    main_leak: String,
    confidence: String,
}

fn x10(v: f32) -> i64 {
    (v * 10.0).round() as i64
}

#[test]
fn scoring_report_matches_golden() {
    let path = PathBuf::from(env!("CARGO_MANIFEST_DIR"))
        .join("../../rleval/assets/replays")
        .join(format!("{SAMPLE}.replay"));
    let data = std::fs::read(&path).unwrap_or_else(|e| panic!("read {}: {e}", path.display()));
    let decoded = BoxcarsParser::new().parse(&data).expect("decode");
    let canonical = build_canonical(&decoded, SAMPLE);

    let cfg = ScoreConfig::default();
    let mut reports = score_all(&canonical, &cfg);
    reports.sort_by_key(|r| r.target_pri);

    // Sanity invariants on every report.
    for r in &reports {
        assert!(
            (0.0..=100.0).contains(&r.composite),
            "{} composite oob",
            r.target_player
        );
        for s in [r.first_man, r.second_man, r.general] {
            assert!((0.0..=100.0).contains(&s));
        }
        assert!(!r.licence.is_empty() && !r.player_type.is_empty());
    }

    let digest: Vec<ReportDigest> = reports
        .iter()
        .map(|r| ReportDigest {
            player: r.target_player.clone(),
            pri: r.target_pri,
            team: r.target_team,
            composite_x10: x10(r.composite),
            first_x10: x10(r.first_man),
            second_x10: x10(r.second_man),
            general_x10: x10(r.general),
            licence: r.licence.clone(),
            player_type: r.player_type.clone(),
            main_leak: r.main_leak.clone(),
            confidence: format!("{:?}", r.confidence),
        })
        .collect();

    let actual = serde_json::to_string_pretty(&digest).unwrap();
    let golden = PathBuf::from(env!("CARGO_MANIFEST_DIR"))
        .join("tests/golden")
        .join(format!("{SAMPLE}.json"));

    if std::env::var_os("UPDATE_GOLDEN").is_some() || !golden.exists() {
        std::fs::create_dir_all(golden.parent().unwrap()).unwrap();
        std::fs::write(&golden, &actual).unwrap();
        eprintln!("wrote golden {}", golden.display());
        return;
    }
    let expected = std::fs::read_to_string(&golden).unwrap();
    let norm = |s: &str| s.replace("\r\n", "\n").trim().to_string();
    assert_eq!(
        norm(&actual),
        norm(&expected),
        "scoring digest drifted; rerun with UPDATE_GOLDEN=1"
    );
}
