//! The built `rb_chaser_bot` executable, run as core runs it: a separate process told where core
//! is and who it is through `RLBOT_SERVER_PORT` and `RLBOT_AGENT_ID`.
//!
//! Against a stand-in core: what it says on the wire, and how it exits. Against the simulated
//! core (`rb_rlbot_core`): that two of them play a match that reaches the ball in a fixed time,
//! and the same match every time.
#![allow(clippy::unwrap_used, clippy::expect_used, clippy::panic)]

use std::io::Write;
use std::net::{SocketAddr, TcpListener, TcpStream};
use std::process::{Child, Command, Output, Stdio};
use std::sync::atomic::AtomicBool;
use std::sync::mpsc;
use std::sync::{Arc, Mutex};
use std::thread::{self, JoinHandle};
use std::time::Duration;

use rb_rlbot_bridge::{chaser, controller_state, observation};
use rb_rlbot_client::Connection;
use rb_rlbot_core::{bot_match, Core, Event, Pacing, RunEnd, Simulation};
use rb_rlbot_wire::frame::{frame, read_frame};
use rb_rlbot_wire::{
    AirState, BallInfo, ConnectionSettings, ControllableInfo, ControllableTeamInfo, CoreMessage,
    FieldInfo, GamePacket, InterfaceMessage, MatchConfiguration, MatchInfo, MatchLengthMutator,
    MatchPhase, Physics, PlayerInfo, StopCommand, Vec3,
};

const BOT: &str = env!("CARGO_BIN_EXE_rb_chaser_bot");
const WAIT: Duration = Duration::from_secs(30);

/// A spawned bot, killed if the test fails before it exits.
struct Bot(Option<Child>);

impl Bot {
    fn spawn(port: u16, agent_id: &str) -> Bot {
        let child = Command::new(BOT)
            .env_remove("RLBOT_SERVER_ADDR")
            .env_remove("RLBOT_SERVER_IP")
            .env("RLBOT_SERVER_PORT", port.to_string())
            .env("RLBOT_AGENT_ID", agent_id)
            .stdout(Stdio::piped())
            .stderr(Stdio::piped())
            .spawn()
            .unwrap();
        Bot(Some(child))
    }

    fn wait(mut self) -> Output {
        self.0.take().unwrap().wait_with_output().unwrap()
    }
}

impl Drop for Bot {
    fn drop(&mut self) {
        if let Some(child) = &mut self.0 {
            let _ = child.kill();
            let _ = child.wait();
        }
    }
}

fn text(bytes: &[u8]) -> String {
    String::from_utf8_lossy(bytes).into_owned()
}

/// One accepted connection, spoken to as core speaks: `CoreMessage`s out, `InterfaceMessage`s in.
struct StandIn {
    stream: TcpStream,
    payload: Vec<u8>,
}

impl StandIn {
    fn accept(listener: &TcpListener) -> StandIn {
        let (stream, _) = listener.accept().unwrap();
        stream.set_read_timeout(Some(WAIT)).unwrap();
        StandIn {
            stream,
            payload: Vec::new(),
        }
    }

    fn send(&mut self, message: CoreMessage) {
        let bytes = frame(&message.to_payload().unwrap()).unwrap();
        self.stream.write_all(&bytes).unwrap();
    }

    fn recv(&mut self) -> InterfaceMessage {
        read_frame(&mut self.stream, &mut self.payload).unwrap();
        InterfaceMessage::from_payload(&self.payload).unwrap()
    }
}

fn car_at(x: f32, y: f32, team: u32) -> PlayerInfo {
    PlayerInfo {
        physics: Physics {
            location: Vec3 { x, y, z: 17.0 },
            ..Physics::default()
        },
        air_state: AirState::OnGround,
        demolished_timeout: 0.0,
        is_bot: true,
        name: String::new(),
        team,
        boost: 33.0,
        player_id: 100 + team as i32,
    }
}

/// Two cars facing +x, the ball at the centre; car 1 level with the ball on its -x side, where
/// the point behind the ball (the chaser's target) is to its right for one team and its left for
/// the other.
fn kickoff_packet(frame_num: u32) -> GamePacket {
    GamePacket {
        players: vec![car_at(0.0, -2000.0, 0), car_at(-1000.0, 0.0, 1)],
        boost_pads: Vec::new(),
        balls: vec![BallInfo {
            physics: Physics {
                location: Vec3 {
                    x: 0.0,
                    y: 0.0,
                    z: 93.0,
                },
                ..Physics::default()
            },
        }],
        match_info: MatchInfo {
            seconds_elapsed: 1.0,
            game_time_remaining: 299.0,
            is_overtime: false,
            is_unlimited_time: false,
            match_phase: MatchPhase::Active,
            frame_num,
        },
        teams: Vec::new(),
    }
}

