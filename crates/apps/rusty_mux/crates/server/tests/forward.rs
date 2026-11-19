//! Port forwards (C81): a tunnel from the input-lease holder to a loopback
//! port of the server's host.

#![cfg(any(target_os = "linux", windows))]

mod common;

use std::io::{Read, Write};
use std::net::{Ipv4Addr, TcpListener};
use std::sync::mpsc;
use std::thread;
use std::time::Duration;

use common::{Client, block_for, start_server};
use rusty_mux_core::BlockId;
use rusty_mux_proto::{Features, Message};

const FORWARD: Features = Features::CHECKPOINT_V6.union(Features::PORT_FORWARD);
const PATIENCE: Duration = Duration::from_secs(10);

fn open(client: &mut Client, block: BlockId, port: u16, id: u64) -> Vec<Message> {
    client.answer(|req| Message::OpenForward {
        req,
        block,
        port,
        id,
    })
}

/// A loopback service that echoes one connection; it reports how that
/// connection ended: `true` when the peer closed it.
fn echo() -> (u16, mpsc::Receiver<bool>) {
    let listener = TcpListener::bind((Ipv4Addr::LOCALHOST, 0)).expect("bind");
    let port = listener.local_addr().expect("addr").port();
    let (ended, rx) = mpsc::channel();
    thread::spawn(move || {
        let (mut peer, _) = listener.accept().expect("accept");
        peer.set_read_timeout(Some(PATIENCE)).expect("timeout");
        let mut buf = [0u8; 4096];
        loop {
            match peer.read(&mut buf) {
                Ok(0) => {
                    let _ = ended.send(true);
                    return;
                }
                Ok(n) => peer.write_all(&buf[..n]).expect("echo"),
                Err(_) => {
                    let _ = ended.send(false);
                    return;
                }
            }
        }
    });
    (port, rx)
}

/// What forward `id` has sent `client` until `want` bytes arrived, and
/// the credit it gave back.
fn collect(client: &mut Client, id: u64, want: usize) -> (Vec<u8>, usize) {
    let (mut got, mut acked) = (Vec::new(), 0);
    while got.len() < want {
        match client.recv() {
            Message::ForwardData { id: i, data } if i == id => {
                client.send(&Message::ForwardAck {
                    id,
                    n: data.len() as u32,
                });
                got.extend(data);
            }
            Message::ForwardAck { id: i, n } if i == id => acked += n as usize,
            Message::CloseForward { id: i, reason } if i == id => {
                panic!("forward closed early: {reason:?}")
            }
            _ => {}
        }
    }
    (got, acked)
}

#[test]
fn a_lease_holder_tunnels_to_a_loopback_port_and_back() {
    let (port, ended) = echo();
    let mut client = Client::connect_with(&start_server(), FORWARD, None);
    let block = block_for(&mut client);
    assert!(matches!(
        open(&mut client, block, port, 7).as_slice(),
        [Message::Done { .. }]
    ));
    client.send(&Message::ForwardData {
        id: 7,
        data: b"hello through".to_vec(),
    });
    let (got, acked) = collect(&mut client, 7, 13);
    assert_eq!(got, b"hello through");
    // The server gave the 13 bytes back as credit, before or after the echo.
    let mut acked = acked;
    while acked < 13 {
        if let Message::ForwardAck { id: 7, n } = client.recv() {
            acked += n as usize;
        }
    }
    assert_eq!(acked, 13);
    client.send(&Message::CloseForward {
        id: 7,
        reason: None,
    });
    assert!(
        ended.recv_timeout(PATIENCE).expect("ended"),
        "closed by the peer"
    );
}

#[test]
fn a_connection_that_ends_closes_its_forwards() {
    let (port, ended) = echo();
    let mut client = Client::connect_with(&start_server(), FORWARD, None);
    let block = block_for(&mut client);
    open(&mut client, block, port, 1);
    client.send(&Message::ForwardData {
        id: 1,
        data: b"x".to_vec(),
    });
    collect(&mut client, 1, 1);
    drop(client);
    assert!(
        ended.recv_timeout(PATIENCE).expect("ended"),
        "closed with the client"
    );
}

#[test]
fn a_refused_dial_closes_the_forward_with_a_reason() {
    // A port nothing listens on: bound, then released.
    let port = TcpListener::bind((Ipv4Addr::LOCALHOST, 0))
        .expect("bind")
        .local_addr()
        .expect("addr")
        .port();
    let mut client = Client::connect_with(&start_server(), FORWARD, None);
    let block = block_for(&mut client);
    assert!(matches!(
        open(&mut client, block, port, 2).as_slice(),
        [Message::Done { .. }]
    ));
    loop {
        if let Message::CloseForward { id: 2, reason } = client.recv() {
            assert!(reason.is_some_and(|r| !r.is_empty()));
            return;
        }
    }
}

#[test]
fn only_the_lease_holder_with_the_feature_may_open_one() {
    let (port, _ended) = echo();
    let path = start_server();
    let mut holder = Client::connect_with(&path, FORWARD, None);
    let block = block_for(&mut holder);

    // Attached, but another client holds the lease.
    let mut viewer = Client::connect_with(&path, FORWARD, None);
    viewer.request(|req| Message::Attach { req, block });
    assert!(matches!(
        open(&mut viewer, block, port, 1).as_slice(),
        [Message::Failed { .. }]
    ));

    // Without the bit the server does not know the message.
    let mut old = Client::connect_with(&path, Features::CHECKPOINT_V6, None);
    old.request(|req| Message::Attach { req, block });
    assert!(matches!(
        open(&mut old, block, port, 1).as_slice(),
        [Message::Failed { .. }]
    ));

    // Port 0 is no port.
    assert!(matches!(
        open(&mut holder, block, 0, 1).as_slice(),
        [Message::Failed { .. }]
    ));
}
