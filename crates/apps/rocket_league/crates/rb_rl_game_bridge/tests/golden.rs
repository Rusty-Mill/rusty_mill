//! The game-socket codec against buffers made by `Bridge.dll`'s own generated FlatBuffers code
//! (`tests/fixtures/*.bin`, produced once by a scratch tool; only the bytes are kept), and
//! against the Rust encoder's own output read back.
#![allow(clippy::unwrap_used, clippy::expect_used, clippy::panic)]

use rb_rl_game_bridge::{
    decode_controller_packet, decode_game_packet, encode_controller_packet, encode_game_packet,
    Command, GameEvent, VehicleInput,
};
use rb_rlbot_wire::{MatchPhase, Physics, Rotator, Vec3};

const GAME: &[u8] = include_bytes!("fixtures/game_packet.bin");
const CONTROLLER: &[u8] = include_bytes!("fixtures/controller_packet.bin");

fn v(x: f32, y: f32, z: f32) -> Vec3 {
    Vec3 { x, y, z }
}

/// The generator's `P(s)`: every component distinct so a swapped field shows.
fn physics(s: f32) -> Physics {
    Physics {
        location: v(s + 1.0, s + 2.0, s + 3.0),
        rotation: Rotator {
            pitch: s + 0.1,
            yaw: s + 0.2,
            roll: s + 0.3,
        },
        velocity: v(s + 4.0, s + 5.0, s + 6.0),
        angular_velocity: v(s + 7.0, s + 8.0, s + 9.0),
    }
}

fn expected_events() -> Vec<GameEvent> {
    vec![
        GameEvent::GameSpawn {
            map_name: "Stadium_P".into(),
            mode: 1,
            team_size: 3,
            gravity: Some(v(0.0, 0.0, -650.0)),
        },
        GameEvent::StateChange {
            phase: MatchPhase::Active,
            is_overtime: true,
        },
        GameEvent::TimeChange {
            display_seconds: 287,
        },
        GameEvent::GoalSpawn {
            id: 40,
            team: 1,
            center: v(0.0, 5120.0, 321.0),
            direction: v(0.0, -1.0, 0.0),
            width: 1786.0,
            height: 642.0,
        },
        GameEvent::PickupSpawn {
            id: 50,
            location: v(-3584.0, 0.0, 73.0),
            big: true,
        },
        GameEvent::PickupState {
            id: 50,
            picked_up: true,
        },
        GameEvent::PlayerSpawn {
            id: 7,
            command_id: Some(3),
            name: "Ana".into(),
            team: 1,
        },
        GameEvent::PlayerPhysics {
            id: 7,
            physics: physics(100.0),
            state: 3,
        },
        GameEvent::PlayerBoost {
            id: 7,
            is_active: true,
            amount: 66,
        },
        GameEvent::BallSpawn { id: 9 },
        GameEvent::BallPhysics {
            id: 9,
            physics: physics(200.0),
        },
        GameEvent::TeamScore { team: 1, score: 4 },
        GameEvent::ActorDestroyed { id: 7 },
    ]
}

#[test]
fn a_game_packet_built_by_the_dlls_generated_code_decodes_field_for_field() {
    let bundle = decode_game_packet(GAME).unwrap();
    assert_eq!(bundle.app_seconds, 12.5);
    assert_eq!(bundle.delta_physics_frames, 2);
    assert_eq!(bundle.events.len(), 13);
    for (got, want) in bundle.events.iter().zip(expected_events()) {
        assert_eq!(got, &want);
    }
}

fn expected_commands() -> Vec<(u16, Command)> {
    vec![
        (
            1,
            Command::PlayerInput {
                actor_id: 7,
                input: VehicleInput {
                    throttle: 1.0,
                    steer: -0.5,
                    pitch: 0.25,
                    yaw: -0.75,
                    roll: 0.125,
                    dodge_forward: -0.25,
                    dodge_strafe: -0.75,
                    jump: true,
                    boost: true,
                    handbrake: false,
                    use_item: true,
                },
            },
        ),
        (2, Command::Exec("Pause".into())),
        (3, Command::SetPaused(true)),
        (4, Command::EndMatch),
        (5, Command::DestroyActor(12)),
        (
            6,
            Command::SetPhysics {
                actor_id: 9,
                physics: physics(300.0),
            },
        ),
        (
            7,
            Command::SetBoost {
                actor_id: 7,
                amount: 80,
            },
        ),
    ]
}

#[test]
fn a_controller_packet_built_by_the_dlls_generated_code_decodes_field_for_field() {
    assert_eq!(
        decode_controller_packet(CONTROLLER).unwrap(),
        expected_commands()
    );
}

#[test]
fn what_the_rust_encoder_writes_the_rust_decoder_reads() {
    let bundle = decode_game_packet(GAME).unwrap();
    let again = decode_game_packet(&encode_game_packet(&bundle).unwrap()).unwrap();
    assert_eq!(again, bundle);

    let commands: Vec<Command> = expected_commands().into_iter().map(|(_, c)| c).collect();
    let bytes = encode_controller_packet(1, &commands).unwrap();
    assert_eq!(
        decode_controller_packet(&bytes).unwrap(),
        expected_commands()
    );
}

#[test]
fn command_ids_count_up_and_wrap() {
    let bytes = encode_controller_packet(65_535, &[Command::EndMatch, Command::EndMatch]).unwrap();
    let ids: Vec<u16> = decode_controller_packet(&bytes)
        .unwrap()
        .into_iter()
        .map(|(id, _)| id)
        .collect();
    assert_eq!(ids, [65_535, 0]);
}

#[test]
fn damaged_input_is_an_error_never_a_panic() {
    // Every truncation of both fixtures, and a flipped byte in each position.
    for fixture in [GAME, CONTROLLER] {
        for len in 0..fixture.len() {
            let _ = decode_game_packet(&fixture[..len]);
            let _ = decode_controller_packet(&fixture[..len]);
        }
        for at in 0..fixture.len() {
            let mut damaged = fixture.to_vec();
            damaged[at] ^= 0xFF;
            let _ = decode_game_packet(&damaged);
            let _ = decode_controller_packet(&damaged);
        }
    }
    assert!(decode_game_packet(&[]).is_err());
}

#[test]
fn an_unmodelled_event_is_reported_not_fatal() {
    // Tag 15 (PlayerStatChange) has no decoder; it must come through as `Other`.
    let mut bundle = decode_game_packet(GAME).unwrap();
    bundle.events = vec![GameEvent::Other(15)];
    let again = decode_game_packet(&encode_game_packet(&bundle).unwrap()).unwrap();
    assert_eq!(again.events, [GameEvent::Other(15)]);
}

/// With `RB_WRITE_ENCODED=<dir>` set, writes what the Rust encoder produces so the generated
/// C# reader can be pointed at it (the other half of the cross-check).
#[test]
fn write_encoded_buffers_for_the_generated_reader_when_asked() {
    let Ok(dir) = std::env::var("RB_WRITE_ENCODED") else {
        return;
    };
    let bundle = decode_game_packet(GAME).unwrap();
    std::fs::write(
        std::path::Path::new(&dir).join("rust_game_packet.bin"),
        encode_game_packet(&bundle).unwrap(),
    )
    .unwrap();
    let commands: Vec<Command> = expected_commands().into_iter().map(|(_, c)| c).collect();
    std::fs::write(
        std::path::Path::new(&dir).join("rust_controller_packet.bin"),
        encode_controller_packet(1, &commands).unwrap(),
    )
    .unwrap();
}
