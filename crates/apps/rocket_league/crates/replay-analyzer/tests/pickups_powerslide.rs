//! Fixture test for T6 authoritative pad pickups + powerslide intervals.
//!
//! Feeds a synthetic decode stream carrying the two replicated attributes the
//! parity-gap work surfaces — `PickupBoost` (a real pad collection, attributed
//! to the instigator car's player) and `Handbrake` (powerslide on/off) — and
//! asserts they land on the canonical model and flow into the ballchasing-shaped
//! `bcstats` block (exact pickup counts/gain, and powerslide count/time/avg).

use replay_analyzer::analyze::bcstats::ballchasing_stats;
use replay_analyzer::analyze::build_canonical;
use replay_analyzer::decode::{
    ActorClass, ActorUpdate, DecodedReplay, NewActorEvent, RawFrame, ReplayMeta,
};
use replay_analyzer::field;
use replay_analyzer::model::PlayerMeta;
use std::collections::BTreeMap;

const ALICE: i32 = 100;
const CAR: i32 = 8;
const COMP: i32 = 50;

fn car(id: i32) -> NewActorEvent {
    NewActorEvent {
        actor_id: id,
        class: ActorClass::Car,
    }
}
fn other(id: i32) -> NewActorEvent {
    NewActorEvent {
        actor_id: id,
        class: ActorClass::Other,
    }
}
fn rb_at(actor: i32, p: [f32; 3]) -> ActorUpdate {
    ActorUpdate::RigidBody {
        actor,
        p,
        v: [0.0, 0.0, 0.0],
        rot: [0.0, 0.0, 0.0],
        sleeping: false,
    }
}

/// Alice sits on a big pad (boost gauge byte 100 ≈ 39.2%), powerslides from
/// t=0.1 to t=0.5, collects the big pad at t=0.2, then drives onto a small pad
/// and collects it at t=0.6.
fn fixture() -> DecodedReplay {
    let big = field::BIG_BOOST_PADS[0]; // (3584, 0)
    let small = field::SMALL_BOOST_PADS[0]; // (0, -4240)
    let big_p = [big.0, big.1, 17.0];
    let small_p = [small.0, small.1, 17.0];

    let frames = vec![
        RawFrame {
            time: 0.0,
            delta: 0.0,
            new_actors: vec![car(CAR), other(COMP)],
            updates: vec![
                ActorUpdate::CarPri {
                    car: CAR,
                    pri: ALICE,
                },
                ActorUpdate::PriName {
                    pri: ALICE,
                    name: "Alice".into(),
                },
                ActorUpdate::CompVehicle {
                    comp: COMP,
                    car: CAR,
                },
                ActorUpdate::BoostAmount {
                    comp: COMP,
                    amount: 100, // ≈ 39.2%
                },
                rb_at(CAR, big_p),
            ],
            deleted: vec![],
        },
        RawFrame {
            time: 0.1,
            delta: 0.1,
            new_actors: vec![],
            updates: vec![
                ActorUpdate::Handbrake { car: CAR, on: true },
                rb_at(CAR, big_p),
            ],
            deleted: vec![],
        },
        RawFrame {
            time: 0.2,
            delta: 0.1,
            new_actors: vec![],
            // Real pad collection on the big pad the car is sitting on.
            updates: vec![ActorUpdate::PickupBoost {
                instigator_car: CAR,
            }],
            deleted: vec![],
        },
        RawFrame {
            time: 0.5,
            delta: 0.3,
            new_actors: vec![],
            updates: vec![
                ActorUpdate::Handbrake {
                    car: CAR,
                    on: false,
                },
                rb_at(CAR, small_p),
            ],
            deleted: vec![],
        },
        RawFrame {
            time: 0.6,
            delta: 0.1,
            new_actors: vec![],
            updates: vec![ActorUpdate::PickupBoost {
                instigator_car: CAR,
            }],
            deleted: vec![],
        },
        RawFrame {
            time: 0.7,
            delta: 0.1,
            new_actors: vec![],
            updates: vec![rb_at(CAR, small_p)],
            deleted: vec![],
        },
    ];

    DecodedReplay {
        meta: ReplayMeta {
            parser_version: "boxcars-test".into(),
            map: Some("TestArena".into()),
            team_size: Some(1),
            record_fps: Some(30.0),
            team_scores: BTreeMap::from([(0, 0)]),
            players: vec![PlayerMeta {
                platform_id: None,
                name: "Alice".into(),
                team: 0,
                score: 0,
                goals: 0,
                assists: 0,
                saves: 0,
                shots: 0,
                car_id: None,
                car_name: None,
                camera: None,
                steering_sensitivity: None,
            }],
            goals: Vec::new(),
        },
        frames,
    }
}

#[test]
fn pickups_are_captured_and_attributed() {
    let m = build_canonical(&fixture(), "pickup-fixture");

    assert_eq!(m.pickups.len(), 2, "two real pad collections");
    let big = m.pickups.iter().find(|p| p.big).expect("a big pickup");
    let small = m.pickups.iter().find(|p| !p.big).expect("a small pickup");

    assert_eq!(big.pri, ALICE);
    assert_eq!(small.pri, ALICE);
    // Big pad fills to 100 from ≈39.2% → gain ≈ 60.8, overfill ≈ 39.2.
    assert!((big.gain - 60.78).abs() < 0.5, "big gain = {}", big.gain);
    assert!(
        (big.overfill - 39.22).abs() < 0.5,
        "big overfill = {}",
        big.overfill
    );
    // Small pad grants 12 (the tank has room), no overfill.
    assert!(
        (small.gain - 12.0).abs() < 0.01,
        "small gain = {}",
        small.gain
    );
    assert_eq!(small.overfill, 0.0);
}

#[test]
fn powerslide_interval_is_captured() {
    let m = build_canonical(&fixture(), "ps-fixture");
    assert_eq!(m.powerslides.len(), 1);
    let ps = m.powerslides[0];
    assert_eq!(ps.pri, ALICE);
    assert!(
        (ps.duration() - 0.4).abs() < 1e-4,
        "duration = {}",
        ps.duration()
    );
}

#[test]
fn bcstats_boost_uses_gauge_not_pickup_events_and_powerslide() {
    let m = build_canonical(&fixture(), "bc-fixture");
    // The fixture fires two authoritative pickup events, but the boost gauge never
    // rises (it stays at byte 100), so the gauge-delta model — the actual stat
    // source — reports no collection. This proves bcstats ignores the raw
    // `VehiclePickup` events for the boost block (they over-count on real replays;
    // see `docs/ballchasing-comparison.md`).
    assert_eq!(m.pickups.len(), 2, "raw pickup events are still captured");
    let stats = ballchasing_stats(&m);
    let alice = stats.iter().find(|s| s.player == "Alice").expect("alice");
    assert_eq!(
        alice.boost.count_collected_big, 0,
        "no gauge rise → no collection"
    );
    assert_eq!(alice.boost.count_collected_small, 0);
    assert_eq!(alice.boost.amount_collected, 0.0);

    // Powerslide comes from the authoritative handbrake interval [0.1, 0.5],
    // ground-gated to the grid (the car sits at z=17, on the ground). Time is
    // frame-quantized by the 30 Hz grid, so allow ~1.5 frames of slack.
    assert_eq!(alice.movement.count_powerslide, 1);
    assert!(
        (alice.movement.time_powerslide_s - 0.4).abs() < 0.05,
        "time = {}",
        alice.movement.time_powerslide_s
    );
    assert!(
        (alice.movement.avg_powerslide_duration_s - alice.movement.time_powerslide_s).abs() < 1e-4
    );
}
