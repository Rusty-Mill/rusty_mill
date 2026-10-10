//! Every modelled message against a payload made by the reference implementation, and a round
//! trip through this crate's own encoder. The values here are the ones the fixtures were made
//! from (see `tests/fixtures/mod.rs`).
#![allow(clippy::unwrap_used)]

mod fixtures;

use rb_rlbot_wire::frame::{frame, FrameDecoder};
use rb_rlbot_wire::*;

fn unhex(s: &str) -> Vec<u8> {
    (0..s.len())
        .step_by(2)
        .map(|i| u8::from_str_radix(&s[i..i + 2], 16).unwrap())
        .collect()
}

fn interface(hex: &str) -> InterfaceMessage {
    InterfaceMessage::from_payload(&unhex(hex)).unwrap()
}

fn core(hex: &str) -> CoreMessage {
    CoreMessage::from_payload(&unhex(hex)).unwrap()
}

/// The fixture decodes to `expected`, and so does what this crate encodes from it.
fn check_interface(hex: &str, expected: InterfaceMessage) {
    assert_eq!(interface(hex), expected, "reference bytes");
    let ours = expected.to_payload().unwrap();
    assert_eq!(
        InterfaceMessage::from_payload(&ours).unwrap(),
        expected,
        "round trip"
    );
}

fn check_core(hex: &str, expected: CoreMessage) {
    assert_eq!(core(hex), expected, "reference bytes");
    let ours = expected.to_payload().unwrap();
    assert_eq!(
        CoreMessage::from_payload(&ours).unwrap(),
        expected,
        "round trip"
    );
}

fn partial(x: f32, y: f32, z: f32) -> Option<PartialVec3> {
    Some(PartialVec3 {
        x: Some(x),
        y: Some(y),
        z: Some(z),
    })
}

fn config() -> MatchConfiguration {
    MatchConfiguration {
        launcher: Launcher::Epic,
        game_map_upk: "Stadium_P".into(),
        player_configurations: vec![
            PlayerConfiguration {
                variety: PlayerClass::CustomBot(CustomBot {
                    name: "tape".into(),
                    root_dir: "C:/bots".into(),
                    run_command: "rb_tape_bot.exe".into(),
                    agent_id: "rb/tape/0".into(),
                    hivemind: true,
                    environment: Some(vec![EnvironmentVariable {
                        name: "RB_TAPE".into(),
                        value: "1".into(),
                    }]),
                }),
                team: 0,
                player_id: 7,
            },
            PlayerConfiguration {
                variety: PlayerClass::PsyonixBot(PsyonixBot {
                    name: "Pro".into(),
                    skill: PsyonixSkill::AllStar,
                }),
                team: 1,
                player_id: -3,
            },
            PlayerConfiguration {
                variety: PlayerClass::Human,
                team: 1,
                player_id: 0,
            },
        ],
        game_mode: GameMode::Hoops,
        mutators: Some(MutatorSettings {
            match_length: MatchLengthMutator::Unlimited,
        }),
        existing_match_behavior: ExistingMatchBehavior::RestartIfDifferent,
        enable_rendering: DebugRendering::AlwaysOff,
        enable_state_setting: false,
        auto_start_agents: false,
        freeplay: true,
        ..MatchConfiguration::default()
    }
}

fn physics(k: f32) -> Physics {
    Physics {
        location: Vec3 {
            x: k,
            y: k + 1.0,
            z: k + 2.0,
        },
        rotation: Rotator {
            pitch: 0.1 * k,
            yaw: 0.2 * k,
            roll: 0.3 * k,
        },
        velocity: Vec3 {
            x: -k,
            y: 2.0 * k,
            z: 0.5,
        },
        angular_velocity: Vec3 {
            x: 0.0,
            y: 0.0,
            z: k,
        },
    }
}

#[test]
fn client_messages_match_the_reference_bytes() {
    check_interface(
        fixtures::I_CONNECTION,
        InterfaceMessage::ConnectionSettings(ConnectionSettings {
            agent_id: "rusty/bot1".into(),
            wants_ball_predictions: true,
            wants_comms: false,
            close_between_matches: true,
        }),
    );
    check_interface(
        fixtures::I_STOP,
        InterfaceMessage::StopCommand(StopCommand {
            shutdown_server: true,
        }),
    );
    check_interface(
        fixtures::I_INIT,
        InterfaceMessage::InitComplete(InitComplete),
    );
    check_interface(
        fixtures::I_INPUT,
        InterfaceMessage::PlayerInput(PlayerInput {
            player_index: 3,
            controller_state: ControllerState {
                throttle: 1.0,
                steer: -0.5,
                pitch: 0.25,
                yaw: 0.0,
                roll: -1.0,
                jump: true,
                boost: true,
                handbrake: false,
                use_item: false,
            },
        }),
    );
}

