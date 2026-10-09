//! The bot and hivemind runners against a fake core: who gets constructed with what, what is
//! sent and in which order, and how a run ends.
#![allow(clippy::unwrap_used, clippy::expect_used, clippy::panic)]

mod common;

use std::thread::{self, JoinHandle};

use common::{packet, FakeCore, Peer};
use rb_rlbot_client::{
    run_bots, run_hivemind, Agent, Connection, CoreMessage, Error, InterfaceMessage, Outbox, Result,
};
use rb_rlbot_wire::{
    ConnectionSettings, ControllerState, DesiredGameState, GamePacket, InitComplete, Ping,
    PlayerInput,
};

fn settings() -> ConnectionSettings {
    ConnectionSettings {
        agent_id: "rb/agents".into(),
        wants_ball_predictions: false,
        wants_comms: false,
        close_between_matches: true,
    }
}

/// Steers its car by the packet's frame number, so a test can tell which packet it answered.
struct Steerer {
    car: u32,
}

impl Agent for Steerer {
    fn tick(&mut self, packet: &GamePacket, out: &mut Outbox) {
        out.push(PlayerInput {
            player_index: self.car,
            controller_state: ControllerState {
                throttle: packet.match_info.frame_num as f32,
                ..ControllerState::default()
            },
        });
    }
}

fn throttle(message: InterfaceMessage) -> (u32, f32) {
    match message {
        InterfaceMessage::PlayerInput(i) => (i.player_index, i.controller_state.throttle),
        other => panic!("expected PlayerInput, got {other:?}"),
    }
}

/// Runs `body` as the client on its own thread and hands back its result.
fn client<T: Send + 'static>(
    core: &FakeCore,
    body: impl FnOnce(Connection) -> T + Send + 'static,
) -> (JoinHandle<T>, Peer) {
    let connection = Connection::connect(core.addr()).unwrap();
    let peer = core.accept();
    (thread::spawn(move || body(connection)), peer)
}

#[test]
fn each_car_gets_a_bot_built_for_it_and_every_bot_answers_every_packet() {
    let core = FakeCore::start();
    let (run, mut peer) = client(&core, |mut c| {
        let mut built = Vec::new();
        let result = run_bots(&mut c, settings(), |init, out| {
            built.push((
                init.team,
                init.controllable.index,
                init.controllable.identifier,
            ));
            out.push(DesiredGameState::default());
            Steerer {
                car: init.controllable.index,
            }
        });
        (result, built)
    });

    assert_eq!(
        peer.recv(),
        InterfaceMessage::ConnectionSettings(settings())
    );
    peer.send_starting_info(1, &[5, 6]);
    // What the bots queued while being built, then readiness: in that order, before any tick.
    assert_eq!(
        peer.recv(),
        InterfaceMessage::DesiredGameState(Default::default())
    );
    assert_eq!(
        peer.recv(),
        InterfaceMessage::DesiredGameState(Default::default())
    );
    assert_eq!(peer.recv(), InterfaceMessage::InitComplete(InitComplete));

    for frame in [10, 11] {
        peer.send(&packet(frame));
        assert_eq!(throttle(peer.recv()), (5, frame as f32));
        assert_eq!(throttle(peer.recv()), (6, frame as f32));
    }
    peer.send(&CoreMessage::DisconnectSignal);

    let (result, built) = run.join().unwrap();
    result.unwrap();
    assert_eq!(built, vec![(1, 5, 105), (1, 6, 106)]);
}

#[test]
fn pings_are_answered_with_their_cookie_without_involving_the_agent() {
    let core = FakeCore::start();
    let (run, mut peer) = client(&core, |mut c| {
        run_bots(&mut c, settings(), |_, _| Steerer { car: 0 })
    });
    peer.recv();
    peer.send_starting_info(0, &[0]);
    assert_eq!(peer.recv(), InterfaceMessage::InitComplete(InitComplete));

    peer.send(&CoreMessage::PingRequest(Ping { cookie: u64::MAX }));
    assert_eq!(
        peer.recv(),
        InterfaceMessage::PingResponse(Ping { cookie: u64::MAX })
    );
    peer.send(&CoreMessage::DisconnectSignal);
    run.join().unwrap().unwrap();
}

