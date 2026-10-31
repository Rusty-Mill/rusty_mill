//! The UDP side against an in-process client association over loopback.

use std::collections::HashMap;
use std::future::poll_fn;

use rusty_mux_core::ClientId;
use rusty_mux_proto::{Features, Message, decode, encode};
use rusty_tokio::time::timeout;
use rusty_tokio::{Builder, LocalOptions};

use super::*;
use crate::Config;
use crate::state::State;

const PATIENCE: Duration = Duration::from_secs(20);
/// The longest a test peer waits for a datagram before looking again.
const TICK: Duration = Duration::from_millis(20);

fn runtime() -> rusty_tokio::LocalRuntime {
    Builder::new_current_thread()
        .build_local(&LocalOptions::default())
        .expect("runtime")
}

/// A state offering `DATAGRAM` and a UDP side on loopback, silent
/// associations ending after `idle`.
async fn server(idle: Duration) -> (Shared, Registry, SocketAddr) {
    let state = State::shared(
        Config {
            program: "unused".into(),
            args: Vec::new(),
            cwd: "/".into(),
        },
        None,
    );
    let socket = UdpSocket::bind("127.0.0.1:0").await.expect("bind");
    let addr = socket.local_addr().expect("addr");
    let registry = Registry::new(addr.port());
    state.borrow_mut().offer_datagrams(registry.clone());
    let udp = serve(Rc::new(socket), registry.clone(), state.clone(), idle);
    rusty_tokio::task::spawn_local(async move {
        udp.await.expect("udp side");
    });
    (state, registry, addr)
}

/// The client end of an association, pumped by hand.
struct Peer {
    socket: UdpSocket,
    assoc: Association,
    /// Bytes read from each stream and not yet decoded.
    received: HashMap<StreamId, Vec<u8>>,
}

impl Peer {
    async fn new(registry: &Registry, server: SocketAddr) -> Self {
        let (port, id, key) = registry.offer(Instant::now()).expect("offer");
        assert_eq!(port, server.port());
        let assoc = Association::new(Side::Client, id, &key, Instant::now(), Some(server));
        Self {
            socket: bind().await,
            assoc,
            received: HashMap::new(),
        }
    }

    /// Sends, receives and feeds the association until `done` holds.
    async fn until(&mut self, mut done: impl FnMut(&mut Association) -> bool) {
        let deadline = Instant::now() + PATIENCE;
        let mut buf = [0; MAX_DATAGRAM];
        loop {
            let now = Instant::now();
            while let Some((to, datagram)) = self.assoc.transmit(now) {
                self.socket.send_to(&datagram, to).await.expect("send");
            }
            if done(&mut self.assoc) {
                return;
            }
            assert!(now < deadline, "timed out");
            let wait = self
                .assoc
                .timeout()
                .map_or(TICK, |at| at.saturating_duration_since(now).min(TICK));
            if let Ok(received) = timeout(wait, self.socket.recv_from(&mut buf)).await {
                let (n, from) = received.expect("recv");
                self.assoc.datagram(Instant::now(), from, &buf[..n]);
            }
        }
    }

    /// Probes until the server answers.
    async fn probe(&mut self) {
        self.assoc.probe();
        self.until(|a| a.last_heard().is_some()).await;
    }

    async fn send(&mut self, stream: StreamId, message: &Message) {
        let frame = encode(message).expect("encode");
        assert_eq!(self.assoc.write(stream, &frame), frame.len());
        self.until(|a| a.queued(stream) == 0).await;
    }

    async fn recv(&mut self, stream: StreamId) -> Message {
        let mut decoded = None;
        let mut received = self.received.remove(&stream).unwrap_or_default();
        self.until(|a| {
            let mut chunk = [0; 4096];
            let n = a.read(stream, &mut chunk);
            received.extend_from_slice(&chunk[..n]);
            decoded = decode(&received).expect("decode");
            decoded.is_some()
        })
        .await;
        let (message, used) = decoded.expect("a message");
        received.drain(..used);
        self.received.insert(stream, received);
        message
    }

    /// The next message on `stream` that is not an event.
    async fn reply(&mut self, stream: StreamId) -> Message {
        loop {
            match self.recv(stream).await {
                Message::Event { .. } => continue,
                other => return other,
            }
        }
    }