/// Plays core's side of the handshake for the car at index 1 on team 1, sends one packet and
/// returns the bot's answer. The connection stays open.
fn handshake_and_one_packet(core: &mut StandIn, agent_id: &str) -> InterfaceMessage {
    match core.recv() {
        InterfaceMessage::ConnectionSettings(s) => assert_eq!(s.agent_id, agent_id),
        other => panic!("expected connection settings first, got {other:?}"),
    }
    core.send(CoreMessage::ControllableTeamInfo(ControllableTeamInfo {
        team: 1,
        controllables: vec![ControllableInfo {
            index: 1,
            identifier: 101,
        }],
    }));
    core.send(CoreMessage::MatchConfiguration(
        MatchConfiguration::default(),
    ));
    core.send(CoreMessage::FieldInfo(FieldInfo::default()));
    assert!(matches!(core.recv(), InterfaceMessage::InitComplete(_)));
    core.send(CoreMessage::GamePacket(kickoff_packet(1)));
    core.recv()
}

fn listener() -> (TcpListener, u16) {
    let listener = TcpListener::bind("127.0.0.1:0").unwrap();
    let port = listener.local_addr().unwrap().port();
    (listener, port)
}

#[test]
fn the_bot_drives_the_car_core_gives_it_and_exits_cleanly_on_disconnect() {
    let (listener, port) = listener();
    let bot = Bot::spawn(port, "test/orange_chaser");
    let mut core = StandIn::accept(&listener);

    let answer = handshake_and_one_packet(&mut core, "test/orange_chaser");
    let InterfaceMessage::PlayerInput(input) = answer else {
        panic!("expected the car's input, got {answer:?}");
    };
    // Car 1, not 0, played as orange: the same decision the in-process chaser makes for team 1,
    // which differs from what it would do for blue in the same spot.
    let frame = observation(&kickoff_packet(1)).unwrap();
    assert_eq!(input.player_index, 1);
    assert_eq!(
        input.controller_state,
        controller_state(&chaser(1)(&frame, 1))
    );
    assert_ne!(
        input.controller_state,
        controller_state(&chaser(0)(&frame, 1))
    );

    core.send(CoreMessage::DisconnectSignal);
    let out = bot.wait();
    let stdout = text(&out.stdout);
    assert!(
        out.status.success(),
        "{:?}: {}",
        out.status,
        text(&out.stderr)
    );
    assert!(
        stdout.contains("`test/orange_chaser`: car 1 on team 1"),
        "{stdout}"
    );
    assert!(stdout.contains("`test/orange_chaser` exited"), "{stdout}");
}

#[test]
fn the_bot_fails_when_core_goes_away_mid_match() {
    let (listener, port) = listener();
    let bot = Bot::spawn(port, "test/orange_chaser");
    let mut core = StandIn::accept(&listener);
    handshake_and_one_packet(&mut core, "test/orange_chaser");

    // Closed without a disconnect signal: the bot must not report success.
    drop(core);
    let out = bot.wait();
    assert!(!out.status.success());
    assert!(
        text(&out.stderr).contains("Closed"),
        "{}",
        text(&out.stderr)
    );
    assert!(!text(&out.stdout).contains("exited"));
}

#[test]
fn the_bot_fails_when_nothing_is_listening() {
    let (listener, port) = listener();
    drop(listener);
    let out = Bot::spawn(port, "test/orange_chaser").wait();
    assert!(!out.status.success());
    assert!(out.stdout.is_empty(), "{}", text(&out.stdout));
}

/// A lockstep simulated core for a blue and an orange chaser, on its own thread.
struct SimCore {
    addr: SocketAddr,
    events: Arc<Mutex<Vec<Event>>>,
    handle: JoinHandle<RunEnd>,
}

const BLUE: &str = "test/blue_chaser";
const ORANGE: &str = "test/orange_chaser";