#[test]
fn state_setting_matches_the_reference_bytes() {
    check_interface(
        fixtures::I_STATE,
        InterfaceMessage::DesiredGameState(DesiredGameState {
            ball_states: vec![
                DesiredBallState {
                    physics: DesiredPhysics {
                        location: partial(1.0, 2.0, 3.0),
                        velocity: partial(0.0, 100.0, -5.5),
                        ..DesiredPhysics::default()
                    },
                },
                DesiredBallState {
                    physics: DesiredPhysics {
                        rotation: Some(PartialRotator {
                            pitch: None,
                            yaw: Some(1.5),
                            roll: None,
                        }),
                        angular_velocity: partial(9.0, 8.0, 7.0),
                        ..DesiredPhysics::default()
                    },
                },
            ],
            car_states: vec![
                DesiredCarState {
                    physics: Some(DesiredPhysics {
                        location: partial(0.0, 0.0, 17.0),
                        ..DesiredPhysics::default()
                    }),
                    boost_amount: Some(33.0),
                },
                DesiredCarState {
                    physics: None,
                    boost_amount: None,
                },
            ],
            match_info: Some(DesiredMatchInfo {
                world_gravity_z: Some(-650.0),
                game_speed: None,
            }),
        }),
    );
}

#[test]
fn match_configuration_matches_the_reference_bytes_in_both_directions() {
    check_interface(
        fixtures::I_CONFIG,
        InterfaceMessage::MatchConfiguration(config()),
    );
    check_core(
        fixtures::C_CONFIG,
        CoreMessage::MatchConfiguration(config()),
    );
}

#[test]
fn unset_schema_defaults_survive_a_round_trip() {
    // Defaults that are not zero: agents auto-started and waited for, state setting on.
    let plain = MatchConfiguration::default();
    let back = InterfaceMessage::from_payload(
        &InterfaceMessage::MatchConfiguration(plain.clone())
            .to_payload()
            .unwrap(),
    )
    .unwrap();
    assert_eq!(back, InterfaceMessage::MatchConfiguration(plain));
}

#[test]
fn a_game_packet_matches_the_reference_bytes() {
    let player = |k, state, timeout, bot: bool, name: &str, team, boost, id| PlayerInfo {
        physics: physics(k),
        air_state: state,
        demolished_timeout: timeout,
        is_bot: bot,
        name: name.into(),
        team,
        boost,
        player_id: id,
    };
    check_core(
        fixtures::C_PACKET,
        CoreMessage::GamePacket(GamePacket {
            players: vec![
                player(
                    10.0,
                    AirState::DoubleJumping,
                    1.25,
                    true,
                    "tape",
                    0,
                    66.0,
                    7,
                ),
                player(20.0, AirState::OnGround, -1.0, false, "human", 1, 0.0, -2),
            ],
            boost_pads: vec![
                BoostPadState {
                    is_active: true,
                    timer: 0.0,
                },
                BoostPadState {
                    is_active: false,
                    timer: 4.5,
                },
                BoostPadState {
                    is_active: true,
                    timer: 1.0,
                },
            ],
            balls: vec![BallInfo {
                physics: physics(3.0),
            }],
            match_info: MatchInfo {
                seconds_elapsed: 12.5,
                game_time_remaining: 287.5,
                is_overtime: true,
                is_unlimited_time: false,
                match_phase: MatchPhase::GoalScored,
                frame_num: 1500,
            },
            teams: vec![
                TeamInfo {
                    team_index: 0,
                    score: 2,
                },
                TeamInfo {
                    team_index: 1,
                    score: 5,
                },
            ],
        }),
    );
}

