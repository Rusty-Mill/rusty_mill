//! CI contract test for the Phase 2 reconstruction cross-check (spec §13).
//! Fully **offline** — it decodes the committed sample replays with `boxcars` and
//! reconstructs them both ways (our `build_canonical` + subtr-actor) live, exactly
//! like `tests/external_validation.rs` / `canonical_roundtrip.rs` already do. No
//! network, no fixtures to capture: the independent decoder runs in-process.

use std::path::PathBuf;

use recon_check::{cross_check_recon, cross_check_replay, reconstructions};

fn replay_bytes(name: &str) -> Vec<u8> {
    let p = PathBuf::from(env!("CARGO_MANIFEST_DIR"))
        .join("../../rleval/assets/replays")
        .join(name);
    std::fs::read(&p).unwrap_or_else(|e| panic!("read {}: {e}", p.display()))
}

/// On a real replay the two independent reconstructions must agree within
/// tolerance (Tier R1) — and a deliberately-drifted ball must be caught. Both
/// share one (expensive) reconstruction pass.
#[test]
fn real_reconstructions_agree_and_drift_is_caught() {
    let (our, subtr) = reconstructions(&replay_bytes("42f2.replay"), "42f2").expect("reconstruct");

    let rep = cross_check_recon(&our, &subtr);
    assert!(
        rep.tier1_ok(),
        "real reconstructions must agree (R1), got: {:#?}",
        rep.tier1
    );
    // Both sample the full match on the same grid; agreement is tight.
    assert_eq!(rep.our_frames, rep.subtr_frames, "same fixed-rate grid");
    assert!(rep.coverage > 0.95, "coverage {:.3}", rep.coverage);
    assert!(
        rep.ball_median_uu < 60.0,
        "median ball Δ {:.1} uu",
        rep.ball_median_uu
    );

    // Drift: shove our ball +600 uu every frame. An independent decoder that no
    // longer agrees is exactly the reconstruction-regression tripwire.
    let mut drifted = our.clone();
    for f in &mut drifted.resampled.frames {
        if let Some(k) = f.ball.as_mut() {
            k.p.x += 600.0;
        }
    }
    let bad = cross_check_recon(&drifted, &subtr);
    assert!(
        !bad.tier1_ok(),
        "a 600 uu ball drift must trip Tier R1, but it passed"
    );
    assert!(
        bad.tier1.iter().any(|f| f.field == "ball_position"),
        "expected a ball_position breach: {:#?}",
        bad.tier1
    );
}

/// A second, larger (3v3) replay also agrees end-to-end through the public
/// one-call pipeline.
#[test]
fn second_replay_agrees() {
    let rep = cross_check_replay(&replay_bytes("419a.replay"), "419a").expect("cross-check");
    assert!(
        rep.tier1_ok(),
        "419a reconstructions must agree (R1), got: {:#?}",
        rep.tier1
    );
    assert!(rep.coverage > 0.95, "coverage {:.3}", rep.coverage);
}