fn start_sim_core() -> SimCore {
    let (tx, rx) = mpsc::channel();
    let events = Arc::new(Mutex::new(Vec::new()));
    let sink = Arc::clone(&events);
    let handle = thread::spawn(move || {
        let players = [(BLUE.to_string(), 0), (ORANGE.to_string(), 1)];
        let sim = Simulation::new(bot_match(&players, MatchLengthMutator::FiveMinutes)).unwrap();
        let pacing = Pacing::Lockstep {
            input_timeout: Duration::from_secs(10),
        };
        let mut core = Core::bind("127.0.0.1:0", sim, pacing).unwrap();
        tx.send(core.local_addr().unwrap()).unwrap();
        core.run(&AtomicBool::new(false), |e| sink.lock().unwrap().push(e))
            .unwrap()
    });
    SimCore {
        addr: rx.recv().unwrap(),
        events,
        handle,
    }
}

/// The first kickoff counts down for 850 ticks; the chasers then have 20 s (2400 ticks at
/// 120 Hz) to hit the ball from the kickoff spots. They take about 12.8 s today (frame 2390):
/// the chaser circles to the point behind the ball before driving through it. The bound catches
/// a bot that never gets there, not a slower kickoff.
const TOUCH_BY_FRAME: u32 = 850 + 2400;

/// The ball has been hit once it moves sideways: until the first touch it only settles onto
/// the centre spot, straight down.
fn ball_moving(packet: &GamePacket) -> bool {
    let v = packet.balls[0].physics.velocity;
    (v.x * v.x + v.y * v.y).sqrt() > 50.0
}

/// Runs both chasers against the simulated core, watching as a match runner (empty agent id)
/// until the ball is hit, then stops the match. Returns every packet up to the first one with
/// the ball moving.
fn chaser_match() -> Vec<GamePacket> {
    let core = start_sim_core();
    let port = core.addr.port();
    let blue = Bot::spawn(port, BLUE);
    let orange = Bot::spawn(port, ORANGE);

    let mut runner = Connection::connect(core.addr).unwrap();
    runner
        .send(ConnectionSettings {
            agent_id: String::new(),
            wants_ball_predictions: false,
            wants_comms: false,
            close_between_matches: true,
        })
        .unwrap();
    let mut seen = Vec::new();
    loop {
        let message = runner.recv_timeout(WAIT).unwrap();
        let Some(CoreMessage::GamePacket(packet)) = message else {
            assert!(message.is_some(), "no packet within {WAIT:?}");
            continue;
        };
        let frame_num = packet.match_info.frame_num;
        assert!(frame_num <= TOUCH_BY_FRAME, "no touch by frame {frame_num}");
        let hit = ball_moving(&packet);
        seen.push(packet);
        if hit {
            break;
        }
    }
    runner
        .send(StopCommand {
            shutdown_server: true,
        })
        .unwrap();

    let end = core.handle.join().unwrap();
    assert_eq!(
        end,
        RunEnd::StopCommand {
            shutdown_server: true
        }
    );
    for (bot, id, line) in [
        (blue, BLUE, "car 0 on team 0"),
        (orange, ORANGE, "car 1 on team 1"),
    ] {
        let out = bot.wait();
        let stdout = text(&out.stdout);
        assert!(
            out.status.success(),
            "{id}: {:?}: {}",
            out.status,
            text(&out.stderr)
        );
        assert!(stdout.contains(&format!("`{id}`: {line}")), "{stdout}");
    }
    let events = core.events.lock().unwrap().clone();
    let bad: Vec<_> = events
        .iter()
        .filter(|e| {
            matches!(
                e,
                Event::InputRejected { .. } | Event::LockstepTimeout { .. }
            )
        })
        .collect();
    assert!(bad.is_empty(), "{bad:?}");
    seen
}

#[test]
fn two_chaser_processes_reach_the_ball_in_a_fixed_time_and_the_same_way_every_time() {
    let first = chaser_match();
    let last = first.last().unwrap();
    assert!(ball_moving(last));
    // Both cars drove toward the ball from their own end.
    let start = &first[0];
    for car in 0..2 {
        let distance = |p: &GamePacket| {
            let (c, b) = (p.players[car].physics.location, p.balls[0].physics.location);
            ((c.x - b.x).powi(2) + (c.y - b.y).powi(2)).sqrt()
        };
        assert!(
            distance(last) < distance(start) / 2.0,
            "car {car} did not close on the ball"
        );
    }
    assert_eq!(chaser_match(), first);
}
