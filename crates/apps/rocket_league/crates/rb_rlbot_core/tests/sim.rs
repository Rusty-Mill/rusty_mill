//! The simulation without sockets: setup, what core hands out, and what a client can change.
#![allow(clippy::unwrap_used, clippy::expect_used, clippy::panic)]

use rb_rlbot_core::{SetupError, Simulation};
use rb_rlbot_wire::{
    ControllerState, CustomBot, DesiredBallState, DesiredCarState, DesiredGameState,
    DesiredPhysics, MatchConfiguration, MatchLengthMutator, MatchPhase, MutatorSettings,
    PartialRotator, PartialVec3, PlayerClass, PlayerConfiguration,
};

pub fn bot(agent_id: &str, team: u32, player_id: i32) -> PlayerConfiguration {
    PlayerConfiguration {
        variety: PlayerClass::CustomBot(CustomBot {
            name: format!("bot {player_id}"),
            root_dir: String::new(),
            run_command: String::new(),
            agent_id: agent_id.into(),
            hivemind: false,
            environment: None,
        }),
        team,
        player_id,
    }
}

fn config(players: Vec<PlayerConfiguration>) -> MatchConfiguration {
    MatchConfiguration {
        player_configurations: players,
        ..MatchConfiguration::default()
    }
}

fn two_bots() -> Simulation {
    Simulation::new(config(vec![bot("a", 0, 100), bot("b", 1, 101)])).unwrap()
}

#[test]
fn a_match_needs_players_on_team_zero_or_one() {
    assert_eq!(
        Simulation::new(config(vec![])).err(),
        Some(SetupError::NoPlayers)
    );
    assert_eq!(
        Simulation::new(config(vec![bot("a", 0, 1), bot("b", 2, 2)])).err(),
        Some(SetupError::BadTeam { player: 1, team: 2 })
    );
}

#[test]
fn an_agent_controls_the_cars_configured_with_its_id() {
    let sim = Simulation::new(config(vec![
        bot("hive", 1, 7),
        bot("solo", 0, 8),
        bot("hive", 1, 9),
    ]))
    .unwrap();
    let hive = sim.controllable_team_info("hive");
    assert_eq!(hive.team, 1);
    let cars: Vec<(u32, i32)> = hive
        .controllables
        .iter()
        .map(|c| (c.index, c.identifier))
        .collect();
    assert_eq!(cars, [(0, 7), (2, 9)]);
    assert_eq!(sim.controllable_team_info("solo").controllables.len(), 1);
    assert!(sim
        .controllable_team_info("nobody")
        .controllables
        .is_empty());
    assert_eq!(sim.expected_agents(), ["hive", "solo"]);
}

#[test]
fn field_info_has_the_34_pads_and_the_two_goals() {
    let field = two_bots().field_info();
    assert_eq!(field.boost_pads.len(), 34);
    assert_eq!(
        field.boost_pads.iter().filter(|p| p.is_full_boost).count(),
        6
    );
    let goals: Vec<(i32, f32, f32)> = field
        .goals
        .iter()
        .map(|g| (g.team_num, g.location.y, g.direction.y))
        .collect();
    // Each goal is named for the team defending it and points into the field.
    assert_eq!(goals, [(0, -5120.0, 1.0), (1, 5120.0, -1.0)]);
    // Width, height and centre height as the real game announces them.
    assert_eq!(
        (
            field.goals[0].width,
            field.goals[0].height,
            field.goals[0].location.z
        ),
        (1920.0, 752.0, 312.0)
    );
}

#[test]
fn the_first_packet_is_a_countdown_with_every_car_and_the_ball() {
    let packet = two_bots().packet();
    assert_eq!(packet.match_info.match_phase, MatchPhase::Countdown);
    assert_eq!(packet.match_info.frame_num, 0);
    assert!(!packet.match_info.is_unlimited_time);
    assert_eq!(packet.players.len(), 2);
    assert_eq!(
        packet.players.iter().map(|p| p.team).collect::<Vec<_>>(),
        [0, 1]
    );
    assert_eq!(packet.players[1].player_id, 101);
    assert!(packet.players.iter().all(|p| p.is_bot));
    assert_eq!(packet.boost_pads.len(), 34);
    assert!(packet.boost_pads.iter().all(|p| p.is_active));
    assert_eq!((packet.balls.len(), packet.teams.len()), (1, 2));
    // Blue starts on its own half, facing the ball (orange's goal is at +y).
    assert!(packet.players[0].physics.location.y < 0.0);
    assert!(packet.players[1].physics.location.y > 0.0);
}

#[test]
fn match_length_sets_the_clock_and_unlimited_has_none() {
    let mut cfg = config(vec![bot("a", 0, 1)]);
    cfg.mutators = Some(MutatorSettings {
        match_length: MatchLengthMutator::TenMinutes,
    });
    let packet = Simulation::new(cfg.clone()).unwrap().packet();
    assert_eq!(packet.match_info.game_time_remaining, 600.0);
    cfg.mutators = Some(MutatorSettings {
        match_length: MatchLengthMutator::Unlimited,
    });
    let packet = Simulation::new(cfg).unwrap().packet();
    assert!(packet.match_info.is_unlimited_time);
}

#[test]
fn stepping_counts_frames_and_the_countdown_ignores_input() {
    let mut sim = two_bots();
    let before = sim.packet().players[0].physics.location;
    let drive = ControllerState {
        throttle: 1.0,
        boost: true,
        ..ControllerState::default()
    };
    assert!(sim.set_input(0, &drive));
    assert!(!sim.set_input(5, &drive), "no such car");
    for _ in 0..30 {
        sim.step();
    }
    let packet = sim.packet();
    assert_eq!(packet.match_info.frame_num, 30);
    assert_eq!(packet.match_info.match_phase, MatchPhase::Countdown);
    let after = packet.players[0].physics.location;
    assert!(
        (after.x - before.x).hypot(after.y - before.y) < 1.0,
        "the car drove during the countdown"
    );
}