#[test]
fn messages_an_agent_does_not_handle_are_ignored_and_the_loop_goes_on() {
    let core = FakeCore::start();
    let (run, mut peer) = client(&core, |mut c| {
        run_bots(&mut c, settings(), |_, _| Steerer { car: 0 })
    });
    peer.recv();
    peer.send_starting_info(0, &[0]);
    peer.recv(); // InitComplete

    // Repeated setup messages and a payload tag this crate does not model.
    peer.send_starting_info(0, &[0]);
    let mut other = CoreMessage::PingRequest(Ping { cookie: 1 })
        .to_payload()
        .unwrap();
    let tag_at = other.iter().position(|b| *b == 9).unwrap();
    other[tag_at] = 8;
    peer.send_bytes(&rb_rlbot_wire::frame::frame(&other).unwrap());

    peer.send(&packet(3));
    assert_eq!(throttle(peer.recv()), (0, 3.0));
    peer.send(&CoreMessage::DisconnectSignal);
    run.join().unwrap().unwrap();
}

#[test]
fn a_connection_with_no_cars_ends_quietly_without_announcing_readiness() {
    let core = FakeCore::start();
    let (run, mut peer) = client(&core, |mut c| {
        let mut built = 0;
        let result = run_bots(&mut c, settings(), |_, _| {
            built += 1;
            Steerer { car: 0 }
        });
        (result, built)
    });
    peer.recv();
    peer.send_starting_info(0, &[]);
    let (result, built) = run.join().unwrap();
    result.unwrap();
    assert_eq!(built, 0);
    // The client is gone and sent nothing after the handshake.
    assert!(peer.is_closed());
}

#[test]
fn a_hivemind_is_one_agent_that_sees_the_whole_starting_info() {
    struct Hive {
        cars: Vec<u32>,
    }
    impl Agent for Hive {
        fn tick(&mut self, packet: &GamePacket, out: &mut Outbox) {
            for &car in &self.cars {
                out.push(PlayerInput {
                    player_index: car,
                    controller_state: ControllerState {
                        steer: packet.match_info.frame_num as f32,
                        ..ControllerState::default()
                    },
                });
            }
        }
    }

    let core = FakeCore::start();
    let (run, mut peer) = client(&core, |mut c| {
        let mut made = 0;
        let result = run_hivemind(&mut c, settings(), |info, _| {
            made += 1;
            Hive {
                cars: info
                    .controllable_team_info
                    .controllables
                    .iter()
                    .map(|c| c.index)
                    .collect(),
            }
        });
        (result, made)
    });
    peer.recv();
    peer.send_starting_info(0, &[2, 3, 4]);
    assert_eq!(peer.recv(), InterfaceMessage::InitComplete(InitComplete));

    peer.send(&packet(8));
    for car in [2, 3, 4] {
        match peer.recv() {
            InterfaceMessage::PlayerInput(i) => {
                assert_eq!(i.player_index, car);
                assert_eq!(i.controller_state.steer, 8.0);
            }
            other => panic!("unexpected {other:?}"),
        }
    }
    peer.send(&CoreMessage::DisconnectSignal);
    let (result, made) = run.join().unwrap();
    result.unwrap();
    assert_eq!(made, 1);
}

#[test]
fn replies_to_one_packet_keep_the_order_of_the_cars() {
    let core = FakeCore::start();
    let (run, mut peer) = client(&core, |mut c| {
        run_bots(&mut c, settings(), |init, _| Steerer {
            car: init.controllable.index,
        })
    });
    peer.recv();
    peer.send_starting_info(0, &[0, 1]);
    peer.recv();
    peer.send(&packet(1));
    assert_eq!(throttle(peer.recv()).0, 0);
    assert_eq!(throttle(peer.recv()).0, 1);
    peer.send(&CoreMessage::DisconnectSignal);
    run.join().unwrap().unwrap();
}

#[test]
fn core_vanishing_mid_run_is_an_error_not_a_clean_exit() {
    let core = FakeCore::start();
    let (run, mut peer) = client(&core, |mut c| {
        run_bots(&mut c, settings(), |_, _| Steerer { car: 0 })
    });
    peer.recv();
    peer.send_starting_info(0, &[0]);
    peer.recv();
    drop(peer);
    let result: Result<()> = run.join().unwrap();
    assert!(matches!(result, Err(Error::Closed)));
}

#[test]
fn a_disconnect_before_the_handshake_ends_is_an_error() {
    let core = FakeCore::start();
    let (run, mut peer) = client(&core, |mut c| {
        run_hivemind(&mut c, settings(), |_, _| Steerer { car: 0 })
    });
    peer.recv();
    peer.send(&CoreMessage::DisconnectSignal);
    assert!(matches!(run.join().unwrap(), Err(Error::Disconnected)));
}
