//! Golden-file test: detect skills in the validated 2v2 sample and pin a
//! compact, deterministic digest (per-player skill counts). The detection core
//! is pure, so this is reproducible. Regenerate with
//! `UPDATE_GOLDEN=1 cargo test -p replay-skills --test golden`.

use std::collections::BTreeMap;
use std::path::PathBuf;

use replay_analyzer::analyze::build_canonical;
use replay_analyzer::decode::boxcars_adapter::BoxcarsParser;
use replay_analyzer::decode::ReplayParser;
use replay_skills::{detect_all, Skill, SkillConfig};
use serde::Serialize;

const SAMPLE: &str = "42f2";

#[derive(Serialize)]
struct PlayerDigest {
    player: String,
    pri: i32,
    team: Option<i32>,
    /// Only skills performed at least once, keyed by stable skill key.
    skills: BTreeMap<String, usize>,
}

#[derive(Serialize)]
struct Digest {
    total_instances: usize,
    /// Total occurrences per skill across the lobby (catalog coverage at a glance).
    by_skill: BTreeMap<String, usize>,
    players: Vec<PlayerDigest>,
}

#[test]
fn skill_report_matches_golden() {
    let path = PathBuf::from(env!("CARGO_MANIFEST_DIR"))
        .join("../assets/replays")
        .join(format!("{SAMPLE}.replay"));
    let data = std::fs::read(&path).unwrap_or_else(|e| panic!("read {}: {e}", path.display()));
    let decoded = BoxcarsParser::new().parse(&data).expect("decode");
    let canonical = build_canonical(&decoded, SAMPLE);

    let cfg = SkillConfig::default();
    let report = detect_all(&canonical, &cfg);

    // Invariants that must hold regardless of the golden values.
    for i in &report.instances {
        assert!(
            (0.0..=1.0).contains(&i.confidence),
            "confidence out of range for {:?}",
            i.skill
        );
        assert!(i.t.is_finite(), "non-finite instance time");
    }
    // Per-player counts must reconcile with the flat instance list.
    for p in &report.players {
        for (skill, n) in &p.counts {
            let actual = report.count_for(*skill, p.pri);
            assert_eq!(actual, *n, "rollup/count mismatch for {:?}", skill);
        }
    }

    let mut by_skill: BTreeMap<String, usize> = BTreeMap::new();
    for s in Skill::ALL {
        let n = report.total(s);
        if n > 0 {
            by_skill.insert(s.key().to_string(), n);
        }
    }

    let players = report
        .players
        .iter()
        .map(|p| PlayerDigest {
            player: p.player.clone(),
            pri: p.pri,
            team: p.team,
            skills: p
                .counts
                .iter()
                .map(|(s, n)| (s.key().to_string(), *n))
                .collect(),
        })
        .collect();

    let digest = Digest {
        total_instances: report.instances.len(),
        by_skill,
        players,
    };

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
        "skill digest drifted; rerun with UPDATE_GOLDEN=1"
    );
}
