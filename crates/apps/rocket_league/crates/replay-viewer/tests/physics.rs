//! Physics-run scenes (ADR-0085): a capture and a candidate run on one
//! timeline. The golden pins the whole scene for the synthetic capture fixture,
//! drawn with itself as the candidate. Regenerate with
//! `UPDATE_GOLDEN=1 cargo test -p replay-viewer --test physics`.

use std::path::PathBuf;

use rb_capture_ingest::CaptureFileSource;
use rb_domain::{PhysicsFrame, PhysicsStateSource};
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
fn candidate_cars_are_ghosts_with_negative_pris() {
    let frames = example();
    let scene = build_physics_scene("example", &frames, Some(&frames)).expect("scene");
    let ghosts: Vec<_> = scene.players.iter().filter(|p| p.ghost).collect();
    assert_eq!(ghosts.len(), 1);
    assert_eq!(ghosts[0].pri, -1);
    for frame in &scene.frames {
        let pris: Vec<i32> = frame.cars.iter().map(|c| c.pri).collect();
        assert_eq!(pris, [0, -1]);
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

fn samples(times: &[f32]) -> Vec<PhysicsFrame> {
    let template = example().remove(0);
    times
        .iter()
        .enumerate()
        .map(|(i, &t)| {
            let mut frame = template.clone();
            frame.timestamp_secs = t;
            frame.cars[0].position.x = i as f32 * 100.0;
            frame
        })
        .collect()
}

#[test]
fn closest_sample_wins_for_exact_jittered_and_mixed_rate_frames() {
    for (recorded, candidate, expected) in [
        (
            vec![0.0, 0.02, 0.04],
            vec![0.011, 0.020],
            vec![None, Some(100.0), None],
        ),
        (
            vec![0.0, 0.02, 0.04],
            vec![0.001, 0.012, 0.019, 0.039],
            vec![Some(0.0), Some(200.0), Some(300.0)],
        ),
        (
            vec![0.0, 0.01, 0.02, 0.03, 0.04],
            vec![0.0, 0.02, 0.04],
            vec![Some(0.0), None, Some(100.0), None, Some(200.0)],
        ),
    ] {
        let scene = build_physics_scene("nearest", &samples(&recorded), Some(&samples(&candidate)))
            .expect("scene");
        let actual: Vec<_> = scene
            .frames
            .iter()
            .map(|f| f.cars.get(1).map(|c| c.p[0]))
            .collect();
        assert_eq!(actual, expected);
    }
}

#[test]
fn equal_distance_ties_choose_the_earlier_timestamp() {
    // Binary fractions make the tie exact, independent of decimal rounding.
    let scene = build_physics_scene(
        "ties",
        &samples(&[0.0, 0.25, 0.5]),
        Some(&samples(&[0.125, 0.375])),
    )
    .expect("scene");
    assert_eq!(scene.frames[1].cars[1].p[0], 0.0);
}

#[test]
fn singleton_requires_an_exact_match() {
    let recorded = samples(&[10.0]);
    for (timestamp, count) in [(10.0, 2), (10.001, 1), (1000.0, 1)] {
        let scene =
            build_physics_scene("one", &recorded, Some(&samples(&[timestamp]))).expect("scene");
        assert_eq!(scene.frames[0].cars.len(), count);
        assert_eq!(scene.hz, 0.0);
    }
}

#[test]
fn invalid_timestamp_order_or_values_are_rejected_in_either_run() {
    for times in [
        vec![0.02, 0.0],
        vec![0.0, 0.0],
        vec![f32::NAN],
        vec![f32::INFINITY],
    ] {
        let invalid = samples(&times);
        let valid = samples(&[0.0, 0.02]);
        assert!(build_physics_scene("bad", &invalid, Some(&valid)).is_err());
        assert!(build_physics_scene("bad", &valid, Some(&invalid)).is_err());
    }
}

#[test]
fn multiple_cars_have_disjoint_stable_ids_in_frames_and_roster() {
    let mut recorded = samples(&[0.0, 0.02]);
    for frame in &mut recorded {
        let car = frame.cars[0];
        frame.cars = [0, 1_000_000, i32::MAX as u32]
            .into_iter()
            .map(|id| {
                let mut car = car;
                car.player_id = id;
                car
            })
            .collect();
    }
    let scene = build_physics_scene("cars", &recorded, Some(&recorded)).expect("scene");
    let expected = [0, 1_000_000, i32::MAX, -1, -1_000_001, i32::MIN];
    assert_eq!(
        scene.players.iter().map(|p| p.pri).collect::<Vec<_>>(),
        expected
    );
    for frame in scene.frames {
        assert_eq!(
            frame.cars.iter().map(|c| c.pri).collect::<Vec<_>>(),
            expected
        );
    }
    for id in [i32::MAX as u32 + 1, u32::MAX] {
        let mut invalid = samples(&[0.0]);
        invalid[0].cars[0].player_id = id;
        assert!(build_physics_scene("overflow", &invalid, None).is_err());
        assert!(build_physics_scene("overflow", &recorded, Some(&invalid)).is_err());
    }
}