#[test]
fn state_setting_changes_only_the_named_fields() {
    let mut sim = two_bots();
    let ball_before = sim.packet().balls[0].physics;
    let car_before = sim.packet().players[1].physics;
    let unknown = sim.apply_desired(&DesiredGameState {
        ball_states: vec![DesiredBallState {
            physics: DesiredPhysics {
                location: Some(PartialVec3 {
                    x: Some(1000.0),
                    y: None,
                    z: Some(500.0),
                }),
                velocity: Some(PartialVec3 {
                    x: None,
                    y: Some(300.0),
                    z: None,
                }),
                ..DesiredPhysics::default()
            },
        }],
        car_states: vec![
            DesiredCarState::default(),
            DesiredCarState {
                physics: Some(DesiredPhysics {
                    rotation: Some(PartialRotator {
                        pitch: None,
                        yaw: Some(1.0),
                        roll: None,
                    }),
                    ..DesiredPhysics::default()
                }),
                boost_amount: Some(250.0),
            },
            DesiredCarState::default(),
        ],
        match_info: None,
    });
    assert!(unknown.unwrap(), "the third car does not exist");
    let packet = sim.packet();
    let ball = packet.balls[0].physics;
    assert_eq!(ball.location.x, 1000.0);
    assert_eq!(ball.location.z, 500.0);
    assert_eq!(ball.location.y, ball_before.location.y, "y was not named");
    assert_eq!(ball.velocity.y, 300.0);
    let car = &packet.players[1];
    assert!((car.physics.rotation.yaw - 1.0).abs() < 1e-3);
    assert_eq!(car.boost, 100.0, "boost is clamped to a full tank");
    assert_eq!(car.physics.location, car_before.location);
}

#[test]
fn the_same_inputs_give_the_same_packets() {
    let drive = ControllerState {
        throttle: 1.0,
        steer: 0.3,
        boost: true,
        ..ControllerState::default()
    };
    let run = || {
        let mut sim = two_bots();
        sim.set_input(0, &drive);
        (0..900)
            .map(|_| {
                sim.step();
                sim.packet()
            })
            .last()
            .unwrap()
    };
    assert_eq!(run(), run());
}

#[test]
fn a_blank_agent_id_is_refused_and_a_padded_one_matches_after_trimming() {
    assert_eq!(
        Simulation::new(config(vec![bot("  ", 0, 1)])).err(),
        Some(SetupError::BlankAgentId { player: 0 })
    );
    // Core trims the id a client sends, so a configured id with spaces must match the trimmed one.
    let sim = Simulation::new(config(vec![bot(" a ", 0, 1), bot("a", 1, 2)])).unwrap();
    assert_eq!(sim.expected_agents(), ["a"]);
    assert_eq!(sim.controllable_team_info("a").controllables.len(), 2);
    assert_eq!(sim.controllable_team_info(" a ").controllables.len(), 2);
}

fn all_finite(packet: &rb_rlbot_wire::GamePacket) -> bool {
    let v = |p: &rb_rlbot_wire::Physics| {
        [
            p.location.x,
            p.location.y,
            p.location.z,
            p.velocity.x,
            p.velocity.y,
            p.velocity.z,
            p.angular_velocity.x,
            p.angular_velocity.y,
            p.angular_velocity.z,
            p.rotation.pitch,
            p.rotation.yaw,
            p.rotation.roll,
        ]
        .iter()
        .all(|x| x.is_finite())
    };
    packet.balls.iter().all(|b| v(&b.physics))
        && packet
            .players
            .iter()
            .all(|p| v(&p.physics) && p.boost.is_finite())
}

#[test]
fn numbers_that_are_not_finite_cannot_poison_the_match() {
    let mut sim = two_bots();
    let before = sim.packet();
    let rejected = sim.apply_desired(&DesiredGameState {
        ball_states: vec![DesiredBallState {
            physics: DesiredPhysics {
                location: Some(PartialVec3 {
                    x: Some(f32::NAN),
                    y: Some(123.0),
                    z: None,
                }),
                rotation: Some(PartialRotator {
                    pitch: Some(f32::NAN),
                    yaw: None,
                    roll: Some(f32::NEG_INFINITY),
                }),
                ..DesiredPhysics::default()
            },
        }],
        car_states: vec![DesiredCarState {
            physics: None,
            boost_amount: Some(90.0),
        }],
        match_info: None,
    });
    assert_eq!(
        rejected.unwrap_err().kind(),
        std::io::ErrorKind::InvalidInput
    );
    let after = sim.packet();
    assert_eq!(after, before);
    assert!(all_finite(&after));
    assert_eq!(
        after.balls[0].physics.location,
        before.balls[0].physics.location
    );
    assert_eq!(after.players[0].boost, before.players[0].boost);

    while sim.packet().match_info.match_phase != MatchPhase::Active {
        sim.step();
    }
    // Wild sticks are held to the legal range, and a NaN is no input; the car just sits.
    let wild = ControllerState {
        throttle: f32::NAN,
        steer: 1.0e30,
        pitch: f32::INFINITY,
        yaw: f32::NAN,
        roll: -1.0e30,
        ..ControllerState::default()
    };
    assert!(sim.set_input(0, &wild));
    for _ in 0..30 {
        sim.step();
    }
    assert!(all_finite(&sim.packet()));
}
