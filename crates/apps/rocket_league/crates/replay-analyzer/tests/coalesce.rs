//! Fixture-based unit test for T1 recycled-actor coalescing.
//!
//! Builds a neutral [`DecodedReplay`] by hand (no `.replay` file needed, since
//! the analyze layer depends only on the decode port) that isolates the exact
//! problem T1 solves: one player fragments across recycled car-actor ids, and an
//! id is later reused by a *different* player. The pipeline must collapse this
//! into one track per stable player identity — not one per car actor.

use replay_analyzer::analyze::build_canonical;
use replay_analyzer::decode::{
    ActorClass, ActorUpdate, DecodedReplay, NewActorEvent, RawFrame, ReplayMeta,
};
use replay_analyzer::model::PlayerMeta;
use std::collections::BTreeMap;

const ALICE: i32 = 100;
const BOB: i32 = 200;

fn player(name: &str, team: i32) -> PlayerMeta {
    PlayerMeta {
        name: name.to_string(),
        team,
        score: 0,
        goals: 0,
        assists: 0,
        saves: 0,
        shots: 0,
    }
}

fn new_car(actor_id: i32) -> NewActorEvent {
    NewActorEvent {
        actor_id,
        class: ActorClass::Car,
    }
}

fn rb(actor: i32, x: f32) -> ActorUpdate {
    ActorUpdate::RigidBody {
        actor,
        p: [x, 0.0, 17.0],
        v: [0.0, 0.0, 0.0],
        rot: [0.0, 0.0, 0.0],
        sleeping: false,
    }
}

/// Two players. Alice (PRI 100, team 0), Bob (PRI 200, team 1).
///
/// Timeline:
/// - t=0.0..0.2: Alice on car **8**, Bob on car **9** (3 samples each).
/// - t=1.0: both cars destroyed (demo / goal reset).
/// - t=3.0..3.1: Alice respawns on car **15**; Bob respawns on car **8**
///   (the id Alice used earlier — recycled).
///
/// Correct coalescing yields exactly 2 tracks, each with 2 segments and 1 gap,
/// and car id 8 split across both tracks by time.
fn recycled_actor_fixture() -> DecodedReplay {
    let frames = vec![
        RawFrame {
            time: 0.0,
            delta: 0.0,
            new_actors: vec![new_car(8), new_car(9)],
            updates: vec![
                ActorUpdate::CarPri { car: 8, pri: ALICE },
                ActorUpdate::CarPri { car: 9, pri: BOB },
                ActorUpdate::PriName {
                    pri: ALICE,
                    name: "Alice".into(),
                },
                ActorUpdate::PriName {
                    pri: BOB,
                    name: "Bob".into(),
                },
                rb(8, 100.0),
                rb(9, -100.0),
            ],
            deleted: vec![],
        },
        RawFrame {
            time: 0.1,
            delta: 0.1,
            new_actors: vec![],
            updates: vec![rb(8, 110.0), rb(9, -110.0)],
            deleted: vec![],
        },
        RawFrame {
            time: 0.2,
            delta: 0.1,
            new_actors: vec![],
            updates: vec![rb(8, 120.0), rb(9, -120.0)],
            deleted: vec![],
        },
        // Both cars destroyed.
        RawFrame {
            time: 1.0,
            delta: 0.8,
            new_actors: vec![],
            updates: vec![],
            deleted: vec![8, 9],
        },
        // Respawn: Alice -> car 15, Bob -> car 8 (recycled id).
        RawFrame {
            time: 3.0,
            delta: 2.0,
            new_actors: vec![new_car(15), new_car(8)],
            updates: vec![
                ActorUpdate::CarPri {
                    car: 15,
                    pri: ALICE,
                },
                ActorUpdate::CarPri { car: 8, pri: BOB },
                rb(15, 200.0),
                rb(8, -200.0),
            ],
            deleted: vec![],
        },
        RawFrame {
            time: 3.1,
            delta: 0.1,
            new_actors: vec![],
            updates: vec![rb(15, 210.0), rb(8, -210.0)],
            deleted: vec![],
        },
    ];

    DecodedReplay {
        meta: ReplayMeta {
            parser_version: "boxcars-test".into(),
            map: Some("TestArena".into()),
            team_size: Some(1),
            record_fps: Some(30.0),
            team_scores: BTreeMap::from([(0, 0), (1, 0)]),
            players: vec![player("Alice", 0), player("Bob", 1)],
            goals: Vec::new(),
        },
        frames,
    }
}

