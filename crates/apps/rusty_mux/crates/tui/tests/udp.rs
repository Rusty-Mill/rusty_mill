//! The UDP relay against a real server with `--udp`: a client moves its
//! control connection from a socket onto a stream, keeps it through a
//! change of address, and opens more streams beside it.

#![cfg(target_os = "linux")]

use std::net::{SocketAddr, UdpSocket};
use std::os::unix::net::UnixStream;
use std::path::PathBuf;
use std::sync::atomic::{AtomicUsize, Ordering};
use std::sync::{Arc, Mutex, mpsc};
use std::time::Duration;

use rusty_mux_core::{BlockId, SessionId, Size, WindowId};
use rusty_mux_proto::{Entry, Features, Launch, Message, Takeover};
use rusty_mux_tui::Conn;
use rusty_mux_tui::udp::{Relay, reach};

const PATIENCE: Duration = Duration::from_secs(10);
const FEATURES: Features = Features::CHECKPOINT_V6
    .union(Features::INSPECT)
    .union(Features::SYNC)
    .union(Features::TAKEOVER)
    .union(Features::DATAGRAM);

/// A real server on a fresh socket and a UDP port on 127.0.0.1.
fn udp_server() -> (PathBuf, SocketAddr) {
    use rusty_mux_server::{Config, ConsoleHost, Udp, serve_udp};
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
                console_host: ConsoleHost::default(),
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

/// Whether the server has a session called `name`.
fn has_session(path: &std::path::Path, name: &str) -> bool {
    let (mut conn, _) = connect(path, None);
    conn.send(&Message::List { req: 9 }).expect("list");
    loop {
        match reply(&mut conn) {
            Message::Entry {
                entry: Entry::Session { name: n, .. },
                ..
            } if n == name => return true,
            Message::Entry { .. } => {}
            Message::Done { req: 9, .. } => return false,
            other => panic!("expected List entries, got {other:?}"),
        }
    }
}

#[test]
fn the_last_request_reaches_the_server_when_the_relay_drops() {
    let (path, udp) = udp_server();
    let (mut over_ssh, _) = connect(&path, None);
    let (_, id, key) = open_datagram(&mut over_ssh);
    let relay = Relay::start(bind([127, 0, 0, 1]), udp, id, &key).expect("relay");
    assert!(relay.probe(Duration::from_secs(2)));
    let (mut conn, _) = connect(relay.socket(), None);
    // Sent and hung up at once, as a client's last Detach is.
    let create = Message::CreateSession {
        req: 1,
        name: "last-words".into(),
    };
    conn.send(&create).expect("create");
    conn.get_ref()
        .shutdown(std::net::Shutdown::Both)
        .expect("hang up");
    drop(relay);
    assert!(has_session(&path, "last-words"), "the request was lost");
}

#[test]
fn a_host_whose_first_address_is_silent_is_reached_on_the_next() {
    let (path, udp) = udp_server();
    let (mut over_ssh, me) = connect(&path, None);
    let (_, id, key) = open_datagram(&mut over_ssh);
    let blackhole = UdpSocket::bind("127.0.0.1:0").expect("bind");
    let silent = blackhole.local_addr().expect("addr");
    let started = std::time::Instant::now();
    let relay = reach(&[silent, udp], id, &key, Duration::from_secs(2)).expect("reach");
    assert!(started.elapsed() < Duration::from_secs(3));
    // The address that answered carries the move.
    let (mut moved, again) = connect(relay.socket(), Some(me));
    assert_eq!(again.client, me.client);
    sync(&mut moved, 2);

    let none = reach(&[silent], id, &key, Duration::from_millis(300));
    assert!(none.is_err());
}

/// A path to `server` the client sends to at `addr`, which drops the next
/// `lose` datagrams the client sends, passes the server's back only while
/// `answers`, and records the sequence number of each client datagram.
struct Path {
    addr: SocketAddr,
    lose: Arc<AtomicUsize>,
    seqs: Arc<Mutex<Vec<u64>>>,
}

fn path(server: SocketAddr, answers: bool) -> Path {
    let near = UdpSocket::bind("127.0.0.1:0").expect("bind");
    let far = UdpSocket::bind("127.0.0.1:0").expect("bind");
    let addr = near.local_addr().expect("addr");
    let lose = Arc::new(AtomicUsize::new(0));
    let seqs = Arc::new(Mutex::new(Vec::new()));
    let client = Arc::new(Mutex::new(None));
    let (dropping, seen, client_up) = (lose.clone(), seqs.clone(), client.clone());
    let (near_up, far_up) = (
        near.try_clone().expect("clone"),
        far.try_clone().expect("clone"),
    );
    std::thread::spawn(move || {
        let mut buf = vec![0; 1 << 16];
        while let Ok((n, from)) = near_up.recv_from(&mut buf) {
            *client_up.lock().expect("client") = Some(from);
            if let Some(seq) = buf.get(8..16).filter(|_| n >= 16) {
                let seq = u64::from_be_bytes(seq.try_into().expect("eight bytes"));
                seen.lock().expect("seqs").push(seq);
            }
            let lost = dropping
                .fetch_update(Ordering::AcqRel, Ordering::Acquire, |n| n.checked_sub(1))
                .is_ok();
            if !lost {
                let _ = far_up.send_to(&buf[..n], server);
            }
        }
    });
    std::thread::spawn(move || {
        let mut buf = vec![0; 1 << 16];
        while let Ok(n) = far.recv(&mut buf) {
            let to = *client.lock().expect("client");
            if let Some(to) = to.filter(|_| answers) {
                let _ = near.send_to(&buf[..n], to);
            }
        }
    });
    Path { addr, lose, seqs }
}

#[test]
fn the_last_request_survives_losing_its_first_datagrams() {
    let (server, udp) = udp_server();
    let (mut over_ssh, _) = connect(&server, None);
    let (_, id, key) = open_datagram(&mut over_ssh);
    let lossy = path(udp, true);
    let relay = Relay::start(bind([127, 0, 0, 1]), lossy.addr, id, &key).expect("relay");
    assert!(relay.probe(Duration::from_secs(2)));
    let (mut conn, _) = connect(relay.socket(), None);
    // The request and the stream's first retries are lost on the way.
    lossy.lose.store(2, Ordering::Release);
    let create = Message::CreateSession {
        req: 1,
        name: "resent".into(),
    };
    conn.send(&create).expect("create");
    conn.get_ref()
        .shutdown(std::net::Shutdown::Both)
        .expect("hang up");
    drop(relay);
    assert_eq!(lossy.lose.load(Ordering::Acquire), 0, "nothing was lost");
    assert!(has_session(&server, "resent"), "the request was not resent");
}

/// Sends `make(req)` and waits for its answer: `Done`'s created id, or
/// the reason it failed.
fn request(
    conn: &mut Conn<UnixStream>,
    req: u64,
    make: impl FnOnce(u64) -> Message,
) -> Result<Option<rusty_uuid::Uuid>, String> {
    conn.send(&make(req)).expect("send");
    loop {
        match conn.recv().expect("an answer") {
            Message::Done { req: r, created } if r == req => return Ok(created),
            Message::Failed { req: r, reason, .. } if r == req => return Err(reason),
            _ => {}
        }
    }
}

/// A block that `conn` creates, attaches to and leases.
fn leased_block(conn: &mut Conn<UnixStream>) -> BlockId {
    let created = |r: Result<Option<rusty_uuid::Uuid>, String>| r.expect("done").expect("an id");
    let session = SessionId::from_uuid(created(request(conn, 10, |req| Message::CreateSession {
        req,
        name: "leased".into(),
    })));
    let window = WindowId::from_uuid(created(request(conn, 11, |req| Message::CreateWindow {
        req,
        session,
        name: "w".into(),
    })));
    let size = Size::new(80, 24).expect("size");
    let block = BlockId::from_uuid(created(request(conn, 12, |req| Message::CreateBlock {
        req,
        window,
        size,
        launch: Launch::default(),
    })));
    request(conn, 13, |req| Message::Attach { req, block }).expect("attach");
    request(conn, 14, |req| Message::AcquireLease { req, block }).expect("lease");
    block
}

#[test]
fn a_lost_detach_is_resent_while_output_keeps_arriving() {
    let (server, udp) = udp_server();
    let (mut over_ssh, _) = connect(&server, None);
    let (_, id, key) = open_datagram(&mut over_ssh);
    let lossy = path(udp, true);
    let relay = Relay::start(bind([127, 0, 0, 1]), lossy.addr, id, &key).expect("relay");
    assert!(relay.probe(Duration::from_secs(2)));
    let (mut conn, _) = connect(relay.socket(), None);
    let block = leased_block(&mut conn);
    // Output keeps coming down after the client hangs up.
    let ticking = b"while :; do echo tick; sleep 0.01; done\n".to_vec();
    let input = Message::Input {
        req: 20,
        block,
        data: ticking,
    };
    conn.send(&input).expect("input");
    // Unanswered: output proves the shell runs it.
    while !matches!(conn.recv().expect("output"), Message::Output { .. }) {}
    lossy.lose.store(2, Ordering::Release);
    conn.send(&Message::Detach { req: 21 }).expect("detach");
    conn.get_ref()
        .shutdown(std::net::Shutdown::Both)
        .expect("hang up");
    drop(relay);
    assert_eq!(lossy.lose.load(Ordering::Acquire), 0, "nothing was lost");
    // A client kept for a takeover keeps its lease unless it detached.
    let (mut other, _) = connect(&server, None);
    request(&mut other, 1, |req| Message::Attach { req, block }).expect("attach");
    let leased = request(&mut other, 2, |req| Message::AcquireLease { req, block });
    assert!(leased.is_ok(), "the detach was lost: {leased:?}");
}

#[test]
fn a_candidate_whose_answers_are_lost_does_not_spoil_the_next() {
    let (server, udp) = udp_server();
    let (mut over_ssh, me) = connect(&server, None);
    let (_, id, key) = open_datagram(&mut over_ssh);
    // The first reaches the server, which hears its probes; no answer returns.
    let (deaf, heard) = (path(udp, false), path(udp, true));
    let started = std::time::Instant::now();
    let relay = reach(&[deaf.addr, heard.addr], id, &key, Duration::from_secs(2)).expect("reach");
    assert!(started.elapsed() < Duration::from_secs(3));
    let (mut moved, again) = connect(relay.socket(), Some(me));
    assert_eq!(again.client, me.client);
    sync(&mut moved, 2);
    // Under the one key, every sequence number went out once.
    let mut sent: Vec<u64> = deaf.seqs.lock().expect("seqs").clone();
    sent.extend(heard.seqs.lock().expect("seqs").iter());
    let all = sent.len();
    sent.sort_unstable();
    sent.dedup();
    assert!(all > 1);
    assert_eq!(sent.len(), all, "a sequence number was sent twice");
}
