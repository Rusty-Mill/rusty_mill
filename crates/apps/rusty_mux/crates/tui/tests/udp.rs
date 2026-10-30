//! The UDP relay against a real server with `--udp`: a client moves its
//! control connection from a socket onto a stream, keeps it through a
//! change of address, and opens more streams beside it.

#![cfg(target_os = "linux")]

use std::net::{SocketAddr, UdpSocket};
use std::os::unix::net::UnixStream;
use std::path::PathBuf;
use std::sync::mpsc;
use std::time::Duration;

use rusty_mux_proto::{Features, Message, Takeover};
use rusty_mux_tui::Conn;
use rusty_mux_tui::udp::Relay;

const PATIENCE: Duration = Duration::from_secs(10);
const FEATURES: Features = Features::CHECKPOINT_V6
    .union(Features::SYNC)
    .union(Features::TAKEOVER)
    .union(Features::DATAGRAM);

/// A real server on a fresh socket and a UDP port on 127.0.0.1.
fn udp_server() -> (PathBuf, SocketAddr) {
    use rusty_mux_server::{Config, Udp, serve_udp};
    use rusty_tokio::io::{UdpSocket, UnixListener};
    use rusty_tokio::{Builder, LocalOptions};

    let dir = std::env::temp_dir().join(format!(
        "rusty-mux-tui-udp-{}-{}",
        std::process::id(),
        rusty_uuid::Uuid::new_v4().simple()
    ));
    std::fs::create_dir(&dir).expect("dir");
    let path = dir.join("s");
    let (ready_tx, ready_rx) = mpsc::channel();
    let bind = path.clone();
    std::thread::spawn(move || {
        let runtime = Builder::new_current_thread()
            .build_local(&LocalOptions::default())
            .expect("runtime");
        runtime.block_on(async move {
            let listener = UnixListener::bind(&bind).expect("bind");
            let socket = UdpSocket::bind("127.0.0.1:0").await.expect("udp");
            ready_tx
                .send(socket.local_addr().expect("addr"))
                .expect("ready");
            let config = Config {
                program: "/bin/sh".into(),
                args: Vec::new(),
                cwd: "/".into(),
            };
            let udp = Udp {
                socket,
                idle: PATIENCE * 6,
            };
            serve_udp(listener, config, udp).await.expect("serve");
        });
    });
    let addr = ready_rx.recv_timeout(PATIENCE).expect("server bound");
    (path, addr)
}

fn connect(path: &std::path::Path, takeover: Option<Takeover>) -> (Conn<UnixStream>, Takeover) {
    let stream = UnixStream::connect(path).expect("connect");
    stream.set_read_timeout(Some(PATIENCE)).expect("timeout");
    let mut conn = Conn::new(stream);
    let hello = Message::Hello {
        features: FEATURES,
        resume: None,
        takeover,
    };
    conn.send(&hello).expect("hello");
    match conn.recv().expect("welcome") {
        Message::Welcome {
            features,
            client,
            token: Some(token),
            ..
        } => {
            assert!(features.contains(Features::DATAGRAM));
            (conn, Takeover { client, token })
        }
        other => panic!("expected a Welcome with a token, got {other:?}"),
    }
}

/// The next message that is not an event.
fn reply(conn: &mut Conn<UnixStream>) -> Message {
    loop {
        match conn.recv().expect("an answer") {
            Message::Event { .. } => {}
            other => return other,
        }
    }
}

/// Asks for an association: its id and key, and the port it is on.
fn open_datagram(conn: &mut Conn<UnixStream>) -> (u16, [u8; 8], [u8; 32]) {
    conn.send(&Message::OpenDatagram { req: 1 }).expect("open");
    match reply(conn) {
        Message::Datagram { port, id, key, .. } => (port, id, key.0),
        other => panic!("expected Datagram, got {other:?}"),
    }
}

fn sync(conn: &mut Conn<UnixStream>, req: u64) {
    conn.send(&Message::Sync { req }).expect("sync");
    match reply(conn) {
        Message::Done { req: r, .. } if r == req => {}
        other => panic!("expected Done, got {other:?}"),
    }
}

fn bind(ip: [u8; 4]) -> SocketAddr {
    SocketAddr::from((ip, 0))
}

#[test]
fn a_moved_connection_keeps_its_client_through_a_roam() {
    let (path, udp) = udp_server();
    let (mut over_ssh, me) = connect(&path, None);
    let (port, id, key) = open_datagram(&mut over_ssh);
    assert_eq!(port, udp.port());
    let relay = Relay::start(bind([127, 0, 0, 2]), udp, id, &key).expect("relay");
    assert!(relay.probe(Duration::from_secs(2)));

    let (mut moved, again) = connect(relay.socket(), Some(me));
    assert_eq!(again.client, me.client, "the same client, taken over");
    loop {
        match over_ssh.recv() {
            Ok(Message::Event { .. }) => {}
            Ok(other) => panic!("the old connection got {other:?}"),
            Err(_) => break,
        }
    }
    sync(&mut moved, 2);

    relay.rebind(bind([127, 0, 0, 3])).expect("rebind");
    sync(&mut moved, 3);
    // A second stream on the same association is a client of its own.
    let (mut other, other_me) = connect(relay.socket(), None);
    assert_ne!(other_me.client, me.client);
    sync(&mut other, 1);
    sync(&mut moved, 4);
}

#[test]
fn a_blocked_port_fails_the_probe_within_its_patience() {
    let (path, udp) = udp_server();
    let (mut conn, _) = connect(&path, None);
    let (_, id, key) = open_datagram(&mut conn);
    // The right key, sent where nothing answers.
    let blackhole = UdpSocket::bind("127.0.0.1:0").expect("bind");
    let nowhere = blackhole.local_addr().expect("addr");
    assert_ne!(nowhere, udp);
    let relay = Relay::start(bind([127, 0, 0, 1]), nowhere, id, &key).expect("relay");
    let started = std::time::Instant::now();
    assert!(!relay.probe(Duration::from_secs(2)));
    assert!(started.elapsed() < Duration::from_secs(3));
    // The control connection never moved, so it still works.
    sync(&mut conn, 2);
}
