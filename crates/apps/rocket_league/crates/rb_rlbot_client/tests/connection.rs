//! The connection against a fake core: framing across arbitrary reads, timeouts that keep
//! partial frames, the handshake, and every way a connection can end.
#![allow(clippy::unwrap_used, clippy::expect_used, clippy::panic)]

mod common;

use std::thread;
use std::time::Duration;

use common::{packet, FakeCore};
use rb_rlbot_client::{Connection, CoreMessage, Environment, Error, InterfaceMessage};
use rb_rlbot_wire::frame::frame;
use rb_rlbot_wire::{
    ConnectionSettings, DesiredGameState, InitComplete, MatchConfiguration, Ping, PlayerInput,
    StopCommand,
};

fn settings() -> ConnectionSettings {
    ConnectionSettings {
        agent_id: "rb/test".into(),
        wants_ball_predictions: false,
        wants_comms: true,
        close_between_matches: true,
    }
}

#[test]
fn every_client_message_reaches_core_intact() {
    let core = FakeCore::start();
    let mut client = Connection::connect(core.addr()).unwrap();
    let mut peer = core.accept();

    let messages = vec![
        InterfaceMessage::ConnectionSettings(settings()),
        InterfaceMessage::MatchConfiguration(MatchConfiguration::default()),
        InterfaceMessage::DesiredGameState(DesiredGameState::default()),
        InterfaceMessage::PlayerInput(PlayerInput {
            player_index: 2,
            ..PlayerInput::default()
        }),
        InterfaceMessage::StopCommand(StopCommand {
            shutdown_server: true,
        }),
        InterfaceMessage::InitComplete(InitComplete),
    ];
    // One write for the batch, one for a single message: core sees them in order either way.
    client.send_all(&messages[..4]).unwrap();
    client.send(messages[4].clone()).unwrap();
    client.send(InitComplete).unwrap();
    for expected in messages {
        assert_eq!(peer.recv(), expected);
    }
}

#[test]
fn a_message_split_into_single_bytes_is_reassembled() {
    let core = FakeCore::start();
    let mut client = Connection::connect(core.addr()).unwrap();
    let mut peer = core.accept();

    let expected = packet(77);
    let bytes = frame(&expected.to_payload().unwrap()).unwrap();
    let sender = thread::spawn(move || {
        for byte in bytes {
            peer.send_bytes(&[byte]);
        }
        peer
    });
    assert_eq!(client.recv().unwrap(), expected);
    sender.join().unwrap();
}

#[test]
fn two_messages_in_one_read_come_out_one_at_a_time() {
    let core = FakeCore::start();
    let mut client = Connection::connect(core.addr()).unwrap();
    let mut peer = core.accept();

    let (a, b) = (packet(1), packet(2));
    let mut bytes = frame(&a.to_payload().unwrap()).unwrap();
    bytes.extend(frame(&b.to_payload().unwrap()).unwrap());
    peer.send_bytes(&bytes);
    assert_eq!(client.recv().unwrap(), a);
    assert_eq!(client.recv().unwrap(), b);
}

#[test]
fn a_timeout_in_the_middle_of_a_frame_loses_nothing() {
    let core = FakeCore::start();
    let mut client = Connection::connect(core.addr()).unwrap();
    let mut peer = core.accept();

    assert_eq!(
        client.recv_timeout(Duration::from_millis(30)).unwrap(),
        None,
        "silence is a timeout, not an error"
    );

    let expected = packet(9);
    let bytes = frame(&expected.to_payload().unwrap()).unwrap();
    let (head, tail) = bytes.split_at(bytes.len() / 2);
    peer.send_bytes(head);
    assert_eq!(
        client.recv_timeout(Duration::from_millis(30)).unwrap(),
        None
    );
    peer.send_bytes(tail);
    assert_eq!(
        client.recv_timeout(Duration::from_secs(5)).unwrap(),
        Some(expected)
    );
}

#[test]
fn a_zero_timeout_polls_instead_of_blocking_forever() {
    let core = FakeCore::start();
    let mut client = Connection::connect(core.addr()).unwrap();
    let _peer = core.accept();
    assert_eq!(client.recv_timeout(Duration::ZERO).unwrap(), None);
}

