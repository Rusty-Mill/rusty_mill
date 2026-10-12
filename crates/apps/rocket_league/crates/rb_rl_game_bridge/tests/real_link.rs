//! `RocketLeagueLink` and the state fold, against a stand-in game that speaks the game socket.
#![allow(clippy::unwrap_used, clippy::expect_used, clippy::panic)]

use std::io::{self, Cursor, Read, Write};
use std::net::TcpStream;
use std::thread;
use std::time::Duration;

use rb_rl_game_bridge::{
    decode_controller_packet, encode_game_packet, BotSkill, BridgeLoop, Command, GameBundle,
    GameCommand, GameEvent, GameLink, GameListener, Loadout, RocketLeagueLink, Roster, SEND_BUDGET,
};
use rb_rlbot_wire::frame::{frame, FrameDecoder};
use rb_rlbot_wire::{AirState, ControllerState, MatchPhase, Physics, Vec3};

/// A stream that reads from a script and records what is written.
struct Duplex {
    input: Cursor<Vec<u8>>,
    output: Vec<u8>,
}

impl Read for Duplex {
    fn read(&mut self, buf: &mut [u8]) -> io::Result<usize> {
        self.input.read(buf)
    }
}

impl Write for Duplex {
    fn write(&mut self, buf: &[u8]) -> io::Result<usize> {
        self.output.write(buf)
    }
    fn flush(&mut self) -> io::Result<()> {
        Ok(())
    }
}

fn bundle(delta: u8, events: Vec<GameEvent>) -> GameBundle {
    GameBundle {
        app_seconds: 0.0,
        delta_physics_frames: delta,
        events,
    }
}

fn framed(bundles: &[GameBundle]) -> Vec<u8> {
    bundles
        .iter()
        .flat_map(|b| frame(&encode_game_packet(b).unwrap()).unwrap())
        .collect()
}

fn link(bundles: &[GameBundle]) -> RocketLeagueLink<Duplex> {
    RocketLeagueLink::new(Duplex {
        input: Cursor::new(framed(bundles)),
        output: Vec::new(),
    })
}

fn at(x: f32, y: f32, z: f32) -> Physics {
    Physics {
        location: Vec3 { x, y, z },
        ..Physics::default()
    }
}

fn spawn_player(id: u16, name: &str, team: u8) -> GameEvent {
    GameEvent::PlayerSpawn {
        id,
        command_id: None,
        name: name.into(),
        team,
    }
}

fn opening() -> GameBundle {
    bundle(
        0,
        vec![
            GameEvent::GameSpawn {
                map_name: "Stadium_P".into(),
                mode: 1,
                team_size: 1,
                gravity: None,
            },
            GameEvent::PickupSpawn {
                id: 31,
                location: Vec3 {
                    x: 100.0,
                    y: 500.0,
                    z: 0.0,
                },
                big: false,
            },
            GameEvent::PickupSpawn {
                id: 30,
                location: Vec3 {
                    x: -100.0,
                    y: 500.0,
                    z: 0.0,
                },
                big: true,
            },
            GameEvent::PickupSpawn {
                id: 29,
                location: Vec3 {
                    x: 0.0,
                    y: -500.0,
                    z: 0.0,
                },
                big: false,
            },
            spawn_player(8, "Blue", 0),
            spawn_player(3, "Orange", 1),
            GameEvent::BallSpawn { id: 20 },
            GameEvent::StateChange {
                phase: MatchPhase::Countdown,
                is_overtime: false,
            },
            GameEvent::TimeChange {
                display_seconds: 300,
            },
        ],
    )
}

