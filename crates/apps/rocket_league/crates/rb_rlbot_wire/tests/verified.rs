//! Encoder output pinned byte for byte. Each payload below was also read back by an independent
//! implementation (`rlbot_flat` 0.6.0 / planus, schema rev c38374e), which returned the values
//! it was built from; a self round trip cannot show that. If an encoder change alters these
//! bytes the test fails, and the new bytes need the same check before the constants change:
//!
//! 1. `cargo test -p rb_rlbot_wire --test verified -- --ignored --nocapture print_cases`
//!    prints `NAME HEX` lines;
//! 2. in a scratch crate depending on `rlbot_flat = "0.6.0"`, read each line with
//!    `InterfacePacketRef`/`CorePacketRef::read_as_root`, convert to the owned type
//!    (`try_from`) and compare the fields.
#![allow(clippy::unwrap_used)]

use rb_rlbot_wire::*;

enum Case {
    Client(InterfaceMessage),
    Core(CoreMessage),
}

impl Case {
    fn payload(&self) -> Vec<u8> {
        match self {
            Case::Client(m) => m.to_payload().unwrap(),
            Case::Core(m) => m.to_payload().unwrap(),
        }
    }
}

fn hex(bytes: &[u8]) -> String {
    bytes.iter().map(|b| format!("{b:02x}")).collect()
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

fn match_info() -> MatchInfo {
    MatchInfo {
        seconds_elapsed: 12.5,
        game_time_remaining: 287.5,
        is_overtime: true,
        is_unlimited_time: false,
        match_phase: MatchPhase::GoalScored,
        frame_num: 1500,
    }
}

fn cases() -> Vec<(&'static str, Case)> {
    let pad = BoostPad {
        location: Vec3 {
            x: -3072.0,
            y: 4096.0,
            z: 73.0,
        },
        is_full_boost: true,
    };
    vec![
        (
            "connection",
            Case::Client(InterfaceMessage::ConnectionSettings(ConnectionSettings {
                agent_id: "rusty/bot1".into(),
                wants_ball_predictions: true,
                wants_comms: false,
                close_between_matches: true,
            })),
        ),
        (
            "stop",
            Case::Client(InterfaceMessage::StopCommand(StopCommand {
                shutdown_server: true,
            })),
        ),
        (
            "init",
            Case::Client(InterfaceMessage::InitComplete(InitComplete)),
        ),
        (
            "input",
            Case::Client(InterfaceMessage::PlayerInput(PlayerInput {
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
            })),
        ),
        (
            "state",
            Case::Client(InterfaceMessage::DesiredGameState(DesiredGameState {
                ball_states: vec![DesiredBallState {
                    physics: DesiredPhysics {
                        location: Some(PartialVec3 {
                            x: Some(1.0),
                            y: Some(2.0),
                            z: Some(3.0),
                        }),
                        ..DesiredPhysics::default()
                    },
                }],
                car_states: vec![DesiredCarState {
                    physics: None,
                    boost_amount: Some(33.0),
                }],
                match_info: Some(DesiredMatchInfo {
                    world_gravity_z: Some(-650.0),
                    game_speed: None,
                }),
            })),
        ),
        (
            "config",
            Case::Client(InterfaceMessage::MatchConfiguration(MatchConfiguration {
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
                enable_state_setting: false,
                auto_start_agents: false,
                freeplay: true,
                ..MatchConfiguration::default()
            })),
        ),
        (
            "packet_empty",
            Case::Core(CoreMessage::GamePacket(GamePacket {
                players: vec![],
                boost_pads: vec![],
                balls: vec![],
                match_info: match_info(),
                teams: vec![],
            })),
        ),
        (
            "packet_full",
            Case::Core(CoreMessage::GamePacket(GamePacket {
                players: vec![
                    PlayerInfo {
                        physics: physics(10.0),
                        air_state: AirState::DoubleJumping,
                        demolished_timeout: 1.25,
                        is_bot: true,
                        name: "tape".into(),
                        team: 0,
                        boost: 66.0,
                        player_id: 7,
                    },
                    PlayerInfo {
                        physics: physics(20.0),
                        air_state: AirState::OnGround,
                        demolished_timeout: -1.0,
                        is_bot: false,
                        name: String::new(),
                        team: 1,
                        boost: 0.0,
                        player_id: -2,
                    },
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
                ],
                balls: vec![
                    BallInfo {
                        physics: physics(3.0),
                    },
                    BallInfo {
                        physics: physics(4.0),
                    },
                ],
                match_info: match_info(),
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
            })),
        ),
        (
            "field_empty",
            Case::Core(CoreMessage::FieldInfo(FieldInfo::default())),
        ),
        (
            "field_full",
            Case::Core(CoreMessage::FieldInfo(FieldInfo {
                boost_pads: vec![
                    BoostPad {
                        location: Vec3 {
                            x: 0.0,
                            y: -4240.0,
                            z: 70.0,
                        },
                        is_full_boost: false,
                    },
                    pad,
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
            })),
        ),
        (
            "team",
            Case::Core(CoreMessage::ControllableTeamInfo(ControllableTeamInfo {
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
            })),
        ),
        (
            "core_config",
            Case::Core(CoreMessage::MatchConfiguration(MatchConfiguration {
                game_map_upk: "Stadium_P".into(),
                player_configurations: vec![PlayerConfiguration {
                    variety: PlayerClass::Human,
                    team: 1,
                    player_id: 3,
                }],
                ..MatchConfiguration::default()
            })),
        ),
    ]
}

#[test]
#[ignore = "prints the payloads for re-verification; see the file header"]
fn print_cases() {
    for (name, case) in cases() {
        println!("CASE {name} {}", hex(&case.payload()));
    }
}

#[test]
fn encoder_output_matches_the_independently_verified_bytes() {
    let pinned: Vec<(&str, &str)> = PINNED.to_vec();
    let all = cases();
    assert_eq!(pinned.len(), all.len(), "every case is pinned");
    for ((name, case), (pinned_name, pinned_hex)) in all.iter().zip(&pinned) {
        assert_eq!(name, pinned_name);
        assert_eq!(
            &hex(&case.payload()),
            pinned_hex,
            "{name}: bytes changed; re-verify, then update"
        );
    }
}

const PINNED: &[(&str, &str)] = &[
    ("connection",
     "0c00000008000c000b0004000800000014000000000000090c000c0008000700000006000c00000000000101040000000a00000072757374792f626f74310000"),
    ("stop",
     "0c00000008000a0009000400080000000c000000000a0600080007000600000000000001"),
    ("init",
     "0c00000008000c000b000400080000000c0000000000000c0400040004000000"),
    ("input",
     "0c00000008000c000b000400080000001000000000000004080020001c000400080000000000803f000000bf0000803e00000000000080bf0101000003000000"),
    ("state",
     "0c00000008000a0009000400080000001000000000050a0012000c00080004000a00000014000000180000002c000000000006000800040006000000008022c4010000000c00000008000800000004000800000000000442010000000c000000000006000a000400060000000c000000000006000a000400060000001000000000000a0010000c00080004000a00000000004040000000400000803f"),
    ("config",
     "0c00000008000c000b000400080000002c0000000000000324002600250020001f0000001800140010000f000000000008000000000007000000060024000000000001002400000000000001240000002400000044010000000000004c01000000010600080007000600000000000003000000000300000078000000380000001000000000000a0010000f00080004000a000000010000000c0000000000000104000400040000000c00120011000c00080004000c000000fdffffff010000001000000000030a000c000800000007000a00000000000003040000000300000050726f000c000e000d000800000004000c0000000700000018000000000212001c0018001400100000000c000b000400120000001800000000000001400000004c0000005c00000064000000010000000c00000008000c000800040008000000080000000c00000001000000310000000700000052425f54415045000900000072622f746170652f300000000f00000072625f746170655f626f742e6578650007000000433a2f626f747300040000007461706500000000090000005374616469756d5f500000000000000000000000"),
    ("packet_empty",
     "0c00000008000c000b00040008000000180000000000000210001c001800140010000c000800040010000000180000001800000030000000400000004000000040000000000000000000000000001600140010000c000b0000000a00000000000000040016000000dc0500000000040100c08f4300004841000000000000000000000000"),
    ("packet_full",
     "0c00000008000c000b00040008000000180000000000000210001c001800140010000c00080004001000000018000000180000004000000050000000080100001801000000000000020000000000000002000000010000000500000000001600140010000c000b0000000a00000000000000040016000000dc0500000000040100c08f4300004841020000006c0000001000000000000a003e000c000b0004000a0000004000000000000002000080400000a0400000c040cdcccc3ecdcc4c3f9a99993f000080c0000000410000003f000000000000000000008040000006000a000400060000000080364300000a003e000c000b0004000a000000400000000000000200004040000080400000a0409a99993e9a99193f6766663f000040c00000c0400000003f00000000000000000000404000000600080004000600000000803643020000000100000000000000000000000000904002000000200100003400000000002e009400640048004400380000000000000034000000000030002c000000280024000c00000000000000000004002e000000000000000000000000000000000000000000000000000000000000000000000070000000feffffff0100000084000000000080bf7b145e41000000000000a64160000000000000000000000000000000000000000000000000000000000000000000a0410000a8410000b04100000040000080400000c0400000a0c1000020420000003f00000000000000000000a0410000000000000a0010000c00080004000a000000cdcc10426666a8420000ec420000000000002e009c006c0050004c00400000003f000000380000003700300000002c00280024000c00000000000000000004002e00000000000000000000000000000000000000000000000000000000000000000000007800000007000000000084428c000000000000010000a03f000000027b145e41000000000000a64160000000000000000000000000000000000000000000000000000000000000000000204100003041000040410000803f0000004000004040000020c10000a0410000003f0000000000000000000020410000000000000a0010000c00080004000a000000cdcc10426666a8420000ec42040000007461706500000000"),
    ("field_empty",
     "0c00000008000a0009000400080000001000000000030a0010000c00080004000a0000000c0000000c0000000c000000000000000000000000000000"),
    ("field_full",
     "0c00000008000a0009000400080000001000000000030a0010000c00080004000a0000000c0000000c0000004800000000000000010000001400000000000e002800240018000c00080004000e000000008020440040df4400000000000080bf00000000000000000000a0450080a04301000000020000002c0000000c00000008001600080007000800000000000001000040c5000080450000924200000600100004000600000000000000008084c500008c42"),
    ("team",
     "0c00000008000c000b00040008000000100000000000000708000c000800040008000000080000000100000002000000240000000c00000008000c000800040008000000090000000500000008000c000800040008000000ffffffff04000000"),
    ("core_config",
     "0c00000008000a0009000400080000001800000000041200140000001000000000000c00080004001200000010000000100000003c000000480000000000000001000000100000000c00140013000c00080004000c00000003000000010000000c000000000000010400040004000000090000005374616469756d5f500000000000000000000000"),
];
