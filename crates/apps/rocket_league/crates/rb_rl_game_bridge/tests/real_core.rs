//! The whole chain: a stand-in Rocket League dials in, `rb_rlbot_core::Core` serves a real RLBot
//! client from it through `LinkBackend`, and the bot's input and state setting reach the game.
#![allow(clippy::unwrap_used, clippy::expect_used, clippy::panic)]

use std::io::{Read, Write};
use std::net::{SocketAddr, TcpStream};
use std::sync::atomic::AtomicBool;
use std::sync::mpsc;
use std::sync::{Arc, Mutex};
use std::thread::{self, JoinHandle};
use std::time::Duration;

use rb_rl_game_bridge::{
    decode_controller_packet, encode_game_packet, Command, GameBundle, GameEvent, GameListener,
    LinkBackend,
};
use rb_rlbot_client::Connection;
use rb_rlbot_core::{bot_match, Core, Event, Pacing, RunEnd};
use rb_rlbot_wire::frame::{frame, FrameDecoder};
use rb_rlbot_wire::{
    ConnectionSettings, ControllerState, CoreMessage, DesiredBallState, DesiredGameState,
    DesiredPhysics, InitComplete, MatchLengthMutator, MatchPhase, PartialVec3, Physics,
    PlayerInput, Vec3,
};

fn at(x: f32) -> Physics {
    Physics {
        location: Vec3 { x, y: 0.0, z: 17.0 },
        ..Physics::default()
    }
}

