//! The real `rb_tape_bot` and `rb_tape_hive` processes against a stand-in for RLBot core: the
//! handshake, the start state set on the first packet where physics advances, and the tape played
//! one input per physics frame. CI cannot run Rocket League; this is the part of the stage 4
//! check that can run anywhere. (Whether the real core accepts the same bytes is checked once by
//! hand; see README.)
#![allow(clippy::unwrap_used, clippy::expect_used, clippy::panic)]

use std::io::{Read, Write};
use std::net::{TcpListener, TcpStream};
use std::path::PathBuf;
use std::process::{Child, Command};
use std::time::{Duration, Instant};

use rb_rlbot_wire::frame::{frame, FrameDecoder};
use rb_rlbot_wire::{
    ControllableInfo, ControllableTeamInfo, CoreMessage, FieldInfo, GamePacket, InterfaceMessage,
    MatchConfiguration, MatchInfo, MatchPhase, PartialVec3, PlayerInput,
};
use rb_scenario::Scenario;
use rb_tape_bot::controller;

/// One car at `x`, ball in front, then a tape that presses something different each step.
fn scenario_json(others: &str) -> String {
    format!(
        r#"{{"name":"process test","settle_ticks":0,
            "car":{{"location":[10,-2000,17],"boost":50}},
            "ball":{{"location":[100,0,100]}},
            "steps":[{{"ticks":2,"throttle":1.0,"jump":true}},{{"ticks":2,"steer":-0.5,"boost":true}}]
            {others}}}"#
    )
}

struct Run {
    child: Child,
    peer: Peer,
    scenario: Scenario,
    tape: PathBuf,
}

impl Drop for Run {
    fn drop(&mut self) {
        let _ = self.child.kill();
        let _ = std::fs::remove_file(&self.tape);
    }
}

struct Peer {
    stream: TcpStream,
    decoder: FrameDecoder,
}

impl Peer {
    fn send(&mut self, message: &CoreMessage) {
        let bytes = frame(&message.to_payload().unwrap()).unwrap();
        self.stream.write_all(&bytes).unwrap();
    }

    /// The next message, or `None` once the bot has closed the connection.
    fn next(&mut self) -> Option<InterfaceMessage> {
        let mut chunk = [0u8; 4096];
        loop {
            if let Some(payload) = self.decoder.next_frame() {
                return Some(InterfaceMessage::from_payload(&payload).unwrap());
            }
            match self.stream.read(&mut chunk).unwrap() {
                0 => return None,
                n => self.decoder.push(&chunk[..n]),
            }
        }
    }

    fn recv(&mut self) -> InterfaceMessage {
        self.next().expect("the bot closed the connection")
    }
}

/// Starts `bin` as a client of a fresh fake core and plays core's opening for `cars`.
fn start(bin: &str, scenario_json: &str, cars: &[u32], name: &str) -> Run {
    let listener = TcpListener::bind("127.0.0.1:0").unwrap();
    let tape = std::env::temp_dir().join(format!("rb_{name}_{}.json", std::process::id()));
    std::fs::write(&tape, scenario_json).unwrap();
    let child = Command::new(bin)
        .env(
            "RLBOT_SERVER_ADDR",
            listener.local_addr().unwrap().to_string(),
        )
        .env_remove("RLBOT_AGENT_ID")
        .env("RB_TAPE", &tape)
        .spawn()
        .unwrap();
    let (stream, _) = listener.accept().unwrap();
    stream
        .set_read_timeout(Some(Duration::from_secs(10)))
        .unwrap();
    let mut peer = Peer {
        stream,
        decoder: FrameDecoder::new(),
    };
    let InterfaceMessage::ConnectionSettings(settings) = peer.recv() else {
        panic!("the first message is the connection settings");
    };
    assert!(!settings.wants_ball_predictions && !settings.wants_comms);
    peer.send(&CoreMessage::ControllableTeamInfo(ControllableTeamInfo {
        team: 0,
        controllables: cars
            .iter()
            .map(|&index| ControllableInfo {
                index,
                identifier: 7,
            })
            .collect(),
    }));
    peer.send(&CoreMessage::MatchConfiguration(
        MatchConfiguration::default(),
    ));
    peer.send(&CoreMessage::FieldInfo(FieldInfo::default()));
    assert!(matches!(peer.recv(), InterfaceMessage::InitComplete(_)));
    Run {
        child,
        peer,
        scenario: Scenario::from_json(scenario_json).unwrap(),
        tape,
    }
}

impl Run {
    fn packet(&mut self, frame_num: u32) {
        self.peer.send(&CoreMessage::GamePacket(GamePacket {
            players: Vec::new(),
            boost_pads: Vec::new(),
            balls: Vec::new(),
            match_info: MatchInfo {
                seconds_elapsed: 0.0,
                game_time_remaining: 300.0,
                is_overtime: false,
                is_unlimited_time: false,
                // Freeplay reports Paused while physics runs; the bots go by the frame counter.
                match_phase: MatchPhase::Paused,
                frame_num,
            },
            teams: Vec::new(),
        }));
    }

