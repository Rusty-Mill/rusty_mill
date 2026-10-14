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
    decode_controller_packet, encode_game_packet, BotSkill, Command, GameBundle, GameEvent,
    GameListener, LinkBackend,
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
        let mut initial = opening();
        initial.events.insert(
            1,
            GameEvent::PlayerSpawn {
                id: 4,
                command_id: Some(0),
                name: "Human".into(),
                team: 0,
            },
        );
        send(&mut stream, &initial);
        let mut decoder = FrameDecoder::new();
        let mut buf = [0u8; 1024];
        for i in 1..=2_000u32 {
            send(
                &mut stream,
                &GameBundle {
                    app_seconds: i as f32 / 60.0,
                    delta_physics_frames: 2,
                    events: vec![
                        GameEvent::ActorDestroyed { id: 4 },
                        GameEvent::PlayerPhysics {
                            id: 5,
                            physics: at(i as f32),
                            state: 0,
                        },
                    ],
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
        let mut config = bot_match(&[("bot".to_string(), 0)], MatchLengthMutator::Unlimited);
        config.player_configurations.insert(
            0,
            rb_rlbot_wire::PlayerConfiguration {
                variety: rb_rlbot_wire::PlayerClass::Human,
                team: 0,
                player_id: 42,
            },
        );
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
    assert_eq!(info.controllable_team_info.controllables[0].index, 1);
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
                assert_eq!(p.players.len(), 2);
                assert_eq!(
                    p.players[1].player_id,
                    info.controllable_team_info.controllables[0].identifier
                );
                assert_eq!(p.players[1].player_id, 100);
                assert_eq!(p.players[0].demolished_timeout, 0.0);
                assert_eq!(p.players[1].name, "Bot");
                assert_eq!(p.match_info.match_phase, MatchPhase::Active);
                xs.push(p.players[1].physics.location.x);
                bot.send(PlayerInput {
                    player_index: 1,
                    controller_state: ControllerState {
                        throttle: 1.0,
                        boost: true,
                        ..ControllerState::default()
                    },
                })
                .unwrap();
                // The game closes once it has seen the ball placement, so the bot places the ball
                // only after its third packet: three is then guaranteed, not a race with the close.
                if !sent_ball && xs.len() >= 3 {
                    sent_ball = true;
                    bot.send(DesiredGameState {
                        ball_states: vec![DesiredBallState {
                            physics: DesiredPhysics {
                                location: Some(PartialVec3 {
                                    x: Some(f32::NAN),
                                    y: Some(999.0),
                                    z: None,
                                }),
                                ..Default::default()
                            },
                        }],
                        ..Default::default()
                    })
                    .unwrap();
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

/// A game that starts with a map and no cars, answers each `SpawnBot` with a `PlayerSpawn`
/// carrying its command id, and then moves the new car along, until a player input arrives.
fn spawning_game(addr: SocketAddr, received: Received) -> JoinHandle<()> {
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
        let mut first = opening();
        first
            .events
            .retain(|e| !matches!(e, GameEvent::PlayerSpawn { .. }));
        send(&mut stream, &first);
        let mut decoder = FrameDecoder::new();
        let mut buf = [0u8; 1024];
        let mut pending = Vec::new();
        let mut spawned = false;
        for i in 1..=3_000u32 {
            let mut events = Vec::new();
            for (command_id, name, team) in pending.drain(..) {
                spawned = true;
                events.push(GameEvent::PlayerSpawn {
                    id: 5,
                    command_id: Some(command_id),
                    name,
                    team,
                });
            }
            if spawned {
                events.push(GameEvent::PlayerPhysics {
                    id: 5,
                    physics: at(i as f32),
                    state: 0,
                });
            }
            send(
                &mut stream,
                &GameBundle {
                    app_seconds: i as f32 / 60.0,
                    delta_physics_frames: 2,
                    events,
                },
            );
            if let Ok(n) = stream.read(&mut buf) {
                decoder.push(&buf[..n]);
                while let Some(p) = decoder.next_frame() {
                    for (id, command) in decode_controller_packet(&p).unwrap() {
                        if let Command::SpawnBot { name, team, .. } = &command {
                            pending.push((id, name.clone(), *team));
                        }
                        received.lock().unwrap().push(command);
                    }
                }
            }
            let saw_input = received
                .lock()
                .unwrap()
                .iter()
                .any(|c| matches!(c, Command::PlayerInput { .. }));
            if saw_input {
                break;
            }
            thread::sleep(Duration::from_millis(3));
        }
    })
}

#[test]
fn the_bridge_spawns_the_bots_and_the_match_waits_for_their_cars() {
    let received: Received = Arc::default();
    let game_listener = GameListener::bind(0).unwrap();
    let game = spawning_game(game_listener.local_addr().unwrap(), Arc::clone(&received));

    let (tx, rx) = mpsc::channel();
    let core = thread::spawn(move || {
        let stream = game_listener
            .wait_for_connection(Duration::from_secs(10))
            .unwrap()
            .expect("the game never connected");
        stream.set_nonblocking(true).unwrap();
        let config = bot_match(&[("bot".to_string(), 0)], MatchLengthMutator::Unlimited);
        let backend = LinkBackend::new(stream, config).with_spawning(true);
        let mut core = Core::bind("127.0.0.1:0", backend, Pacing::RealTime).unwrap();
        tx.send(core.local_addr().unwrap()).unwrap();
        core.run(&AtomicBool::new(false), |_| {}).unwrap()
    });

    let mut bot = Connection::connect(rx.recv().unwrap()).unwrap();
    // The handshake completes only once the game has a car for this bot: the opening reply
    // waits for the match, and the match waits for the spawn.
    bot.handshake(ConnectionSettings {
        agent_id: "bot".into(),
        wants_ball_predictions: false,
        wants_comms: false,
        close_between_matches: true,
    })
    .unwrap();
    bot.send(InitComplete).unwrap();
    let mut seen = 0;
    loop {
        match bot.recv().unwrap() {
            CoreMessage::GamePacket(p) => {
                assert_eq!(p.players.len(), 1, "the match began with the car in it");
                assert_eq!(p.players[0].name, "bot");
                seen += 1;
                bot.send(PlayerInput {
                    player_index: 0,
                    controller_state: ControllerState {
                        throttle: 1.0,
                        ..ControllerState::default()
                    },
                })
                .unwrap();
            }
            CoreMessage::DisconnectSignal => break,
            _ => {}
        }
    }
    game.join().unwrap();
    assert_eq!(core.join().unwrap(), RunEnd::GameClosed);
    assert!(seen >= 1);

    let got = received.lock().unwrap();
    let spawns: Vec<&Command> = got
        .iter()
        .filter(|c| matches!(c, Command::SpawnBot { .. }))
        .collect();
    assert_eq!(spawns.len(), 1, "one bot, spawned once: {got:?}");
    assert!(
        matches!(got.first(), Some(Command::SpawnBot { .. })),
        "the spawn came before any input"
    );
    let Command::SpawnBot {
        name,
        skill,
        team,
        loadout,
    } = spawns[0]
    else {
        unreachable!()
    };
    assert_eq!((name.as_str(), *skill, *team), ("bot", BotSkill::Custom, 0));
    assert_eq!(loadout.products[0], 23);
}

/// A game that is killed does not close its socket politely: it resets it. That is the game
/// going away (`GameClosed`), not an error that ends the run. The reset is made the way a dying
/// process makes it: closing a socket with bytes the game never read, here the `SpawnBot` the
/// bridge sends after the map announcement.
#[test]
fn a_game_that_dies_with_unread_bytes_ends_the_run_as_closed_not_as_an_error() {
    let game_listener = GameListener::bind(0).unwrap();
    let addr = game_listener.local_addr().unwrap();
    let game = thread::spawn(move || {
        let mut stream = TcpStream::connect(addr).unwrap();
        let mut first = opening();
        first
            .events
            .retain(|e| !matches!(e, GameEvent::PlayerSpawn { .. }));
        stream
            .write_all(&frame(&encode_game_packet(&first).unwrap()).unwrap())
            .unwrap();
        // The bridge spawns its bots two messages after the map loads; keep the game talking
        // for a few, then die without ever reading the SpawnBot. Closing a socket with unread
        // data sends a reset.
        for i in 1..=5u32 {
            thread::sleep(Duration::from_millis(20));
            let tick = GameBundle {
                app_seconds: i as f32 / 60.0,
                delta_physics_frames: 2,
                events: vec![],
            };
            stream
                .write_all(&frame(&encode_game_packet(&tick).unwrap()).unwrap())
                .unwrap();
        }
        thread::sleep(Duration::from_millis(400));
    });
    let (tx, rx) = mpsc::channel();
    let core = thread::spawn(move || {
        let stream = game_listener
            .wait_for_connection(Duration::from_secs(10))
            .unwrap()
            .expect("the game never connected");
        stream.set_nonblocking(true).unwrap();
        let config = bot_match(&[("bot".to_string(), 0)], MatchLengthMutator::Unlimited);
        let backend = LinkBackend::new(stream, config).with_spawning(true);
        let mut core = Core::bind("127.0.0.1:0", backend, Pacing::RealTime).unwrap();
        let end = core.run(&AtomicBool::new(false), |_| {});
        tx.send(end.map_err(|e| e.to_string())).unwrap();
    });
    let end = rx
        .recv_timeout(Duration::from_secs(10))
        .expect("the run never ended");
    game.join().unwrap();
    core.join().unwrap();
    assert_eq!(end, Ok(RunEnd::GameClosed));
}
