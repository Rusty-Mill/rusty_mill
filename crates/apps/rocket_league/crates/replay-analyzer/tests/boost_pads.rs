//! Boost-pad pickup attribution tests (`analyze::boost_pads`).
//!
//! A synthetic track is driven over three known pads — a midline big pad, a
//! small pad in the opponent half, and a big pad in the opponent half — and we
//! assert the kind / stolen / overfill attribution and the collected-sum
//! invariant (big + small == the net boost-gauge gain).

use replay_analyzer::analyze::boost_pads::{pad_pickups, pad_stats};
use replay_analyzer::field::PadKind;
use replay_analyzer::model::{PlayerTrack, TrackSample, Vec3};

fn s(t: f32, p: (f32, f32), boost: u8) -> TrackSample {
    TrackSample {
        t,
        actor_id: 1,
        p: Vec3 {
            x: p.0,
            y: p.1,
            z: 17.0,
        },
        v: Vec3 {
            x: 0.0,
            y: 0.0,
            z: 0.0,
        },
        boost: Some(boost),
        rot: None,
    }
}

/// Team-0 player (attack sign +1) collecting:
///   big pad (3584, 0) at midline, small pad (1788, 2300) opp half,
///   big pad (3072, 4096) opp half. Decreases between are boost use.
fn track() -> PlayerTrack {
    PlayerTrack {
        player: "P".into(),
        pri: 1,
        team: Some(0),
        num_segments: 1,
        samples: vec![
            s(0.0, (3584.0, 0.0), 0),      // on midline big pad, empty
            s(0.1, (3584.0, 0.0), 255),    // +100% big, midline → not stolen
            s(0.2, (0.0, 0.0), 200),       // use boost (decrease)
            s(0.3, (1788.0, 2300.0), 231), // +~12% small, opp half → stolen
            s(0.4, (3072.0, 4096.0), 10),  // use boost (decrease)
            s(0.5, (3072.0, 4096.0), 255), // +~96% big, opp half → stolen
        ],
        gaps: vec![],
    }
}

fn near(a: f32, b: f32, eps: f32) -> bool {
    (a - b).abs() < eps
}

#[test]
fn pickups_are_attributed_by_kind_and_half() {
    let picks = pad_pickups(&track(), 1);
    assert_eq!(picks.len(), 3, "three increases ⇒ three pickups");

    // midline big: not stolen, no overfill (collected from empty).
    assert_eq!(picks[0].kind, PadKind::Big);
    assert!(!picks[0].stolen);
    assert!(near(picks[0].gain, 100.0, 0.2));
    assert!(near(picks[0].overfill, 0.0, 0.2));

    // opp-half small: stolen.
    assert_eq!(picks[1].kind, PadKind::Small);
    assert!(picks[1].stolen);
    assert!(near(picks[1].gain, 12.16, 0.3));

    // opp-half big, grabbed at low boost: stolen, small overfill (~3.9%).
    assert_eq!(picks[2].kind, PadKind::Big);
    assert!(picks[2].stolen);
    assert!(
        near(picks[2].overfill, 3.92, 0.3),
        "overfill {}",
        picks[2].overfill
    );
}

#[test]
fn aggregates_and_collected_invariant() {
    let st = pad_stats(&track(), 1);
    assert_eq!(st.count_collected_big, 2);
    assert_eq!(st.count_collected_small, 1);
    assert_eq!(st.count_stolen_big, 1); // midline big is NOT stolen
    assert_eq!(st.count_stolen_small, 1);

    // big total = 100 + ~96.08, small = ~12.16.
    assert!(near(st.amount_collected_big, 196.08, 0.5));
    assert!(near(st.amount_collected_small, 12.16, 0.5));
    assert!(near(st.amount_stolen, 12.16 + 96.08, 0.7));
    assert!(near(st.amount_stolen_big, 96.08, 0.5));

    // Invariant: attributed big+small == net gauge gain (255+31+245 bytes).
    let gauge_gain = (255.0 + 31.0 + 245.0) / 255.0 * 100.0;
    assert!(
        near(
            st.amount_collected_big + st.amount_collected_small,
            gauge_gain,
            0.5
        ),
        "big+small {} vs gauge {}",
        st.amount_collected_big + st.amount_collected_small,
        gauge_gain
    );
}

#[test]
fn respawn_gap_is_not_a_pickup() {
    let mut t = track();
    // A spawn boost across a gap (10 → 255) must not read as a pickup.
    t.samples = vec![s(0.0, (0.0, 0.0), 10), s(1.0, (0.0, 0.0), 255)];
    t.gaps = vec![replay_analyzer::model::TrackGap {
        start: 0.0,
        end: 1.0,
        reason: replay_analyzer::model::GapReason::Respawn,
    }];
    assert!(pad_pickups(&t, 1).is_empty());
}
