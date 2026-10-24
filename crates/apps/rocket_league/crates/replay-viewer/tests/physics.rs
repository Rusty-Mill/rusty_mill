//! Physics-run scenes (ADR-0085): a capture and a candidate run on one
//! timeline. The golden pins the whole scene for the synthetic capture fixture,
//! drawn with itself as the candidate. Regenerate with
//! `UPDATE_GOLDEN=1 cargo test -p replay-viewer --test physics`.

use std::path::PathBuf;

use rb_capture_ingest::CaptureFileSource;
use rb_domain::{PhysicsFrame, PhysicsStateSource};
use replay_viewer::physics::SIM_PRI_OFFSET;
use replay_viewer::{build_physics_scene, html};

fn example() -> Vec<PhysicsFrame> {
    let path = PathBuf::from(env!("CARGO_MANIFEST_DIR"))
        .join("../rb_capture_ingest/fixtures/example.capture.jsonl");
    CaptureFileSource::new(path)
        .frames()
        .expect("example capture")
}

#[test]
fn scene_matches_golden() {
    let frames = example();
    let scene = build_physics_scene("example", &frames, Some(&frames)).expect("scene");
    let json = serde_json::to_string_pretty(&scene).expect("json") + "\n";
    let golden =
        PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("tests/golden/example-capture.json");
    if std::env::var_os("UPDATE_GOLDEN").is_some() {
        std::fs::write(&golden, &json).expect("write golden");
    }
    let expected = std::fs::read_to_string(&golden).expect("read golden");
    assert_eq!(json, expected, "regenerate with UPDATE_GOLDEN=1");
}

#[test]
fn candidate_cars_are_ghosts_in_their_own_pri_range() {
    let frames = example();
    let scene = build_physics_scene("example", &frames, Some(&frames)).expect("scene");
    let ghosts: Vec<_> = scene.players.iter().filter(|p| p.ghost).collect();
    assert_eq!(ghosts.len(), 1);
    assert_eq!(ghosts[0].pri, SIM_PRI_OFFSET);
    for frame in &scene.frames {
        let pris: Vec<i32> = frame.cars.iter().map(|c| c.pri).collect();
        assert_eq!(pris, [0, SIM_PRI_OFFSET]);
        assert_eq!(frame.cars[0].p, frame.cars[1].p);
    }
    assert_eq!(scene.frames[0].t, 0.0);
    assert!((scene.duration_s - 0.2).abs() < 1e-3);
    let page = html(&scene);
    assert!(page.contains("\"ghost\":true"));
}

#[test]
fn recorded_frames_without_a_candidate_match_show_only_the_recording() {
    let frames = example();
    // A candidate that starts two ticks late, like one seeded mid-capture.
    let late = &frames[2..];
    let scene = build_physics_scene("example", &frames, Some(late)).expect("scene");
    let counts: Vec<usize> = scene.frames.iter().map(|f| f.cars.len()).collect();
    assert_eq!(counts, [1, 1, 2, 2, 2]);
}

#[test]
fn a_recording_alone_has_no_ghosts() {
    let scene = build_physics_scene("example", &example(), None).expect("scene");
    assert!(scene.players.iter().all(|p| !p.ghost));
    let json = serde_json::to_string(&scene).expect("json");
    assert!(!json.contains("ghost"), "ghost is omitted when false");
}

#[test]
fn an_empty_recording_is_an_error() {
    assert!(build_physics_scene("empty", &[], None).is_err());
}
