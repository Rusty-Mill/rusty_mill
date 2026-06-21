//! Fixture test for T3 boost extraction: boost is linked to its car via the
//! component's `Vehicle` link, attached to per-frame car state and track
//! samples, and correctly re-linked when the car (and its boost component) are
//! recycled on respawn.

use replay_analyzer::analyze::build_canonical;
use replay_analyzer::decode::{
    ActorClass, ActorUpdate, DecodedReplay, NewActorEvent, RawFrame, ReplayMeta,
};
use replay_analyzer::model::PlayerMeta;
use std::collections::BTreeMap;

const ALICE: i32 = 100;

fn car(id: i32) -> NewActorEvent {
    NewActorEvent {
        actor_id: id,
        class: ActorClass::Car,
    }
}
fn comp(id: i32) -> NewActorEvent {
    // A boost component is not a car body; class is irrelevant to boost linkage.
    NewActorEvent {
        actor_id: id,
        class: ActorClass::Other,
    }
}
fn rb(actor: i32) -> ActorUpdate {
    ActorUpdate::RigidBody {
        actor,
        p: [0.0, 0.0, 17.0],
        v: [0.0, 0.0, 0.0],
        rot: [0.0, 0.0, 0.0],
        sleeping: false,
    }
}

/// Alice on car 8 (boost comp 50), demoed, respawns on car 15 (boost comp 51).
/// Boost amounts: 200 -> 150 on the first car, 77 on the respawn car.
fn fixture() -> DecodedReplay {
    let frames = vec![
        RawFrame {
            time: 0.0,
            delta: 0.0,
            new_actors: vec![car(8), comp(50)],
            updates: vec![
                ActorUpdate::CarPri { car: 8, pri: ALICE },
                ActorUpdate::PriName {
                    pri: ALICE,
                    name: "Alice".into(),
                },
                ActorUpdate::CompVehicle { comp: 50, car: 8 },
                ActorUpdate::BoostAmount {
                    comp: 50,
                    amount: 200,
                },
                rb(8),
            ],
            deleted: vec![],
        },
        RawFrame {
            time: 0.1,
            delta: 0.1,
            new_actors: vec![],
            updates: vec![
                ActorUpdate::BoostAmount {
                    comp: 50,
                    amount: 150,
                },
                rb(8),
            ],
            deleted: vec![],
        },
        RawFrame {
            time: 1.0,
            delta: 0.9,
            new_actors: vec![],
            updates: vec![],
            deleted: vec![8, 50],
        },
        RawFrame {
            time: 2.0,
            delta: 1.0,
            new_actors: vec![car(15), comp(51)],
            updates: vec![
                ActorUpdate::CarPri {
                    car: 15,
                    pri: ALICE,
                },
                ActorUpdate::CompVehicle { comp: 51, car: 15 },
                ActorUpdate::BoostAmount {
                    comp: 51,
                    amount: 77,
                },
                rb(15),
            ],
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
fn boost_is_linked_to_car_and_attached_to_state() {
    let m = build_canonical(&fixture(), "boost-fixture");

    // Per-frame car state carries the boost of the linked car.
    let boost_at = |t: f32, actor: i32| -> Option<u8> {
        m.frames
            .iter()
            .find(|f| (f.t - t).abs() < 1e-6)
            .and_then(|f| f.cars.iter().find(|c| c.actor_id == actor))
            .and_then(|c| c.boost)
    };
    assert_eq!(boost_at(0.0, 8), Some(200));
    assert_eq!(boost_at(0.1, 8), Some(150));
    assert_eq!(
        boost_at(2.0, 15),
        Some(77),
        "boost re-linked to respawn car"
    );

    // The coalesced track records boost per sample across both cars.
    let alice = m.tracks.iter().find(|t| t.player == "Alice").unwrap();
    let by_actor: Vec<(i32, Option<u8>)> = alice
        .samples
        .iter()
        .map(|s| (s.actor_id, s.boost))
        .collect();
    assert!(by_actor.contains(&(8, Some(200))));
    assert!(by_actor.contains(&(8, Some(150))));
    assert!(
        by_actor.contains(&(15, Some(77))),
        "respawn-car sample uses the new car's boost, not stale 150"
    );
}