    fn input(&mut self) -> PlayerInput {
        match self.peer.recv() {
            InterfaceMessage::PlayerInput(input) => input,
            other => panic!("expected PlayerInput, got {other:?}"),
        }
    }

    /// Ends the run the way core does and checks the process exits cleanly.
    fn finish(&mut self) -> Vec<InterfaceMessage> {
        self.peer.send(&CoreMessage::DisconnectSignal);
        let mut rest = Vec::new();
        while let Some(message) = self.peer.next() {
            rest.push(message);
        }
        let deadline = Instant::now() + Duration::from_secs(10);
        loop {
            if let Some(status) = self.child.try_wait().unwrap() {
                assert!(status.success(), "the bot exited with {status}");
                return rest;
            }
            assert!(Instant::now() < deadline, "the bot did not exit");
            std::thread::sleep(Duration::from_millis(20));
        }
    }
}

fn at(x: f32, y: f32, z: f32) -> Option<PartialVec3> {
    Some(PartialVec3 {
        x: Some(x),
        y: Some(y),
        z: Some(z),
    })
}

#[test]
fn the_tape_bot_sets_the_start_state_then_plays_one_input_per_physics_frame() {
    let mut run = start(
        env!("CARGO_BIN_EXE_rb_tape_bot"),
        &scenario_json(""),
        &[0],
        "bot",
    );

    run.packet(100); // the first packet only records the frame
    run.packet(101); // physics advanced: set the state, press nothing yet
    let InterfaceMessage::DesiredGameState(state) = run.peer.recv() else {
        panic!("the start state comes first");
    };
    let car = state.car_states[0].physics.as_ref().unwrap();
    assert_eq!(car.location, at(10.0, -2000.0, 17.0));
    assert_eq!(state.car_states[0].boost_amount, Some(50.0));
    assert_eq!(state.ball_states[0].physics.location, at(100.0, 0.0, 100.0));
    assert_eq!(run.input(), neutral(0));

    run.packet(101); // a paused game repeats its frame: nothing is sent
    for (tick, frame_num) in [(0, 102), (1, 103), (2, 104), (3, 105)] {
        run.packet(frame_num);
        assert_eq!(
            run.input(),
            PlayerInput {
                player_index: 0,
                controller_state: controller(run.scenario.input_at(tick)),
            },
            "tick {tick}"
        );
    }
    // Literal values, not through `controller` again: step 1 is throttle and jump, step 2 steer
    // left with boost.
    let first = controller(run.scenario.input_at(0));
    assert!(first.throttle == 1.0 && first.jump && first.steer == 0.0 && !first.boost);
    let third = controller(run.scenario.input_at(2));
    assert!(third.steer == -0.5 && third.boost && third.throttle == 0.0 && !third.jump);

    assert_eq!(run.finish(), vec![], "nothing after the tape's frames");
}

#[test]
fn a_tape_bot_that_is_not_car_zero_stays_neutral() {
    let mut run = start(
        env!("CARGO_BIN_EXE_rb_tape_bot"),
        &scenario_json(""),
        &[1],
        "bot1",
    );
    for frame_num in 100..106 {
        run.packet(frame_num);
    }
    assert_eq!(run.finish(), vec![], "only car 0 plays the tape");
}

#[test]
fn the_hive_drives_every_car_from_one_clock() {
    let others = r#","others":[{"car":{"location":[-10,2000,17]},
        "steps":[{"ticks":2,"steer":1.0},{"ticks":2,"pitch":0.5}]}]"#;
    let mut run = start(
        env!("CARGO_BIN_EXE_rb_tape_hive"),
        &scenario_json(others),
        &[0, 1],
        "hive",
    );
    run.packet(200);
    run.packet(201);
    let InterfaceMessage::DesiredGameState(state) = run.peer.recv() else {
        panic!("the start state comes first");
    };
    assert_eq!(state.car_states.len(), 2, "both cars are set");
    assert_eq!(
        state.car_states[1].physics.as_ref().unwrap().location,
        at(-10.0, 2000.0, 17.0)
    );
    assert_eq!(run.input(), neutral(0));
    assert_eq!(run.input(), neutral(1));

    // Frame 203 never arrives: the tape is indexed by physics frame, not by packet count,
    // so the second packet is tick 2 (the pitch step), not tick 1.
    for (tick, frame_num) in [(0, 202), (2, 204)] {
        run.packet(frame_num);
        for car in 0..2usize {
            let input = run.input();
            assert_eq!(input.player_index, car as u32);
            assert_eq!(
                input.controller_state,
                controller(run.scenario.input_at_car(car, tick)),
                "car {car} tick {tick}"
            );
        }
    }
    assert_ne!(
        controller(run.scenario.input_at_car(1, 0)),
        Default::default()
    );
    assert_eq!(run.finish(), vec![]);
}

fn neutral(player_index: u32) -> PlayerInput {
    PlayerInput {
        player_index,
        controller_state: Default::default(),
    }
}
