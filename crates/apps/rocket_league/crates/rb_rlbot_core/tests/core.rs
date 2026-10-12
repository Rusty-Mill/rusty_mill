//! The core over real sockets, driven by the real `rb_rlbot_client`.
#![allow(clippy::unwrap_used, clippy::expect_used, clippy::panic)]

use std::net::SocketAddr;
use std::sync::atomic::AtomicBool;
use std::sync::mpsc;
use std::sync::{Arc, Mutex};
use std::thread::{self, JoinHandle};
use std::time::Duration;

use rb_rlbot_client::Connection;
use rb_rlbot_core::{Core, Event, Pacing, RunEnd, Simulation};
use rb_rlbot_wire::{
    ConnectionSettings, ControllerState, CoreMessage, CustomBot, GamePacket, InitComplete,
    InterfaceMessage, MatchConfiguration, MatchPhase, PlayerClass, PlayerConfiguration,
    PlayerInput, StopCommand,
};

fn bot(agent_id: &str, team: u32, player_id: i32) -> PlayerConfiguration {
    PlayerConfiguration {
        variety: PlayerClass::CustomBot(CustomBot {
            name: agent_id.into(),
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

struct Running {
    addr: SocketAddr,
    events: Arc<Mutex<Vec<Event>>>,
    handle: JoinHandle<RunEnd>,
}

impl Running {
    fn finish(self) -> (RunEnd, Vec<Event>) {
        let end = self.handle.join().unwrap();
        let events = self.events.lock().unwrap().clone();
        (end, events)
    }
}

/// Starts a lockstep core for `players` on a free port. The core is built on its own thread
/// because the simulation is not meant to move between threads.
fn start(players: Vec<PlayerConfiguration>) -> Running {
    let (tx, rx) = mpsc::channel();
    let events = Arc::new(Mutex::new(Vec::new()));
    let sink = Arc::clone(&events);
    let handle = thread::spawn(move || {
        let sim = Simulation::new(MatchConfiguration {
            player_configurations: players,
            ..MatchConfiguration::default()
        })
        .unwrap();
        let mut core = Core::bind(
            "127.0.0.1:0",
            sim,
            Pacing::Lockstep {
                input_timeout: Duration::from_secs(10),
            },
        )
        .unwrap();
        tx.send(core.local_addr().unwrap()).unwrap();
        core.run(&AtomicBool::new(false), |e| sink.lock().unwrap().push(e))
            .unwrap()
    });
    Running {
        addr: rx.recv().unwrap(),
        events,
        handle,
    }
}

fn settings(agent_id: &str) -> ConnectionSettings {
    ConnectionSettings {
        agent_id: agent_id.into(),
        wants_ball_predictions: false,
        wants_comms: false,
        close_between_matches: true,
    }
}

fn next_packet(connection: &mut Connection) -> GamePacket {
    loop {
        match connection.recv().unwrap() {
            CoreMessage::GamePacket(p) => return p,
            CoreMessage::DisconnectSignal => panic!("disconnected before the next packet"),
            _ => {}
        }
    }
}

/// Connects as `agent_id`, plays `packets` packets with `input` held on `car`, then stops the
/// match. Returns every packet seen.
fn play(addr: SocketAddr, agent_id: &str, car: u32, packets: usize) -> Vec<GamePacket> {
    let mut connection = Connection::connect(addr).unwrap();
    let info = connection.handshake(settings(agent_id)).unwrap();
    assert_eq!(info.controllable_team_info.controllables[0].index, car);
    connection.send(InitComplete).unwrap();
    let input = ControllerState {
        throttle: 1.0,
        boost: true,
        ..ControllerState::default()
    };
    let mut seen = Vec::new();
    while seen.len() < packets {
        seen.push(next_packet(&mut connection));
        connection
            .send(PlayerInput {
                player_index: car,
                controller_state: input,
            })
            .unwrap();
    }
    connection
        .send(StopCommand {
            shutdown_server: true,
        })
        .unwrap();
    // Core answers a stop with a disconnect.
    loop {
        if matches!(connection.recv().unwrap(), CoreMessage::DisconnectSignal) {
            return seen;
        }
    }
}

#[test]
fn a_bot_gets_its_cars_the_match_and_the_field_then_drives_a_car() {
    let core = start(vec![bot("solo", 0, 100)]);
    // 850 ticks of first-kickoff countdown, then the kickoff itself.
    let seen = play(core.addr, "solo", 0, 1200);
    let (end, events) = core.finish();
    assert_eq!(
        end,
        RunEnd::StopCommand {
            shutdown_server: true
        }
    );
    assert!(events.contains(&Event::MatchStarted));
    assert!(events.contains(&Event::AgentReady {
        agent_id: "solo".into()
    }));
    // The packet loop runs in lockstep: one new frame per answered packet.
    assert!(seen
        .windows(2)
        .all(|w| { w[1].match_info.frame_num == w[0].match_info.frame_num + 1 }));
    assert_eq!(seen[0].match_info.match_phase, MatchPhase::Countdown);
    let last = seen.last().unwrap();
    assert_ne!(last.match_info.match_phase, MatchPhase::Countdown);
    let (first, now) = (&seen[0].players[0].physics, &last.players[0].physics);
    let moved = (now.location.x - first.location.x).hypot(now.location.y - first.location.y);
    assert!(
        moved > 100.0,
        "the car moved {moved} uu after the countdown"
    );
}

#[test]
fn the_same_bot_gives_the_same_match_every_time() {
    let run = || {
        let core = start(vec![bot("solo", 0, 100)]);
        let seen = play(core.addr, "solo", 0, 900);
        core.finish();
        seen
    };
    assert_eq!(run(), run());
}

#[test]
fn a_match_waits_for_every_named_agent_and_input_is_routed_by_agent() {
    let core = start(vec![bot("one", 0, 100), bot("two", 1, 101)]);
    let one = thread::spawn({
        let addr = core.addr;
        move || {
            let mut c = Connection::connect(addr).unwrap();
            c.handshake(settings("one")).unwrap();
            c.send(InitComplete).unwrap();
            c
        }
    });
    let mut one = one.join().unwrap();

    // Agent one is ready but agent two is not: no packet yet.
    assert!(one
        .recv_timeout(Duration::from_millis(300))
        .unwrap()
        .is_none());

    let mut two = Connection::connect(core.addr).unwrap();
    two.handshake(settings("two")).unwrap();
    two.send(InitComplete).unwrap();
    let first_one = next_packet(&mut one);
    let first_two = next_packet(&mut two);
    assert_eq!(first_one, first_two, "both see the same packet");

    // Two sends input for car 0, which is one's: refused, and the match still advances once
    // each controlling client has answered.
    two.send(PlayerInput {
        player_index: 0,
        controller_state: ControllerState::default(),
    })
    .unwrap();
    two.send(PlayerInput {
        player_index: 1,
        controller_state: ControllerState::default(),
    })
    .unwrap();
    one.send(PlayerInput {
        player_index: 0,
        controller_state: ControllerState::default(),
    })
    .unwrap();
    assert_eq!(next_packet(&mut one).match_info.frame_num, 1);

    one.send(StopCommand {
        shutdown_server: true,
    })
    .unwrap();
    let (_, events) = core.finish();
    assert!(events.contains(&Event::InputRejected {
        agent_id: "two".into(),
        player_index: 0
    }));
}

#[test]
fn an_anonymous_client_watches_without_an_init_complete() {
    let core = start(vec![bot("solo", 0, 100)]);
    let mut watcher = Connection::connect(core.addr).unwrap();
    // A match runner or logger: no agent id, so no team information and no cars to wait for.
    watcher.send(settings("")).unwrap();
    let mut opening = Vec::new();
    for _ in 0..2 {
        opening.push(watcher.recv().unwrap());
    }
    assert!(matches!(opening[0], CoreMessage::MatchConfiguration(_)));
    assert!(matches!(opening[1], CoreMessage::FieldInfo(_)));

    let mut bot_side = Connection::connect(core.addr).unwrap();
    bot_side.handshake(settings("solo")).unwrap();
    bot_side.send(InitComplete).unwrap();
    let seen = next_packet(&mut watcher);
    assert_eq!(seen.match_info.frame_num, 0);
    watcher
        .send(InterfaceMessage::StopCommand(StopCommand {
            shutdown_server: true,
        }))
        .unwrap();
    core.finish();
}

#[test]
fn a_duplicate_agent_id_and_garbage_are_dropped_not_fatal() {
    let core = start(vec![bot("solo", 0, 100)]);
    let mut first = Connection::connect(core.addr).unwrap();
    first.handshake(settings("solo")).unwrap();

    // Same id again: refused by closing the connection.
    let mut dup = Connection::connect(core.addr).unwrap();
    dup.send(settings("solo")).unwrap();
    assert!(dup.recv().is_err(), "the duplicate should be disconnected");

    // A frame that is not a message.
    let mut junk = std::net::TcpStream::connect(core.addr).unwrap();
    std::io::Write::write_all(&mut junk, &[0, 3, 1, 2, 3]).unwrap();

    first.send(InitComplete).unwrap();
    assert_eq!(next_packet(&mut first).match_info.frame_num, 0);
    first
        .send(StopCommand {
            shutdown_server: true,
        })
        .unwrap();
    let (_, events) = core.finish();
    let dropped: Vec<&Event> = events
        .iter()
        .filter(|e| matches!(e, Event::ClientDropped { .. }))
        .collect();
    assert_eq!(dropped.len(), 2, "{events:?}");
}

#[test]
fn a_client_that_says_it_is_leaving_is_dropped_and_the_match_goes_on() {
    let core = start(vec![bot("solo", 0, 100)]);
    let mut leaver = Connection::connect(core.addr).unwrap();
    leaver.send(settings("")).unwrap();
    leaver.send(InterfaceMessage::DisconnectSignal).unwrap();

    let mut bot_side = Connection::connect(core.addr).unwrap();
    bot_side.handshake(settings("solo")).unwrap();
    bot_side.send(InitComplete).unwrap();
    assert_eq!(next_packet(&mut bot_side).match_info.frame_num, 0);
    bot_side
        .send(StopCommand {
            shutdown_server: true,
        })
        .unwrap();
    let (_, events) = core.finish();
    assert!(
        events.iter().any(|e| matches!(
            e,
            Event::ClientDropped { reason, .. } if reason.contains("disconnect signal")
        )),
        "{events:?}"
    );
}

impl Running {
    /// Like `finish`, but gives up (returning `None`) if the run does not end in `wait`, so a
    /// regression that leaves the core running fails the test instead of hanging it.
    fn finish_within(self, wait: Duration) -> Option<(RunEnd, Vec<Event>)> {
        let deadline = std::time::Instant::now() + wait;
        while !self.handle.is_finished() {
            if std::time::Instant::now() > deadline {
                return None;
            }
            thread::sleep(Duration::from_millis(5));
        }
        Some(self.finish())
    }
}

#[test]
fn a_stop_sent_just_before_the_client_exits_still_stops_the_match() {
    let core = start(vec![bot("solo", 0, 100)]);
    {
        let mut runner = Connection::connect(core.addr).unwrap();
        // A runner introduces itself (anonymously) before it may stop the match.
        runner.send(settings("")).unwrap();
        runner
            .send(StopCommand {
                shutdown_server: true,
            })
            .unwrap();
        // Dropped here: the close follows the stop, and the core may see both in one pass.
    }
    let (end, _) = core
        .finish_within(Duration::from_secs(5))
        .expect("the stop was lost when its sender closed the socket");
    assert_eq!(
        end,
        RunEnd::StopCommand {
            shutdown_server: true
        }
    );
}

#[test]
fn a_stop_followed_by_a_disconnect_in_one_write_still_stops_the_match() {
    let core = start(vec![bot("solo", 0, 100)]);
    let mut runner = Connection::connect(core.addr).unwrap();
    runner
        .send_all(&[
            InterfaceMessage::ConnectionSettings(settings("")),
            InterfaceMessage::StopCommand(StopCommand {
                shutdown_server: true,
            }),
            InterfaceMessage::DisconnectSignal,
        ])
        .unwrap();
    let (end, events) = core
        .finish_within(Duration::from_secs(5))
        .expect("the stop was lost because a later message in the same read was an error");
    assert!(
        matches!(end, RunEnd::StopCommand { .. }),
        "{end:?} {events:?}"
    );
}

#[test]
fn connections_that_vanish_do_not_disturb_the_match() {
    let core = start(vec![bot("solo", 0, 100)]);
    for _ in 0..40 {
        drop(std::net::TcpStream::connect(core.addr).unwrap());
    }
    let seen = play(core.addr, "solo", 0, 5);
    assert_eq!(seen.len(), 5);
    core.finish();
}

#[test]
fn init_complete_before_the_connection_settings_is_refused() {
    let core = start(vec![bot("solo", 0, 100)]);
    let mut rude = Connection::connect(core.addr).unwrap();
    rude.send(InitComplete).unwrap();
    let mut stopper = Connection::connect(core.addr).unwrap();
    // Give the core a pass over the rude client first, then end the run.
    thread::sleep(Duration::from_millis(200));
    stopper.send(settings("")).unwrap();
    stopper
        .send(StopCommand {
            shutdown_server: true,
        })
        .unwrap();
    let (_, events) = core.finish();
    assert!(
        events.iter().any(|e| matches!(
            e,
            Event::ClientDropped { reason, .. } if reason.contains("before its connection settings")
        )),
        "{events:?}"
    );
}

#[test]
fn an_input_sent_before_the_match_began_does_not_answer_its_first_packet() {
    let core = start(vec![bot("early", 0, 100), bot("late", 1, 101)]);
    let mut early = Connection::connect(core.addr).unwrap();
    early.handshake(settings("early")).unwrap();
    early.send(InitComplete).unwrap();
    // Sent before the match exists, while the core waits for the other bot.
    early
        .send(PlayerInput {
            player_index: 0,
            controller_state: ControllerState::default(),
        })
        .unwrap();
    thread::sleep(Duration::from_millis(200));

    let mut late = Connection::connect(core.addr).unwrap();
    late.handshake(settings("late")).unwrap();
    late.send(InitComplete).unwrap();
    assert_eq!(next_packet(&mut early).match_info.frame_num, 0);
    assert_eq!(next_packet(&mut late).match_info.frame_num, 0);

    // `late` answers the first packet; `early` has not (its earlier input does not count), so
    // lockstep must still wait for it.
    late.send(PlayerInput {
        player_index: 1,
        controller_state: ControllerState::default(),
    })
    .unwrap();
    assert!(
        early
            .recv_timeout(Duration::from_millis(400))
            .unwrap()
            .is_none(),
        "the second tick ran without early answering the first packet"
    );
    early
        .send(PlayerInput {
            player_index: 0,
            controller_state: ControllerState::default(),
        })
        .unwrap();
    assert_eq!(next_packet(&mut early).match_info.frame_num, 1);

    early
        .send(StopCommand {
            shutdown_server: true,
        })
        .unwrap();
    core.finish();
}

/// A connection that has not sent its connection settings cannot stop the match, set its state
/// or ping; it is dropped, and the match carries on for the real bot.
#[test]
fn a_bare_connection_cannot_stop_the_match() {
    use rb_rlbot_wire::frame::frame;
    use std::io::Write;

    let running = start(vec![bot("test/bot", 0, 0)]);
    let mut bare = std::net::TcpStream::connect(running.addr).unwrap();
    let stop = InterfaceMessage::StopCommand(StopCommand {
        shutdown_server: true,
    });
    bare.write_all(&frame(&stop.to_payload().unwrap()).unwrap())
        .unwrap();

    let packets = play(running.addr, "test/bot", 0, 5);
    assert_eq!(packets.len(), 5, "the match ran on after the bare stop");
    let (end, events) = running.finish();
    // `play` ends the match with shutdown requested.
    assert_eq!(
        end,
        RunEnd::StopCommand {
            shutdown_server: true
        }
    );
    assert!(events.iter().any(|e| matches!(
        e,
        Event::ClientDropped { reason, .. } if reason.contains("before its connection settings")
    )));
}

/// A stream of connections that never speak does not keep the loop from serving a bot or from
/// ending the match.
#[test]
fn a_connection_flood_does_not_starve_a_bot_or_the_stop() {
    use std::sync::atomic::{AtomicBool, Ordering};

    let running = start(vec![bot("test/bot", 0, 0)]);
    let addr = running.addr;
    let done = Arc::new(AtomicBool::new(false));
    let flag = Arc::clone(&done);
    let flooder = thread::spawn(move || {
        let mut held = Vec::new();
        while !flag.load(Ordering::Relaxed) {
            if let Ok(socket) = std::net::TcpStream::connect(addr) {
                held.push(socket);
            }
            if held.len() > 2000 {
                held.clear();
            }
        }
    });
    let packets = play(addr, "test/bot", 0, 20);
    done.store(true, Ordering::Relaxed);
    flooder.join().unwrap();
    assert_eq!(packets.len(), 20);
    let (end, _) = running.finish();
    assert!(matches!(end, RunEnd::StopCommand { .. }));
}