#[test]
fn events_fold_into_the_packet_bots_receive() {
    let mut l = link(&[
        opening(),
        bundle(
            2,
            vec![
                GameEvent::PlayerPhysics {
                    id: 3,
                    physics: at(1.0, 2.0, 3.0),
                    state: 4,
                },
                GameEvent::PlayerBoost {
                    id: 3,
                    is_active: false,
                    amount: 71,
                },
                GameEvent::BallPhysics {
                    id: 20,
                    physics: at(0.0, 0.0, 93.0),
                },
                GameEvent::PickupState {
                    id: 30,
                    picked_up: true,
                },
                GameEvent::TeamScore { team: 1, score: 2 },
                GameEvent::StateChange {
                    phase: MatchPhase::Active,
                    is_overtime: true,
                },
            ],
        ),
    ]);
    let first = l.next_clump().unwrap().unwrap();
    assert!(first.map_just_loaded);
    let second = l.next_clump().unwrap().unwrap();
    assert!(!second.map_just_loaded);
    let p = second.packet;

    // Cars are in spawn order: the first to spawn is player index 0, whatever its actor id.
    let names: Vec<&str> = p.players.iter().map(|c| c.name.as_str()).collect();
    assert_eq!(names, ["Blue", "Orange"]);
    assert_eq!(p.players[1].physics.location.z, 3.0);
    assert_eq!(p.players[1].air_state, AirState::InAir);
    assert_eq!(p.players[1].boost, 71.0);
    assert_eq!((p.players[0].team, p.players[1].team), (0, 1));
    assert_eq!(p.balls[0].physics.location.z, 93.0);

    // Pads are ordered by y, then x: (0,-500), (-100,500), (100,500). Pad 30 (big) was picked up
    // and has run down by the 2 frames since (2/120 s) from its 10 s cooldown.
    let pads: Vec<(bool, f32)> = p
        .boost_pads
        .iter()
        .map(|b| (b.is_active, b.timer))
        .collect();
    assert_eq!(pads[0], (true, 0.0));
    assert!(!pads[1].0 && (pads[1].1 - 10.0).abs() < 0.1);
    assert_eq!(pads[2], (true, 0.0));

    assert_eq!(p.match_info.match_phase, MatchPhase::Active);
    assert!(p.match_info.is_overtime);
    assert_eq!(p.match_info.frame_num, 2);
    assert!((p.match_info.seconds_elapsed - 2.0 / 120.0).abs() < 1e-6);
    assert_eq!((p.teams[0].score, p.teams[1].score), (0, 2));
}

#[test]
fn the_match_clock_runs_between_scoreboard_updates_only_while_active() {
    let mut l = link(&[
        opening(),
        bundle(
            12,
            vec![GameEvent::StateChange {
                phase: MatchPhase::Active,
                is_overtime: false,
            }],
        ),
        bundle(12, vec![]),
        bundle(
            12,
            vec![GameEvent::TimeChange {
                display_seconds: 299,
            }],
        ),
    ]);
    let times: Vec<f32> = (0..4)
        .map(|_| {
            l.next_clump()
                .unwrap()
                .unwrap()
                .packet
                .match_info
                .game_time_remaining
        })
        .collect();
    // Countdown is not active, so 300 holds. The phase is judged after a bundle's events, so the
    // bundle that makes it active already runs the clock: 0.1 s per 12 frames, until the
    // scoreboard says 299, which it takes as the truth.
    assert_eq!(times[0], 300.0);
    assert!((times[1] - 299.9).abs() < 1e-3);
    assert!((times[2] - 299.8).abs() < 1e-3);
    assert_eq!(times[3], 299.0);
}

#[test]
fn a_new_map_clears_the_old_one_but_not_the_clock() {
    let mut l = link(&[
        opening(),
        bundle(6, vec![]),
        bundle(
            0,
            vec![GameEvent::GameSpawn {
                map_name: "Hoops_P".into(),
                mode: 2,
                team_size: 2,
                gravity: None,
            }],
        ),
    ]);
    l.next_clump().unwrap();
    l.next_clump().unwrap();
    let c = l.next_clump().unwrap().unwrap();
    assert!(c.map_just_loaded);
    assert!(c.packet.players.is_empty() && c.packet.boost_pads.is_empty());
    assert_eq!(c.packet.match_info.frame_num, 6);
    assert_eq!(l.folder().map_name(), "Hoops_P");
}

#[test]
fn a_destroyed_car_leaves_and_the_others_keep_their_indices() {
    let mut l = link(&[
        opening(),
        bundle(0, vec![GameEvent::ActorDestroyed { id: 8 }]),
    ]);
    l.next_clump().unwrap();
    let p = l.next_clump().unwrap().unwrap().packet;
    // A bot driving index 1 must not find itself in another car because index 0 left.
    assert_eq!(p.players.len(), 1);
    assert_eq!(
        (p.players[0].name.as_str(), p.players[0].player_id),
        ("Orange", 1)
    );
    assert_eq!(l.folder().actor_for_player(0), None);
    assert_eq!(l.folder().actor_for_player(1), Some(3));
}

/// What the link wrote so far, decoded.
fn written(output: &[u8]) -> Vec<(u16, Command)> {
    let mut d = FrameDecoder::new();
    d.push(output);
    let mut out = Vec::new();
    while let Some(payload) = d.next_frame() {
        out.extend(decode_controller_packet(&payload).unwrap());
    }
    out
}