#[test]
fn recycled_actors_collapse_to_one_track_per_player() {
    let decoded = recycled_actor_fixture();
    let m = build_canonical(&decoded, "fixture");

    // The whole point: 4 distinct car-actor lifetimes (8, 9, 15, and 8-again)
    // across 2 players must coalesce to exactly 2 tracks.
    assert_eq!(m.tracks.len(), 2, "one track per stable player identity");

    let alice = m
        .tracks
        .iter()
        .find(|t| t.player == "Alice")
        .expect("Alice track present");
    let bob = m
        .tracks
        .iter()
        .find(|t| t.player == "Bob")
        .expect("Bob track present");

    assert_eq!(alice.pri, ALICE);
    assert_eq!(alice.team, Some(0));
    assert_eq!(bob.pri, BOB);
    assert_eq!(bob.team, Some(1));

    // Each player = 2 car segments, 1 respawn gap, 5 samples.
    for t in [alice, bob] {
        assert_eq!(t.num_segments, 2, "{} segments", t.player);
        assert_eq!(t.gaps.len(), 1, "{} gaps", t.player);
        assert_eq!(t.samples.len(), 5, "{} samples", t.player);

        // The gap spans the dead window; nothing is carried forward across it.
        let gap = t.gaps[0];
        assert!((gap.start - 0.2).abs() < 1e-4, "{} gap.start", t.player);
        assert!((gap.end - 3.0).abs() < 1e-4, "{} gap.end", t.player);
        assert!(
            !t.samples.iter().any(|s| s.t > gap.start && s.t < gap.end),
            "{}: no samples inside the gap",
            t.player
        );
    }

    // Car id 8 was Alice's first car AND Bob's respawn car: it must appear in
    // both tracks, split by time, never merged into a single identity.
    assert!(alice.samples.iter().any(|s| s.actor_id == 8 && s.t < 1.0));
    assert!(alice.samples.iter().any(|s| s.actor_id == 15));
    assert!(bob.samples.iter().any(|s| s.actor_id == 9));
    assert!(bob.samples.iter().any(|s| s.actor_id == 8 && s.t > 1.0));
}

#[test]
fn short_unbound_orphan_track_is_dropped() {
    // Alice is bound (car 8 -> PRI 100, named). Car 50 reports a couple of rigid
    // bodies but never binds a PRI — a transient glitch that must not coalesce
    // into a spurious "<unknown>" track.
    let frames = vec![
        RawFrame {
            time: 0.0,
            delta: 0.0,
            new_actors: vec![new_car(8), new_car(50)],
            updates: vec![
                ActorUpdate::CarPri { car: 8, pri: ALICE },
                ActorUpdate::PriName {
                    pri: ALICE,
                    name: "Alice".into(),
                },
                rb(8, 100.0),
                rb(50, 500.0),
            ],
            deleted: vec![],
        },
        RawFrame {
            time: 0.1,
            delta: 0.1,
            new_actors: vec![],
            updates: vec![rb(8, 110.0), rb(50, 510.0)],
            deleted: vec![50],
        },
        RawFrame {
            time: 0.2,
            delta: 0.1,
            new_actors: vec![],
            updates: vec![rb(8, 120.0)],
            deleted: vec![],
        },
    ];
    let decoded = DecodedReplay {
        meta: ReplayMeta {
            parser_version: "boxcars-test".into(),
            map: Some("TestArena".into()),
            team_size: Some(1),
            record_fps: Some(30.0),
            team_scores: BTreeMap::from([(0, 0)]),
            players: vec![player("Alice", 0)],
            goals: Vec::new(),
        },
        frames,
    };

    let m = build_canonical(&decoded, "fixture");
    assert_eq!(m.tracks.len(), 1, "the short unbound orphan is dropped");
    assert_eq!(m.tracks[0].player, "Alice");
    assert!(
        m.tracks.iter().all(|t| t.player != "<unknown>"),
        "no spurious <unknown> track"
    );
}