#[test]
fn handshake_sends_settings_first_then_gathers_the_three_messages_in_any_order() {
    let core = FakeCore::start();
    let mut client = Connection::connect(core.addr()).unwrap();
    let mut peer = core.accept();

    let core_side = thread::spawn(move || {
        assert_eq!(
            peer.recv(),
            InterfaceMessage::ConnectionSettings(settings())
        );
        // Field info first, a game packet and an unmodelled message in between, a ping to answer.
        peer.send(&CoreMessage::FieldInfo(Default::default()));
        peer.send(&packet(1));
        peer.send(&CoreMessage::PingRequest(Ping { cookie: 42 }));
        peer.send(&CoreMessage::MatchConfiguration(Default::default()));
        assert_eq!(
            peer.recv(),
            InterfaceMessage::PingResponse(Ping { cookie: 42 })
        );
        peer.send(&CoreMessage::ControllableTeamInfo(common::team_info(
            1,
            &[3, 4],
        )));
        peer
    });

    let info = client.handshake(settings()).unwrap();
    assert_eq!(info.controllable_team_info, common::team_info(1, &[3, 4]));
    core_side.join().unwrap();
}

#[test]
fn a_disconnect_signal_during_the_handshake_is_reported() {
    let core = FakeCore::start();
    let mut client = Connection::connect(core.addr()).unwrap();
    let mut peer = core.accept();
    peer.send(&CoreMessage::DisconnectSignal);
    assert!(matches!(
        client.handshake(settings()),
        Err(Error::Disconnected)
    ));
}

#[test]
fn core_closing_the_socket_is_closed_not_a_hang() {
    let core = FakeCore::start();
    let mut client = Connection::connect(core.addr()).unwrap();
    drop(core.accept());
    assert!(matches!(client.recv(), Err(Error::Closed)));
    assert!(matches!(
        client.recv_timeout(Duration::from_secs(5)),
        Err(Error::Closed)
    ));
}

#[test]
fn core_closing_in_the_middle_of_a_frame_is_closed() {
    let core = FakeCore::start();
    let mut client = Connection::connect(core.addr()).unwrap();
    let mut peer = core.accept();
    let bytes = frame(&packet(5).to_payload().unwrap()).unwrap();
    peer.send_bytes(&bytes[..bytes.len() - 1]);
    drop(peer);
    assert!(matches!(client.recv(), Err(Error::Closed)));
}

#[test]
fn bytes_that_are_not_a_message_are_a_protocol_error_and_the_next_message_still_reads() {
    let core = FakeCore::start();
    let mut client = Connection::connect(core.addr()).unwrap();
    let mut peer = core.accept();

    peer.send_bytes(&frame(&[1, 2, 3]).unwrap());
    assert!(matches!(client.recv(), Err(Error::Wire(_))));

    let good = packet(3);
    peer.send(&good);
    assert_eq!(client.recv().unwrap(), good);
}

#[test]
fn connecting_where_nothing_listens_is_an_io_error() {
    let core = FakeCore::start();
    let addr = core.addr();
    drop(core);
    assert!(matches!(Connection::connect(addr), Err(Error::Io(_))));
}

#[test]
fn environment_prefers_the_full_address_and_falls_back_field_by_field() {
    let env = |vars: &'static [(&str, &str)]| {
        Environment::from_lookup(move |name| {
            vars.iter()
                .find(|(k, _)| *k == name)
                .map(|(_, v)| v.to_string())
        })
    };
    assert_eq!(
        env(&[]),
        Environment {
            server_addr: "127.0.0.1:23234".into(),
            agent_id: None
        }
    );
    assert_eq!(
        env(&[("RLBOT_SERVER_IP", "10.0.0.2"), ("RLBOT_SERVER_PORT", "5")]).server_addr,
        "10.0.0.2:5"
    );
    let full = env(&[
        ("RLBOT_SERVER_ADDR", "core:9"),
        ("RLBOT_SERVER_PORT", "5"),
        ("RLBOT_AGENT_ID", "me"),
    ]);
    assert_eq!(full.server_addr, "core:9");
    assert_eq!(full.agent_id.as_deref(), Some("me"));
    // An empty id is unset, as RLBot launches processes with the variable present but empty.
    assert_eq!(env(&[("RLBOT_AGENT_ID", "")]).agent_id, None);
}