#[test]
fn commands_reach_the_game_with_the_right_actor_and_counting_ids() {
    let mut l = link(&[opening()]);
    l.next_clump().unwrap();
    let state = ControllerState {
        throttle: 1.0,
        pitch: 0.5,
        yaw: -0.25,
        boost: true,
        ..ControllerState::default()
    };
    // Player index 1 is the second car to spawn: actor 3.
    l.send(GameCommand::PlayerInput {
        player_index: 1,
        state,
    })
    .unwrap();
    l.send(GameCommand::Console("Pause".into())).unwrap();
    let err = l
        .send(GameCommand::PlayerInput {
            player_index: 5,
            state,
        })
        .unwrap_err();
    assert_eq!(err.kind(), io::ErrorKind::InvalidInput);

    let out = take_output(l);
    let sent = written(&out);
    assert_eq!(sent.len(), 2);
    assert_eq!(sent[0].0, 1);
    let Command::PlayerInput { actor_id, input } = &sent[0].1 else {
        panic!("{:?}", sent[0]);
    };
    assert_eq!(*actor_id, 3);
    // RLBot's rule: dodge forward is minus pitch, strafe is yaw.
    assert_eq!((input.dodge_forward, input.dodge_strafe), (-0.5, -0.25));
    assert!(input.boost && input.throttle == 1.0);
    assert_eq!(sent[1], (2, Command::Exec("Pause".into())));
}

fn take_output(l: RocketLeagueLink<Duplex>) -> Vec<u8> {
    l.into_inner().output
}

#[test]
fn the_per_frame_byte_budget_stops_sends_until_the_game_speaks_again() {
    let mut l = link(&[opening(), bundle(1, vec![])]);
    l.next_clump().unwrap();
    let long = "x".repeat(2_000);
    let mut sent = 0usize;
    let blocked = loop {
        match l.send(GameCommand::Console(long.clone())) {
            Ok(()) => sent += 1,
            Err(e) => break e,
        }
        assert!(sent < 100, "never blocked");
    };
    assert_eq!(blocked.kind(), io::ErrorKind::WouldBlock);
    assert!(sent * 2_000 <= SEND_BUDGET && sent > 20, "sent {sent}");
    // Any message from the game resets the budget.
    l.next_clump().unwrap();
    l.send(GameCommand::Console(long)).unwrap();
}

#[test]
fn garbage_is_invalid_data_and_a_closed_game_is_the_end() {
    let mut bad = RocketLeagueLink::new(Duplex {
        input: Cursor::new(vec![0, 3, 1, 2, 3]),
        output: Vec::new(),
    });
    assert_eq!(
        bad.next_clump().unwrap_err().kind(),
        io::ErrorKind::InvalidData
    );
    let mut empty = link(&[]);
    assert!(empty.next_clump().unwrap().is_none());
}

#[test]
fn the_bridge_loop_distributes_only_messages_that_advance_time() {
    let mut l = link(&[
        opening(),
        bundle(2, vec![]),
        bundle(0, vec![]),
        bundle(2, vec![]),
    ]);
    let mut bridge = BridgeLoop::new();
    bridge.set_roster(Some(Roster {
        players: 2,
        bots: 2,
    }));
    let mut frames = Vec::new();
    bridge
        .run(&mut l, |p| frames.push(p.match_info.frame_num))
        .unwrap();
    // The opening and the zero-delta message do not advance time.
    assert_eq!(frames, [2, 4]);
}

#[test]
fn over_a_real_socket_a_game_dials_in_and_is_read_and_answered() {
    let listener = GameListener::bind(0).unwrap();
    let addr = listener.local_addr().unwrap();
    let game = thread::spawn(move || {
        let mut s = TcpStream::connect(addr).unwrap();
        s.write_all(&framed(&[opening(), bundle(2, vec![])]))
            .unwrap();
        // Read RLBot's reply to the first message.
        let mut d = FrameDecoder::new();
        let mut buf = [0u8; 256];
        s.set_read_timeout(Some(Duration::from_secs(10))).unwrap();
        loop {
            let n = s.read(&mut buf).unwrap();
            assert!(n > 0);
            d.push(&buf[..n]);
            if let Some(p) = d.next_frame() {
                return decode_controller_packet(&p).unwrap();
            }
        }
    });
    let stream = listener
        .wait_for_connection(Duration::from_secs(10))
        .unwrap()
        .expect("the game never connected");
    let mut l = RocketLeagueLink::new(stream);
    assert!(l.next_clump().unwrap().unwrap().map_just_loaded);
    let second = l.next_clump().unwrap().unwrap();
    assert_eq!(second.packet.match_info.frame_num, 2);
    l.send(GameCommand::Console("Pause".into())).unwrap();
    assert_eq!(game.join().unwrap(), [(1, Command::Exec("Pause".into()))]);
}

