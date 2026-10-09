//! Golden-file test: build the playback scene for the validated 2v2 sample and
//! pin a compact, deterministic digest (frame/roster/event shape). Regenerate
//! with `UPDATE_GOLDEN=1 cargo test -p replay-viewer --test golden`.

use std::collections::BTreeMap;
use std::path::PathBuf;

use replay_analyzer::analyze::build_canonical;
use replay_analyzer::decode::boxcars_adapter::BoxcarsParser;
use replay_analyzer::decode::ReplayParser;
use replay_skills::{detect_all, SkillConfig};
use replay_viewer::{build_scene, html};
use serde::Serialize;

const SAMPLE: &str = "42f2";

#[derive(Serialize)]
struct Digest {
    replay_id: String,
    hz: f32,
    duration_s: f32,
    num_frames: usize,
    players: Vec<(i32, String, Option<i32>)>,
    events_by_kind: BTreeMap<String, usize>,
}

#[test]
fn scene_digest_matches_golden() {
    let path = PathBuf::from(env!("CARGO_MANIFEST_DIR"))
        .join("../../rleval/assets/replays")
        .join(format!("{SAMPLE}.replay"));
    let data = std::fs::read(&path).unwrap_or_else(|e| panic!("read {}: {e}", path.display()));
    let decoded = BoxcarsParser::new().parse(&data).expect("decode");
    let canonical = build_canonical(&decoded, SAMPLE);
    let skills = detect_all(&canonical, &SkillConfig::default());
    let scene = build_scene(&canonical, &skills.instances);

    // Invariants independent of the golden values.
    assert!(!scene.frames.is_empty(), "scene has playback frames");
    assert!(
        scene.frames.windows(2).all(|w| w[1].t >= w[0].t),
        "frames time-ordered"
    );
    assert!(
        scene.events.windows(2).all(|w| w[1].t >= w[0].t),
        "events time-ordered"
    );
    // The HTML must render and fully substitute its placeholders.
    let out = html(&scene);
    assert!(out.contains("const S = {") && !out.contains("/*SCENE_DATA*/"));

    let mut events_by_kind: BTreeMap<String, usize> = BTreeMap::new();
    for e in &scene.events {
        *events_by_kind.entry(e.kind.clone()).or_default() += 1;
    }
    let digest = Digest {
        replay_id: scene.replay_id.clone(),
        hz: scene.hz,
        duration_s: scene.duration_s,
        num_frames: scene.frames.len(),
        players: scene
            .players
            .iter()
            .map(|p| (p.pri, p.name.clone(), p.team))
            .collect(),
        events_by_kind,
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
        "scene digest drifted; rerun with UPDATE_GOLDEN=1"
    );
}
