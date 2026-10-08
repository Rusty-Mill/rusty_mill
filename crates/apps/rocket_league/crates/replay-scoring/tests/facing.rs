//! Convention guard: the heading derived from `rot.yaw` (via `CarView::facing`)
//! must point the same way the car is driving. If the analyzer's
//! quaternion→Euler extraction or the yaw→heading mapping ever drifts, the
//! facing-based metrics (challenge_timing / transition_readiness /
//! recovery_speed) silently invert; this catches that on real data.

use replay_analyzer::analyze::build_canonical;
use replay_analyzer::decode::boxcars_adapter::BoxcarsParser;
use replay_analyzer::decode::ReplayParser;
use replay_scoring::features::build_frames;
use replay_scoring::ScoreConfig;
use std::path::PathBuf;

#[test]
fn heading_aligns_with_velocity_on_real_sample() {
    let path =
        PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("../../rleval/assets/replays/42f2.replay");
    let data = std::fs::read(&path).unwrap_or_else(|e| panic!("read {}: {e}", path.display()));
    let decoded = BoxcarsParser::new().parse(&data).expect("decode");
    let canonical = build_canonical(&decoded, "42f2");
    let cfg = ScoreConfig::default();
    let frames = build_frames(&canonical, &cfg);

    // Average heading↔velocity alignment over clearly-driving cars (ground-plane
    // speed > 1000 uu/s). Cars drive forward the vast majority of the time, so a
    // correct convention yields a strongly positive mean; an inverted one would
    // be strongly negative.
    let (mut sum, mut n) = (0.0f32, 0usize);
    for f in &frames {
        for c in &f.cars {
            let sp = (c.v.x * c.v.x + c.v.y * c.v.y).sqrt();
            if sp < 1000.0 {
                continue;
            }
            if let Some(a) = c.forward_align(c.v) {
                sum += a;
                n += 1;
            }
        }
    }
    assert!(n > 1000, "too few fast-moving samples to judge: {n}");
    let mean = sum / n as f32;
    assert!(
        mean > 0.6,
        "heading should align with driving direction (yaw convention); mean = {mean}"
    );
}