fn spawn(index: u32, name: &str, team: u8) -> GameCommand {
    GameCommand::SpawnBot {
        player_index: index,
        name: name.into(),
        skill: BotSkill::Custom,
        team,
        loadout: Loadout::standard(team),
    }
}

fn answered(actor: u16, command_id: Option<u16>, name: &str) -> GameBundle {
    bundle(
        1,
        vec![GameEvent::PlayerSpawn {
            id: actor,
            command_id,
            name: name.into(),
            team: 0,
        }],
    )
}

#[test]
fn a_spawned_car_takes_the_index_it_was_spawned_for_whatever_order_cars_appear_in() {
    // The game answers the second spawn command first, then the first, then a human (command 0).
    let mut l = link(&[
        answered(40, Some(2), "Second"),
        answered(41, Some(1), "First"),
        answered(42, Some(0), "Human"),
    ]);
    l.send(spawn(0, "First", 0)).unwrap();
    l.send(spawn(1, "Second", 1)).unwrap();
    for _ in 0..3 {
        l.next_clump().unwrap();
    }
    let folder = l.folder();
    assert_eq!(folder.actor_for_player(0), Some(41));
    assert_eq!(folder.actor_for_player(1), Some(40));
    assert_eq!(
        folder.actor_for_player(2),
        Some(42),
        "a human takes the next free index"
    );
    let names: Vec<String> = folder
        .packet()
        .players
        .into_iter()
        .map(|p| p.name)
        .collect();
    assert_eq!(
        names,
        ["First", "Second", "Human"],
        "the packet is by player index"
    );

    // What went to the game: two SpawnBot commands, counting from 1, with the loadout.
    let sent = written(&l.into_inner().output);
    assert_eq!(sent.len(), 2);
    let (
        id,
        Command::SpawnBot {
            name,
            skill,
            team,
            loadout,
        },
    ) = &sent[1]
    else {
        panic!("{:?}", sent[1]);
    };
    assert_eq!(
        (*id, name.as_str(), *skill, *team),
        (2, "Second", BotSkill::Custom, 1)
    );
    assert_eq!(loadout.products[0], 23, "an Octane");
}

#[test]
fn a_refused_spawn_leaves_no_promise_behind() {
    // Use up the byte budget so the spawn command is refused.
    let mut l = link(&[answered(50, None, "Someone")]);
    // Tiny commands, so what is left cannot hold a spawn.
    while l.send(GameCommand::Console("x".into())).is_ok() {}
    let err = l.send(spawn(0, "Refused", 0)).unwrap_err();
    assert_eq!(err.kind(), io::ErrorKind::WouldBlock);
    // A car that then appears unannounced takes index 0: nothing was promised to the refused
    // spawn, so nothing is held back for it.
    l.next_clump().unwrap();
    assert_eq!(l.folder().actor_for_player(0), Some(50));
}

#[test]
fn a_human_already_in_the_game_moves_along_when_a_bot_is_spawned_for_its_index() {
    // The human spawns first (command id 0) and takes index 0; the bot then spawned for index 0
    // gets it, and the human is not lost.
    let mut l = link(&[answered(40, Some(0), "Human"), answered(41, Some(1), "Bot")]);
    l.next_clump().unwrap();
    assert_eq!(l.folder().actor_for_player(0), Some(40));
    assert!(
        !l.folder().is_claimed(0),
        "a human's car is not a spawned bot"
    );
    l.send(spawn(0, "Bot", 0)).unwrap();
    l.next_clump().unwrap();
    assert!(l.folder().is_claimed(0));
    assert_eq!(
        l.folder().actor_for_player(0),
        Some(41),
        "the bot has its index"
    );
    assert_eq!(
        l.folder().actor_for_player(1),
        Some(40),
        "the human moved to the next"
    );
    let names: Vec<String> = l
        .folder()
        .packet()
        .players
        .into_iter()
        .map(|p| p.name)
        .collect();
    assert_eq!(names, ["Bot", "Human"]);
}