    /// Opens a stream and says Hello on it; returns the stream and client.
    async fn connect(&mut self, features: Features) -> (StreamId, ClientId) {
        let stream = self.assoc.open().expect("a stream");
        let hello = Message::Hello {
            features,
            resume: None,
            takeover: None,
        };
        self.send(stream, &hello).await;
        match self.recv(stream).await {
            Message::Welcome { client, .. } => (stream, client),
            other => panic!("expected Welcome, got {other:?}"),
        }
    }
}

async fn bind() -> UdpSocket {
    UdpSocket::bind("127.0.0.1:0").await.expect("bind")
}

#[test]
fn a_stream_is_served_as_a_connection_and_follows_a_roam() {
    runtime().block_on(async {
        let (_state, registry, addr) = server(PATIENCE).await;
        let mut peer = Peer::new(&registry, addr).await;
        peer.probe().await;
        assert_eq!(registry.live(), 1);
        let features = Features::DATAGRAM.union(Features::TAKEOVER);
        let (stream, _) = peer.connect(features).await;

        // A new source port is a new address: the replies follow it.
        peer.socket = bind().await;
        peer.send(stream, &Message::Sync { req: 7 }).await;
        assert!(matches!(
            peer.reply(stream).await,
            Message::Done { req: 7, .. }
        ));

        // A second stream on the same association is its own client.
        let (_, other) = peer.connect(features).await;
        let (_, third) = peer.connect(features).await;
        assert_ne!(other, third);
    });
}

#[test]
fn only_an_authentic_datagram_brings_an_offer_alive() {
    runtime().block_on(async {
        let (_state, registry, addr) = server(PATIENCE).await;
        let now = Instant::now();
        let mut peer = Peer::new(&registry, addr).await;
        peer.assoc.probe();
        let (_, mut forged) = peer.assoc.transmit(now).expect("a probe");
        *forged.last_mut().expect("a tag") ^= 1;
        // An id never offered, under a key the server never made.
        let mut stranger =
            Association::new(Side::Client, [9; ID_LEN], &[9; KEY_LEN], now, Some(addr));
        stranger.probe();
        let (_, unknown) = stranger.transmit(now).expect("a probe");
        let socket = bind().await;
        // Garbage larger than any datagram must not end the receiver.
        let oversized = vec![0xee; 4000];
        for datagram in [&forged[..], &unknown[..], &[0; 4], &oversized[..]] {
            socket.send_to(datagram, addr).await.expect("send");
        }
        rusty_tokio::time::sleep(TICK * 5).await;
        assert_eq!(
            registry.live(),
            0,
            "forged, short, oversized or unknown: nothing live"
        );

        peer.probe().await;
        assert_eq!(registry.live(), 1);
    });
}

#[test]
fn silence_past_idle_ends_the_association_and_its_connections() {
    runtime().block_on(async {
        let idle = Duration::from_millis(400);
        let (state, registry, addr) = server(idle).await;
        let mut peer = Peer::new(&registry, addr).await;
        peer.probe().await;
        // Without TAKEOVER the client goes when its connection does.
        let (_, client) = peer.connect(Features::DATAGRAM).await;
        assert!(state.borrow().has_client(client));

        rusty_tokio::time::sleep(idle * 3).await;
        assert_eq!(registry.live(), 0);
        assert!(!state.borrow().has_client(client));
        // The server said so before it went.
        peer.until(|a| a.is_ended()).await;
    });
}

#[test]
fn closing_a_stream_ends_its_connection_only() {
    runtime().block_on(async {
        let (state, registry, addr) = server(PATIENCE).await;
        let mut peer = Peer::new(&registry, addr).await;
        peer.probe().await;
        let (first, gone) = peer.connect(Features::DATAGRAM).await;
        let (second, kept) = peer.connect(Features::DATAGRAM).await;
        peer.assoc.close(Instant::now(), first);
        let deadline = Instant::now() + PATIENCE;
        while state.borrow().has_client(gone) {
            assert!(
                Instant::now() < deadline,
                "the closed stream's client stayed"
            );
            peer.until(|_| true).await;
            rusty_tokio::time::sleep(TICK).await;
        }
        assert!(state.borrow().has_client(kept));
        peer.send(second, &Message::Sync { req: 1 }).await;
        assert!(matches!(
            peer.reply(second).await,
            Message::Done { req: 1, .. }
        ));
    });
}

