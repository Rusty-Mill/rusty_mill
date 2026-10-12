//! The loop, listener and process probe against a simulated game and fake connections.
#![allow(clippy::unwrap_used, clippy::expect_used, clippy::panic)]

use std::net::TcpStream;
use std::time::Duration;

use rb_rl_game_bridge::{
    find_usable_game_port, rlbot_args, tcp_port_is_free, BridgeLoop, GameCommand, GameLink,
    GameListener, ProcessSource, Roster, SimGameLink, SystemProcesses,
};
use rb_rlbot_core::Simulation;
use rb_rlbot_wire::{
    ControllerState, CustomBot, MatchConfiguration, MatchPhase, PlayerClass, PlayerConfiguration,
};

fn config() -> MatchConfiguration {
    let bot = |id: &str, team, player_id| PlayerConfiguration {
        variety: PlayerClass::CustomBot(CustomBot {
            name: id.into(),
            root_dir: String::new(),
            run_command: String::new(),
            agent_id: id.into(),
            hivemind: false,
            environment: None,
        }),
        team,
        player_id,
    };
    MatchConfiguration {
        player_configurations: vec![bot("a", 0, 1), bot("b", 1, 2)],
        ..MatchConfiguration::default()
    }
}

#[test]
fn the_loop_distributes_every_message_that_advances_time() {
    let sim = Simulation::new(config()).unwrap();
    let roster = Roster::from_configuration(sim.configuration());
    let mut link = SimGameLink::ending_after(sim, 300);
    let mut bridge = BridgeLoop::new();
    bridge.set_roster(Some(roster));

    let mut packets = Vec::new();
    bridge.run(&mut link, |p| packets.push(p)).unwrap();

    // The first message (the map load) is at time 0, so time has not advanced past the start;
    // every later one is a new tick.
    assert_eq!(packets.len(), 299);
    assert!(packets
        .windows(2)
        .all(|w| w[1].match_info.frame_num == w[0].match_info.frame_num + 1));
    assert_eq!(packets[0].match_info.match_phase, MatchPhase::Countdown);
    assert_eq!(bridge.tracker().ticks_skipped(), 0);
}

#[test]
fn without_a_roster_nothing_is_distributed() {
    let mut link = SimGameLink::ending_after(Simulation::new(config()).unwrap(), 50);
    let mut seen = 0;
    BridgeLoop::new().run(&mut link, |_| seen += 1).unwrap();
    assert_eq!(seen, 0);
}

#[test]
fn a_map_load_is_reported_once_and_restarts_the_counter() {
    let mut link = SimGameLink::ending_after(Simulation::new(config()).unwrap(), 5);
    let mut bridge = BridgeLoop::new();
    let loads: Vec<bool> = std::iter::from_fn(|| link.next_clump().unwrap())
        .map(|clump| bridge.on_clump(clump).map_loaded)
        .collect();
    assert_eq!(loads, [true, false, false, false, false]);
    assert_eq!(bridge.tracker().ticks_since_map_load(), 4);
}

#[test]
fn input_reaches_the_simulated_game_and_a_bad_car_or_console_is_refused() {
    let mut link = SimGameLink::new(Simulation::new(config()).unwrap());
    let state = ControllerState {
        throttle: 1.0,
        ..ControllerState::default()
    };
    link.send(GameCommand::PlayerInput {
        player_index: 1,
        state,
    })
    .unwrap();
    let err = link
        .send(GameCommand::PlayerInput {
            player_index: 9,
            state,
        })
        .unwrap_err();
    assert_eq!(err.kind(), std::io::ErrorKind::InvalidInput);
    let err = link.send(GameCommand::Console("rlbot".into())).unwrap_err();
    assert_eq!(err.kind(), std::io::ErrorKind::Unsupported);
}

#[test]
fn the_listener_accepts_the_game_dialling_in_and_times_out_otherwise() {
    let listener = GameListener::bind(0).unwrap();
    assert!(listener
        .wait_for_connection(Duration::from_millis(50))
        .unwrap()
        .is_none());
    let addr = listener.local_addr().unwrap();
    let _game = TcpStream::connect(addr).unwrap();
    assert!(listener
        .wait_for_connection(Duration::from_secs(5))
        .unwrap()
        .is_some());
}

#[test]
fn the_game_is_told_to_dial_the_port_the_listener_holds() {
    let listener = GameListener::bind(0).unwrap();
    let port = listener.local_addr().unwrap().port();
    assert!(!tcp_port_is_free(port), "a listener holds it");
    let url = &rlbot_args(port)[1];
    assert_eq!(url, &format!("RLBot_ControllerURL=127.0.0.1:{port}"));
    // The port finder would skip a held port and so avoid colliding with the listener.
    let chosen = find_usable_game_port(1, &[], |p| p != port);
    assert_ne!(chosen, port);
}

#[test]
fn the_process_probe_runs_and_sees_no_game_here() {
    // A smoke test of the real probe (PowerShell on Windows, /proc on Linux). It must not
    // fail, and nobody runs Rocket League inside CI.
    let lines = SystemProcesses.rocket_league_command_lines().unwrap();
    assert!(
        lines.iter().all(|l| !l.is_empty()),
        "blank lines are filtered out"
    );
}
