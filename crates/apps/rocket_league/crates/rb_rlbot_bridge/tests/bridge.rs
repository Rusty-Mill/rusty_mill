//! The two conversions and the bot's per-packet decision.
#![allow(clippy::unwrap_used, clippy::expect_used, clippy::panic)]

use rb_domain::{ControllerInput, PhysicsFrame, Vec3 as DVec3};
use rb_env::{chaser::chase, Env};
use rb_rlbot_bridge::{controller_state, observation, PolicyBot};
use rb_rlbot_wire::{
    AirState, BallInfo, GamePacket, MatchInfo, MatchPhase, Physics, PlayerInfo, Rotator, Vec3,
};

fn at(x: f32, y: f32, z: f32) -> Physics {
    Physics {
        location: Vec3 { x, y, z },
        ..Physics::default()
    }
}

fn car(physics: Physics, team: u32, boost: f32) -> PlayerInfo {
    PlayerInfo {
        physics,
        air_state: AirState::OnGround,
        demolished_timeout: 0.0,
        is_bot: true,
        name: String::new(),
        team,
        boost,
        player_id: 7,
    }
}

fn packet(players: Vec<PlayerInfo>, ball: Option<Physics>) -> GamePacket {
    GamePacket {
        players,
        boost_pads: Vec::new(),
        balls: ball
            .map(|physics| BallInfo { physics })
            .into_iter()
            .collect(),
        match_info: MatchInfo {
            seconds_elapsed: 12.5,
            game_time_remaining: 287.5,
            is_overtime: false,
            is_unlimited_time: false,
            match_phase: MatchPhase::Active,
            frame_num: 1500,
        },
        teams: Vec::new(),
    }
}

/// Two cars, the second facing +y (yaw a quarter turn), the ball moving.
fn two_cars() -> GamePacket {
    let mut second = at(100.0, -2000.0, 17.0);
    second.rotation = Rotator {
        pitch: 0.0,
        yaw: std::f32::consts::FRAC_PI_2,
        roll: 0.0,
    };
    second.velocity = Vec3 {
        x: 0.0,
        y: 900.0,
        z: 0.0,
    };
    second.angular_velocity = Vec3 {
        x: 0.1,
        y: 0.2,
        z: 0.3,
    };
    let mut ball = at(0.0, 0.0, 93.0);
    ball.velocity = Vec3 {
        x: 5.0,
        y: 6.0,
        z: 7.0,
    };
    packet(
        vec![car(at(-50.0, 40.0, 17.0), 0, 33.0), car(second, 1, 100.0)],
        Some(ball),
    )
}

#[test]
fn an_observation_carries_every_field_in_car_order() {
    let frame = observation(&two_cars()).unwrap();
    assert_eq!(frame.timestamp_secs, 12.5);
    assert_eq!(frame.ball.position, DVec3::new(0.0, 0.0, 93.0));
    assert_eq!(frame.ball.velocity, DVec3::new(5.0, 6.0, 7.0));
    assert_eq!(frame.cars.len(), 2);
    assert_eq!(frame.cars[0].position, DVec3::new(-50.0, 40.0, 17.0));
    assert_eq!(frame.cars[0].boost_amount, 33.0);
    let second = frame.cars[1];
    assert_eq!(second.position, DVec3::new(100.0, -2000.0, 17.0));
    assert_eq!(second.velocity, DVec3::new(0.0, 900.0, 0.0));
    assert_eq!(second.angular_velocity, DVec3::new(0.1, 0.2, 0.3));
    assert_eq!(second.boost_amount, 100.0);
    // `player_id` is the car's place in the list, not the game's id (7 above).
    assert_eq!([frame.cars[0].player_id, second.player_id], [0, 1]);
    assert_eq!(second.input, None);
}

#[test]
fn a_quarter_turn_of_yaw_points_the_nose_along_plus_y() {
    let frame = observation(&two_cars()).unwrap();
    let nose = frame.cars[1].rotation.rotate(&DVec3::new(1.0, 0.0, 0.0));
    assert!(nose.x.abs() < 1e-5 && (nose.y - 1.0).abs() < 1e-5 && nose.z.abs() < 1e-5);
    let level = frame.cars[0].rotation.rotate(&DVec3::new(1.0, 0.0, 0.0));
    assert!((level.x - 1.0).abs() < 1e-5);
}

#[test]
fn a_packet_without_a_ball_has_no_observation() {
    assert_eq!(
        observation(&packet(vec![car(at(0.0, 0.0, 17.0), 0, 0.0)], None)),
        None
    );
}

#[test]
fn unset_stick_axes_are_centred_and_the_rest_carried() {
    let input = ControllerInput {
        throttle: -0.5,
        steer: 0.25,
        pitch: Some(-1.0),
        yaw: None,
        roll: Some(0.75),
        jump: true,
        boost: true,
        handbrake: true,
    };
    let state = controller_state(&input);
    assert_eq!(
        (
            state.throttle,
            state.steer,
            state.pitch,
            state.yaw,
            state.roll
        ),
        (-0.5, 0.25, -1.0, 0.0, 0.75)
    );
    assert!(state.jump && state.boost && state.handbrake && !state.use_item);
    assert_eq!(
        controller_state(&ControllerInput::default()),
        Default::default()
    );
}

#[test]
fn a_bot_answers_for_its_own_car_only() {
    let seen = std::cell::RefCell::new(Vec::new());
    let policy = |frame: &PhysicsFrame, car: usize| {
        seen.borrow_mut().push((car, frame.cars.len()));
        ControllerInput {
            steer: car as f32,
            ..ControllerInput::default()
        }
    };
    let mut bot = PolicyBot::new(policy, 1);
    let input = bot.input(&two_cars()).unwrap();
    assert_eq!(input.player_index, 1);
    assert_eq!(input.controller_state.steer, 1.0);
    assert_eq!(*seen.borrow(), vec![(1, 2)]);
}

#[test]
fn a_bot_stays_quiet_without_a_ball_or_without_its_car() {
    let policy = |_: &PhysicsFrame, _: usize| -> ControllerInput { panic!("asked to act") };
    let mut bot = PolicyBot::new(policy, 1);
    let one_car = packet(
        vec![car(at(0.0, 0.0, 17.0), 0, 0.0)],
        Some(at(0.0, 0.0, 93.0)),
    );
    assert_eq!(bot.input(&one_car), None);
    let no_ball = packet(vec![car(at(0.0, 0.0, 17.0), 0, 0.0); 2], None);
    assert_eq!(bot.input(&no_ball), None);
}

/// A game packet is something `rb_env` accepts and a policy built on its types can act on: the
/// chaser, facing the point it wants to reach, drives straight and boosts. A wrong rotator
/// convention would turn the nose and make it steer.
#[test]
fn a_game_observation_feeds_rb_env_and_the_chaser() {
    let frame = observation(&two_cars()).unwrap();
    let mut env = Env::new();
    let reset = env.reset(&frame);
    assert_eq!(reset.cars.len(), 2);
    assert_eq!(reset.cars[1].position, frame.cars[1].position);
    let stepped = env.step(&[ControllerInput::default(); 2]);
    assert!(stepped.timestamp_secs > frame.timestamp_secs);

    // Car 1 sits behind the ball on the side blue attacks from and faces +y, toward it.
    let drive = chase(&frame.cars[1], &frame.ball, 0);
    assert_eq!(drive.throttle, 1.0);
    assert!(drive.boost && drive.steer.abs() < 0.2, "{drive:?}");
}