#[test]
fn offers_not_taken_up_expire() {
    let registry = Registry::new(1);
    let start = Instant::now();
    let (_, first, _) = registry.offer(start).expect("offer");
    let (_, second, _) = registry.offer(start + OFFER_TTL / 2).expect("offer");
    assert_ne!(first, second, "fresh ids");
    registry.offer(start + OFFER_TTL).expect("offer");
    let slots = registry.slots.borrow();
    assert!(!slots.contains_key(&first));
    assert!(slots.contains_key(&second));
}

/// A live link on loopback with no connections served, for driving a
/// stream's halves by hand.
async fn bare_link(peer: &mut Peer, server: Rc<UdpSocket>, registry: Registry) -> Rc<Link> {
    peer.assoc.probe();
    peer.until(|_| true).await;
    let mut buf = [0; MAX_DATAGRAM];
    let (n, from) = server.recv_from(&mut buf).await.expect("recv");
    let Received::New(link) = registry.datagram(Instant::now(), from, &buf[..n]) else {
        panic!("the probe should bring the offer alive");
    };
    rusty_tokio::task::spawn_local(drive(link.clone(), server.clone(), registry, PATIENCE));
    let feed = link.clone();
    rusty_tokio::task::spawn_local(async move {
        let mut buf = [0; MAX_DATAGRAM];
        while let Ok((n, from)) = server.recv_from(&mut buf).await {
            feed.datagram(Instant::now(), from, &buf[..n]);
        }
    });
    link
}

#[test]
fn shutting_a_stream_down_delivers_everything_written_first() {
    runtime().block_on(async {
        let socket = Rc::new(bind().await);
        let addr = socket.local_addr().expect("addr");
        let registry = Registry::new(addr.port());
        let mut peer = Peer::new(&registry, addr).await;
        let link = bare_link(&mut peer, socket, registry).await;
        let stream = peer.assoc.open().expect("a stream");
        assert_eq!(peer.assoc.write(stream, b"hi"), 2);
        peer.until(|a| a.queued(stream) == 0).await;
        let id = link.accept().expect("the peer's stream");

        let sent: Vec<u8> = (0..300_000u32).map(|i| i as u8).collect();
        let mut wr = StreamWriter {
            link: link.clone(),
            id,
        };
        let bytes = sent.clone();
        let writer = rusty_tokio::task::spawn_local(async move {
            let mut rest = &bytes[..];
            while !rest.is_empty() {
                let n = poll_fn(|cx| Pin::new(&mut wr).poll_write(cx, rest)).await;
                rest = &rest[n.expect("write")..];
            }
            poll_fn(|cx| Pin::new(&mut wr).poll_shutdown(cx))
                .await
                .expect("shutdown");
        });

        let mut received = Vec::new();
        peer.until(|a| {
            let mut chunk = [0; 4096];
            loop {
                let n = a.read(stream, &mut chunk);
                if n == 0 {
                    break;
                }
                received.extend_from_slice(&chunk[..n]);
            }
            !a.is_open(stream) && a.readable(stream) == 0
        })
        .await;
        writer.await.expect("writer");
        assert_eq!(received.len(), sent.len());
        assert!(received == sent, "every byte, in order, before the close");
    });
}

#[test]
fn an_offer_past_its_lifetime_cannot_be_taken_up() {
    let registry = Registry::new(1);
    let start = Instant::now();
    let (_, id, key) = registry.offer(start).expect("offer");
    let mut client = Association::new(Side::Client, id, &key, start, Some(addr_of(1)));
    client.probe();
    let (_, probe) = client.transmit(start).expect("a probe");
    let late = start + OFFER_TTL;
    let received = registry.datagram(late, addr_of(2), &probe);
    assert!(matches!(received, Received::Nothing));
    assert!(!registry.slots.borrow().contains_key(&id), "forgotten");
}

#[test]
fn offers_stop_at_the_association_limit_until_one_expires() {
    let registry = Registry::new(1);
    let start = Instant::now();
    for _ in 0..MAX_ASSOCIATIONS {
        registry.offer(start).expect("offer");
    }
    assert!(registry.offer(start).is_err(), "one past the limit");
    registry
        .offer(start + OFFER_TTL)
        .expect("room once they expire");
}

fn addr_of(port: u16) -> SocketAddr {
    SocketAddr::from(([127, 0, 0, 1], port))
}