#[test]
fn field_and_team_info_match_the_reference_bytes() {
    check_core(
        fixtures::C_FIELD,
        CoreMessage::FieldInfo(FieldInfo {
            boost_pads: vec![
                BoostPad {
                    location: Vec3 {
                        x: 0.0,
                        y: -4240.0,
                        z: 70.0,
                    },
                    is_full_boost: false,
                },
                BoostPad {
                    location: Vec3 {
                        x: -3072.0,
                        y: 4096.0,
                        z: 73.0,
                    },
                    is_full_boost: true,
                },
            ],
            goals: vec![GoalInfo {
                team_num: 1,
                location: Vec3 {
                    x: 0.0,
                    y: 5120.0,
                    z: 321.0,
                },
                direction: Vec3 {
                    x: 0.0,
                    y: -1.0,
                    z: 0.0,
                },
                width: 1786.0,
                height: 642.0,
            }],
        }),
    );
    check_core(
        fixtures::C_TEAM,
        CoreMessage::ControllableTeamInfo(ControllableTeamInfo {
            team: 1,
            controllables: vec![
                ControllableInfo {
                    index: 4,
                    identifier: -1,
                },
                ControllableInfo {
                    index: 5,
                    identifier: 9,
                },
            ],
        }),
    );
}

#[test]
fn disconnect_and_ping_match_the_reference_bytes() {
    check_core(fixtures::C_DISCONNECT, CoreMessage::DisconnectSignal);
    check_core(
        fixtures::C_PING,
        CoreMessage::PingRequest(Ping { cookie: 5 }),
    );
    check_interface(
        fixtures::I_PONG,
        InterfaceMessage::PingResponse(Ping { cookie: 5 }),
    );
}

#[test]
fn a_message_type_that_is_not_modelled_is_reported_not_fatal() {
    // The same payload as the ping fixture, with the tag of a member this crate does not model.
    let mut other = unhex(fixtures::C_PING);
    let tag_at = other.iter().position(|b| *b == 9).unwrap();
    other[tag_at] = 8;
    assert_eq!(
        CoreMessage::from_payload(&other).unwrap(),
        CoreMessage::Other(8)
    );
    assert!(CoreMessage::Other(8).to_payload().is_err());
    // A client message this crate has no member for is an error where a stand-in core reads it.
    let mut other = unhex(fixtures::I_STOP);
    let tag_at = other.iter().position(|b| *b == 10).unwrap();
    other[tag_at] = 6;
    assert!(matches!(
        InterfaceMessage::from_payload(&other),
        Err(Error::UnknownUnion {
            name: "InterfaceMessage",
            tag: 6
        })
    ));
}

#[test]
fn an_enum_value_from_a_newer_protocol_is_an_error_naming_it() {
    let good = InterfaceMessage::MatchConfiguration(config())
        .to_payload()
        .unwrap();
    // Overwrite each byte in turn with an undefined value: exactly the game-mode byte must
    // fail, and as an error that names the enum.
    let named = (0..good.len()).any(|i| {
        let mut bad = good.clone();
        bad[i] = 99;
        InterfaceMessage::from_payload(&bad)
            == Err(Error::UnknownEnum {
                name: "GameMode",
                value: 99,
            })
    });
    assert!(named);
}

#[test]
fn damaged_payloads_never_panic() {
    let fixtures_hex = [
        fixtures::I_CONNECTION,
        fixtures::I_STATE,
        fixtures::I_CONFIG,
        fixtures::C_PACKET,
        fixtures::C_FIELD,
        fixtures::C_TEAM,
    ];
    for hex in fixtures_hex {
        let good = unhex(hex);
        for cut in 0..good.len() {
            let _ = InterfaceMessage::from_payload(&good[..cut]);
            let _ = CoreMessage::from_payload(&good[..cut]);
        }
        for i in 0..good.len() {
            for flip in [0x01u8, 0x80, 0xff] {
                let mut bad = good.clone();
                bad[i] ^= flip;
                let _ = InterfaceMessage::from_payload(&bad);
                let _ = CoreMessage::from_payload(&bad);
            }
        }
    }
}

#[test]
fn messages_cross_a_framed_stream_in_pieces() {
    let a = InterfaceMessage::ConnectionSettings(ConnectionSettings {
        agent_id: "x".into(),
        ..ConnectionSettings::default()
    });
    let b = InterfaceMessage::InitComplete(InitComplete);
    let mut stream = frame(&a.to_payload().unwrap()).unwrap();
    stream.extend(frame(&b.to_payload().unwrap()).unwrap());
    let mut decoder = FrameDecoder::new();
    let mut got = Vec::new();
    for piece in stream.chunks(3) {
        decoder.push(piece);
        while let Some(payload) = decoder.next_frame() {
            got.push(InterfaceMessage::from_payload(&payload).unwrap());
        }
    }
    assert_eq!(got, [a, b]);
}