fn opening() -> GameBundle {
    let pad = |id, x, y, big| GameEvent::PickupSpawn {
        id,
        location: Vec3 { x, y, z: 0.0 },
        big,
    };
    GameBundle {
        app_seconds: 0.0,
        delta_physics_frames: 0,
        events: vec![
            GameEvent::GameSpawn {
                map_name: "Stadium_P".into(),
                mode: 1,
                team_size: 1,
                gravity: None,
            },
            pad(2, 100.0, 4000.0, false),
            pad(1, -3584.0, 0.0, true),
            GameEvent::GoalSpawn {
                id: 90,
                team: 1,
                center: Vec3 {
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
            },
            GameEvent::PlayerSpawn {
                id: 5,
                command_id: None,
                name: "Bot".into(),
                team: 0,
            },
            GameEvent::BallSpawn { id: 6 },
            GameEvent::StateChange {
                phase: MatchPhase::Active,
                is_overtime: false,
            },
        ],
    }
}

/// What the stand-in game saw RLBot send it.
type Received = Arc<Mutex<Vec<Command>>>;

/// Dials `addr`, sends the opening and then a bundle every few milliseconds with the car
/// further along, until RLBot has sent both a player input and a ball placement (or ~10 s),
/// then closes the socket.
fn fake_game(addr: SocketAddr, received: Received) -> JoinHandle<()> {
    thread::spawn(move || {
        let mut stream = TcpStream::connect(addr).unwrap();
        stream
            .set_read_timeout(Some(Duration::from_millis(1)))
            .unwrap();
        let send = |stream: &mut TcpStream, b: &GameBundle| {
            stream
                .write_all(&frame(&encode_game_packet(b).unwrap()).unwrap())
                .unwrap();
        };
        send(&mut stream, &opening());
        let mut decoder = FrameDecoder::new();
        let mut buf = [0u8; 1024];
        for i in 1..=2_000u32 {
            send(
                &mut stream,
                &GameBundle {
                    app_seconds: i as f32 / 60.0,
                    delta_physics_frames: 2,
                    events: vec![GameEvent::PlayerPhysics {
                        id: 5,
                        physics: at(i as f32),
                        state: 0,
                    }],
                },
            );
            if let Ok(n) = stream.read(&mut buf) {
                decoder.push(&buf[..n]);
                while let Some(p) = decoder.next_frame() {
                    let mut got = received.lock().unwrap();
                    got.extend(
                        decode_controller_packet(&p)
                            .unwrap()
                            .into_iter()
                            .map(|c| c.1),
                    );
                }
            }
            let got = received.lock().unwrap();
            let saw_input = got.iter().any(|c| matches!(c, Command::PlayerInput { .. }));
            let saw_ball = got
                .iter()
                .any(|c| matches!(c, Command::SetPhysics { actor_id: 6, .. }));
            drop(got);
            if saw_input && saw_ball {
                break;
            }
            thread::sleep(Duration::from_millis(5));
        }
        // Dropping the stream closes it: the core must notice and tell the bot.
    })
}

#[test]
fn a_bot_plays_through_core_in_a_game_that_is_not_the_simulation() {
    let received: Received = Arc::default();
    let game_listener = GameListener::bind(0).unwrap();
    let game_addr = game_listener.local_addr().unwrap();
    let game = fake_game(game_addr, Arc::clone(&received));

    let (tx, rx) = mpsc::channel();
    let events = Arc::new(Mutex::new(Vec::new()));
    let sink = Arc::clone(&events);
    let core = thread::spawn(move || {
        let stream = game_listener
            .wait_for_connection(Duration::from_secs(10))
            .unwrap()
            .expect("the game never connected");
        stream.set_nonblocking(true).unwrap();
        let config = bot_match(&[("bot".to_string(), 0)], MatchLengthMutator::Unlimited);
        let mut core = Core::bind(
            "127.0.0.1:0",
            LinkBackend::new(stream, config),
            Pacing::RealTime,
        )
        .unwrap();
        tx.send(core.local_addr().unwrap()).unwrap();
        core.run(&AtomicBool::new(false), |e| sink.lock().unwrap().push(e))
            .unwrap()
    });

    let mut bot = Connection::connect(rx.recv().unwrap()).unwrap();
    let info = bot
        .handshake(ConnectionSettings {
            agent_id: "bot".into(),
            wants_ball_predictions: false,
            wants_comms: false,
            close_between_matches: true,
        })
        .unwrap();
    assert_eq!(info.controllable_team_info.controllables[0].index, 0);
    // The field is what the game announced, not the simulated arena's 34 pads.
    assert_eq!(info.field_info.boost_pads.len(), 2);
    assert_eq!(
        info.field_info.boost_pads[0].location.y, 0.0,
        "pads are ordered by y"
    );
    assert!(info.field_info.boost_pads[0].is_full_boost);
    assert_eq!(info.field_info.goals.len(), 1);
    bot.send(InitComplete).unwrap();

    let mut xs = Vec::new();
    let mut disconnected = false;
    let mut sent_ball = false;
    while !disconnected {
        match bot.recv().unwrap() {
            CoreMessage::GamePacket(p) => {
                assert_eq!(p.players[0].name, "Bot");
                assert_eq!(p.match_info.match_phase, MatchPhase::Active);
                xs.push(p.players[0].physics.location.x);
                bot.send(PlayerInput {
                    player_index: 0,
                    controller_state: ControllerState {
                        throttle: 1.0,
                        boost: true,
                        ..ControllerState::default()
                    },
                })
                .unwrap();
                if !sent_ball && xs.len() > 1 {
                    sent_ball = true;
                    bot.send(DesiredGameState {
                        ball_states: vec![DesiredBallState {
                            physics: DesiredPhysics {
                                location: Some(PartialVec3 {
                                    x: Some(10.0),
                                    y: Some(20.0),
                                    z: Some(500.0),
                                }),
                                ..DesiredPhysics::default()
                            },
                        }],
                        car_states: vec![],
                        match_info: None,
                    })
                    .unwrap();
                }
            }
            CoreMessage::DisconnectSignal => disconnected = true,
            _ => {}
        }
    }
    game.join().unwrap();
    let end = core.join().unwrap();

    // The game closing its socket ends the run, and the bot was told.
    assert_eq!(end, RunEnd::GameClosed);
    // Packets mirrored the game's car moving along, in order.
    assert!(xs.len() >= 3, "{} packets", xs.len());
    assert!(xs.windows(2).all(|w| w[1] > w[0]), "{xs:?}");
    // The bot's input reached the game for the right actor, as the game takes it.
    let got = received.lock().unwrap();
    let input = got
        .iter()
        .find_map(|c| match c {
            Command::PlayerInput { actor_id, input } => Some((*actor_id, *input)),
            _ => None,
        })
        .expect("no input reached the game");
    assert_eq!(input.0, 5);
    assert!(input.1.boost && input.1.throttle == 1.0);
    // State setting became a placement of the ball (actor 6) with the unnamed fields kept.
    let ball = got
        .iter()
        .find_map(|c| match c {
            Command::SetPhysics {
                actor_id: 6,
                physics,
            } => Some(*physics),
            _ => None,
        })
        .expect("the ball was never placed");
    assert_eq!(
        (ball.location.x, ball.location.y, ball.location.z),
        (10.0, 20.0, 500.0)
    );
    let events = events.lock().unwrap();
    assert!(events.contains(&Event::MatchStarted));
}
