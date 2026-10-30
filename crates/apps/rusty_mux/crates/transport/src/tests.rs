//! Two associations over a simulated link: loss, duplication, reordering
//! and delay, on a virtual clock.

use std::collections::BTreeMap;
use std::net::SocketAddr;
use std::time::{Duration, Instant};

use rusty_wire::Writer;

use crate::chunk::Chunk;
use crate::seal::Sealer;
use crate::{Association, KEEPALIVE, MAX_DATAGRAM, MAX_STREAMS, Side, StreamId, WINDOW};

const KEY: [u8; 32] = [3; 32];
const ID: [u8; 8] = [9; 8];
const MS: Duration = Duration::from_millis(1);

fn addr(port: u16) -> SocketAddr {
    SocketAddr::from(([127, 0, 0, 2], port))
}

const SERVER: u16 = 60001;

/// A seeded xorshift, so a failing run repeats.
struct Rng(u64);

impl Rng {
    fn next(&mut self) -> u64 {
        self.0 ^= self.0 << 13;
        self.0 ^= self.0 >> 7;
        self.0 ^= self.0 << 17;
        self.0
    }

    /// True `percent` times in a hundred.
    fn chance(&mut self, percent: u64) -> bool {
        self.next() % 100 < percent
    }
}

/// How the link treats each datagram.
#[derive(Clone, Copy)]
struct Path {
    delay: Duration,
    jitter_ms: u64,
    loss: u64,
    dup: u64,
}

const CLEAN: Path = Path {
    delay: Duration::from_millis(30),
    jitter_ms: 0,
    loss: 0,
    dup: 0,
};

struct Net {
    now: Instant,
    rng: Rng,
    path: Path,
    client: Association,
    server: Association,
    /// The client's source address.
    from: SocketAddr,
    /// Datagrams in flight: when they land, toward the server or not, the
    /// source and the bytes.
    wire: BTreeMap<(Instant, u64), (bool, SocketAddr, Vec<u8>)>,
    serial: u64,
    sent: [u64; 2],
}

impl Net {
    fn new(path: Path, seed: u64) -> Self {
        let now = Instant::now();
        Self {
            now,
            rng: Rng(seed),
            path,
            client: Association::new(Side::Client, ID, &KEY, now, Some(addr(SERVER))),
            server: Association::new(Side::Server, ID, &KEY, now, None),
            from: addr(5000),
            wire: BTreeMap::new(),
            serial: 0,
            sent: [0; 2],
        }
    }

    fn put(&mut self, up: bool, bytes: Vec<u8>) {
        assert!(bytes.len() <= MAX_DATAGRAM, "{} bytes", bytes.len());
        self.sent[usize::from(up)] += 1;
        let copies = if self.rng.chance(self.path.dup) { 2 } else { 1 };
        for _ in 0..copies {
            if self.rng.chance(self.path.loss) {
                continue;
            }
            let jitter = MS * (self.rng.next() % (self.path.jitter_ms + 1)) as u32;
            let at = self.now + self.path.delay + jitter;
            self.serial += 1;
            self.wire
                .insert((at, self.serial), (up, self.from, bytes.clone()));
        }
    }

    /// Sends all each side has to send now.
    fn flush(&mut self) {
        while let Some((to, bytes)) = self.client.transmit(self.now) {
            assert_eq!(to, addr(SERVER));
            self.put(true, bytes);
        }
        while let Some((to, bytes)) = self.server.transmit(self.now) {
            assert_eq!(to, self.from, "the server answers the newest source");
            self.put(false, bytes);
        }
    }

    /// Advances to the next landing or timer, delivers what is due, and
    /// sends what that causes.
    fn step(&mut self) {
        self.flush();
        let landing = self.wire.keys().next().map(|k| k.0);
        let next = [landing, self.client.timeout(), self.server.timeout()]
            .into_iter()
            .flatten()
            .min();
        if let Some(next) = next {
            self.now = self.now.max(next);
        }
        while let Some(entry) = self.wire.first_entry() {
            if entry.key().0 > self.now {
                break;
            }
            let (up, from, bytes) = entry.remove();
            if up {
                self.server.datagram(self.now, from, &bytes);
            } else {
                self.client.datagram(self.now, addr(SERVER), &bytes);
            }
        }
        self.flush();
    }
}

fn pattern(stream: StreamId, len: usize) -> Vec<u8> {
    (0..len)
        .map(|i| (i as u64 * 31 + stream * 7) as u8)
        .collect()
}

/// Pushes `want` through, both ways on every stream, reading as it comes.
fn exchange(net: &mut Net, streams: usize, len: usize) {
    let ids: Vec<StreamId> = (0..streams).map(|_| net.client.open().unwrap()).collect();
    let mut up_sent = vec![0; streams];
    let mut down_sent = vec![0; streams];
    let mut up_got: Vec<Vec<u8>> = vec![Vec::new(); streams];
    let mut down_got: Vec<Vec<u8>> = vec![Vec::new(); streams];
    let mut buf = vec![0; 64 << 10];
    let data: Vec<Vec<u8>> = ids.iter().map(|&id| pattern(id, len)).collect();
    for _ in 0..200_000 {
        for (i, &id) in ids.iter().enumerate() {
            let data = &data[i];
            up_sent[i] += net.client.write(id, &data[up_sent[i]..]);
            if net.server.is_open(id) {
                down_sent[i] += net.server.write(id, &data[down_sent[i]..]);
            }
            let n = net.server.read(id, &mut buf);
            up_got[i].extend_from_slice(&buf[..n]);
            let n = net.client.read(id, &mut buf);
            down_got[i].extend_from_slice(&buf[..n]);
        }
        while net.server.accept().is_some() {}
        let done = (0..streams).all(|i| up_got[i].len() == len && down_got[i].len() == len);
        if done {
            for i in 0..streams {
                assert_eq!(up_got[i], data[i]);
                assert_eq!(down_got[i], data[i]);
            }
            return;
        }
        net.step();
    }
    panic!(
        "not delivered: up {:?}, down {:?}",
        up_got.iter().map(Vec::len).collect::<Vec<_>>(),
        down_got.iter().map(Vec::len).collect::<Vec<_>>()
    );
}

#[test]
fn streams_arrive_whole_and_in_order_over_a_clean_path() {
    let mut net = Net::new(CLEAN, 1);
    exchange(&mut net, 2, 300_000);
    assert!(
        net.client.rtt().unwrap() >= 55 * MS,
        "about two 30 ms trips"
    );
}

#[test]
fn streams_arrive_whole_and_in_order_through_loss_duplication_and_reordering() {
    for seed in [7, 8, 9] {
        let path = Path {
            delay: 40 * MS,
            jitter_ms: 30,
            loss: 10,
            dup: 5,
        };
        let mut net = Net::new(path, seed);
        exchange(&mut net, 3, 200_000);
    }
}

#[test]
fn a_heavy_loss_path_still_delivers() {
    let path = Path {
        delay: 80 * MS,
        jitter_ms: 50,
        loss: 35,
        dup: 0,
    };
    exchange(&mut Net::new(path, 42), 1, 50_000);
}

#[test]
fn the_server_answers_the_newest_source_and_a_late_one_does_not_win() {
    let mut net = Net::new(CLEAN, 3);
    exchange(&mut net, 1, 10_000);
    // The client roams; its next datagram moves the server.
    let old = net.from;
    net.from = addr(5001);
    exchange(&mut net, 1, 10_000);
    assert_eq!(net.server.peer(), Some(addr(5001)));
    // Two datagrams cross: the newer from the new address lands first,
    // the older, authentic and unseen, lands later from the old one. The
    // server stays with the newer.
    net.client.probe();
    let (_, older) = net.client.transmit(net.now).unwrap();
    net.client.probe();
    let (_, newer) = net.client.transmit(net.now).unwrap();
    assert!(net.server.datagram(net.now, addr(5002), &newer));
    assert!(net.server.datagram(net.now, old, &older));
    assert_eq!(net.server.peer(), Some(addr(5002)));
}

#[test]
fn a_datagram_reflected_back_to_the_server_changes_nothing() {
    let mut net = Net::new(CLEAN, 4);
    exchange(&mut net, 1, 1000);
    net.server.probe();
    let (_, own) = net.server.transmit(net.now).unwrap();
    let peer = net.server.peer();
    assert!(!net.server.datagram(net.now, addr(6666), &own));
    assert_eq!(net.server.peer(), peer);
}

#[test]
fn a_probe_is_answered_before_any_stream_opens() {
    let mut net = Net::new(CLEAN, 5);
    let start = net.now;
    net.client.probe();
    while net.client.last_heard().is_none() {
        net.step();
    }
    // A round trip and the ack delay, not the peer's own keepalive.
    let heard = net.client.last_heard().unwrap() - start;
    assert!(heard < KEEPALIVE / 10, "answered after {heard:?}");
    assert_eq!(net.server.accept(), None);
    // The answer asks for nothing back: the link goes quiet.
    let sent = net.sent;
    for _ in 0..5 {
        net.step();
    }
    assert!(net.now - start >= KEEPALIVE || net.sent == sent);
}

#[test]
fn a_stalled_reader_closes_the_window_and_loses_nothing() {
    let mut net = Net::new(CLEAN, 6);
    let id = net.client.open().unwrap();
    let total = 3 * WINDOW as usize;
    let data = pattern(id, total);
    let mut written = net.client.write(id, &data);
    assert_eq!(
        written, WINDOW as usize,
        "the sender's own buffer is bounded"
    );
    for _ in 0..2000 {
        net.step();
        written += net.client.write(id, &data[written..]);
    }
    // The server reads nothing: it holds one window, the client the rest.
    assert_eq!(net.server.accept(), Some(id));
    assert_eq!(net.server.readable(id), WINDOW as usize);
    assert!(net.client.queued(id) <= WINDOW as usize);
    assert!(written < total);
    let mut got = Vec::new();
    let mut buf = vec![0; 64 << 10];
    for _ in 0..200_000 {
        let n = net.server.read(id, &mut buf);
        got.extend_from_slice(&buf[..n]);
        written += net.client.write(id, &data[written..]);
        if got.len() == total {
            assert_eq!(got, data);
            return;
        }
        net.step();
    }
    panic!("got {} of {total}", got.len());
}

#[test]
fn a_closed_stream_is_reported_once_and_its_number_never_reopens() {
    let mut net = Net::new(CLEAN, 7);
    exchange(&mut net, 2, 1000);
    net.client.close(net.now, 0);
    for _ in 0..20 {
        net.step();
    }
    assert_eq!(net.server.closed(), Some(0));
    assert_eq!(net.server.closed(), None);
    assert!(!net.server.is_open(0) && net.server.is_open(1));
    // Confirmed: the client stops resending its close.
    assert!(net.client.timeout().unwrap() >= net.now + 10 * MS);
}

#[test]
fn an_end_reaches_the_peer() {
    let mut net = Net::new(CLEAN, 8);
    exchange(&mut net, 1, 100);
    net.client.end();
    for _ in 0..10 {
        net.step();
    }
    assert!(net.server.is_ended());
}

#[test]
fn an_idle_path_is_kept_alive() {
    let mut net = Net::new(CLEAN, 9);
    exchange(&mut net, 1, 100);
    let before = net.sent;
    let heard = net.client.last_heard().unwrap();
    let until = net.now + KEEPALIVE * 3;
    while net.now < until {
        net.step();
    }
    assert!(net.sent[1] > before[1] && net.sent[0] > before[0]);
    assert!(net.client.last_heard().unwrap() > heard);
}

#[test]
fn streams_whose_first_bytes_arrive_out_of_order_are_all_accepted() {
    let mut net = Net::new(CLEAN, 10);
    let ids: Vec<StreamId> = (0..4).map(|_| net.client.open().unwrap()).collect();
    for &id in ids.iter().rev() {
        net.client.write(id, b"x");
        net.step();
    }
    net.client.close(net.now, ids[0]);
    for _ in 0..20 {
        net.step();
    }
    let accepted: Vec<StreamId> = std::iter::from_fn(|| net.server.accept()).collect();
    assert_eq!(accepted, vec![3, 2, 1, 0]);
    assert!(!net.server.is_open(0));
    // A late copy of stream 0's first bytes does not reopen it.
    let mut again = Association::new(Side::Client, ID, &KEY, net.now, Some(addr(SERVER)));
    assert_eq!(again.open(), Some(0));
    again.write(0, b"x");
    let (_, datagram) = again.transmit(net.now).unwrap();
    net.server.datagram(net.now, net.from, &datagram);
    assert!(!net.server.is_open(0));
}

/// A client datagram carrying exactly `chunks`, for orders the client
/// itself would never send.
fn forged(sealer: &mut Sealer, chunks: &[Chunk]) -> Vec<u8> {
    let mut w = Writer::new();
    chunks.iter().for_each(|c| c.encode(&mut w));
    sealer.seal(w.as_slice()).unwrap()
}

fn data(stream: StreamId) -> Chunk {
    Chunk::Data {
        stream,
        offset: 0,
        bytes: b"x".to_vec(),
    }
}

fn close(stream: StreamId) -> Chunk {
    Chunk::Close {
        stream,
        reply: false,
    }
}

#[test]
fn a_stream_closed_before_its_first_bytes_never_opens() {
    let now = Instant::now();
    let mut server = Association::new(Side::Server, ID, &KEY, now, None);
    let mut client = Sealer::new(Side::Client, ID, &KEY);
    let (late, early) = (
        forged(&mut client, &[data(0)]),
        forged(&mut client, &[close(0)]),
    );
    assert!(server.datagram(now, addr(1), &early));
    assert!(server.datagram(now, addr(1), &late));
    assert_eq!(server.accept(), None);
    assert!(!server.is_open(0));
}

#[test]
fn skipped_stream_numbers_stay_bounded() {
    let now = Instant::now();
    let mut server = Association::new(Side::Server, ID, &KEY, now, None);
    let mut client = Sealer::new(Side::Client, ID, &KEY);
    let skip = MAX_STREAMS as u64 - 1;
    let first = forged(&mut client, &[data(skip), close(skip)]);
    assert!(server.datagram(now, addr(1), &first));
    assert_eq!(server.accept(), Some(skip));
    assert_eq!(server.closed(), Some(skip));
    // Skipping as far again would hold twice as many gaps: refused.
    let again = forged(&mut client, &[data(2 * skip + 1)]);
    assert!(server.datagram(now, addr(1), &again));
    assert_eq!(server.accept(), None);
    // A number already skipped still opens.
    let filled = forged(&mut client, &[data(3)]);
    assert!(server.datagram(now, addr(1), &filled));
    assert_eq!(server.accept(), Some(3));
}

#[test]
fn a_close_lost_to_an_outage_is_told_again_when_the_peer_speaks() {
    let mut net = Net::new(CLEAN, 7);
    let id = net.client.open().unwrap();
    net.client.write(id, b"hello");
    while net.server.accept().is_none() {
        net.step();
    }
    net.path.loss = 100;
    net.server.close(net.now, id);
    let start = net.now;
    // Long enough for every try of the close to be lost.
    while net.now - start < 10 * KEEPALIVE {
        net.step();
    }
    assert!(net.client.is_open(id));
    net.path.loss = 0;
    net.client.write(id, b" again");
    let start = net.now;
    while net.client.is_open(id) {
        assert!(
            net.now - start < 2 * KEEPALIVE,
            "the close was not told again"
        );
        net.step();
    }
    assert_eq!(net.client.closed(), Some(id));
}

#[test]
fn closes_that_do_not_fit_one_datagram_wait_for_the_next() {
    let now = Instant::now();
    let mut client = Association::new(Side::Client, ID, &KEY, now, Some(addr(SERVER)));
    for _ in 0..1000 {
        let id = client.open().unwrap();
        client.close(now, id);
    }
    let mut sizes = Vec::new();
    while let Some((_, datagram)) = client.transmit(now) {
        sizes.push(datagram.len());
    }
    assert!(sizes.len() > 1, "{sizes:?}");
    assert!(sizes.iter().all(|&n| n <= MAX_DATAGRAM), "{sizes:?}");
}

#[test]
fn streams_closed_before_any_bytes_leave_later_numbers_free() {
    let now = Instant::now();
    let mut server = Association::new(Side::Server, ID, &KEY, now, None);
    let mut client = Sealer::new(Side::Client, ID, &KEY);
    // Many more empty open/close cycles than streams fit at once.
    for id in 0..4 * MAX_STREAMS as u64 {
        assert!(server.datagram(now, addr(1), &forged(&mut client, &[close(id)])));
    }
    let next = 4 * MAX_STREAMS as u64;
    let opened = forged(&mut client, &[data(next + 1), data(next)]);
    assert!(server.datagram(now, addr(1), &opened));
    assert_eq!(server.accept(), Some(next + 1));
    assert_eq!(server.accept(), Some(next));
    // None of the closed ones comes back.
    let late = forged(&mut client, &[data(0), data(next - 1)]);
    assert!(server.datagram(now, addr(1), &late));
    assert_eq!(server.accept(), None);
}

#[test]
fn acks_closes_and_data_together_stay_within_a_datagram() {
    let mut net = Net::new(CLEAN, 8);
    let ids: Vec<StreamId> = (0..MAX_STREAMS)
        .map(|_| net.client.open().unwrap())
        .collect();
    for &id in &ids {
        net.client.write(id, b"x");
    }
    let mut accepted = 0;
    while accepted < ids.len() {
        net.step();
        while net.server.accept().is_some() {
            accepted += 1;
        }
    }
    // Every stream owes an ack, half close and the rest have data, all at
    // once; `Net::put` checks each datagram's size.
    let big = vec![7; 4000];
    for &id in &ids {
        net.client.write(id, b"y");
        net.server.write(id, &big);
    }
    net.step();
    for &id in &ids[..MAX_STREAMS / 2] {
        net.server.close(net.now, id);
    }
    for _ in 0..200 {
        net.step();
    }
    for &id in &ids[..MAX_STREAMS / 2] {
        assert!(!net.client.is_open(id));
    }
}

#[test]
fn closes_after_acks_that_fill_a_datagram_wait_for_the_next() {
    let now = Instant::now();
    let mut client = Association::new(Side::Client, ID, &KEY, now, Some(addr(SERVER)));
    let mut server = Sealer::new(Side::Server, ID, &KEY);
    let ids: Vec<StreamId> = (0..MAX_STREAMS).map(|_| client.open().unwrap()).collect();
    let (ranged, closed) = ids.split_at(36);
    // Holes in each ranged stream: its ack carries four ranges with
    // three-byte offsets, so the acks alone nearly fill one datagram.
    for &stream in ranged {
        let chunks: Vec<Chunk> = (0..5)
            .map(|k| Chunk::Data {
                stream,
                offset: 100_000 + 2000 * k,
                bytes: b"z".to_vec(),
            })
            .collect();
        assert!(client.datagram(now, addr(SERVER), &forged(&mut server, &chunks)));
    }
    for &id in closed {
        client.close(now, id);
    }
    // And the peer's own closes, each owed a reply.
    let peer_closes: Vec<Chunk> = (1000..1000 + MAX_STREAMS as u64).map(close).collect();
    assert!(client.datagram(now, addr(SERVER), &forged(&mut server, &peer_closes)));
    let mut sizes = Vec::new();
    while let Some((_, datagram)) = client.transmit(now) {
        sizes.push(datagram.len());
    }
    assert!(sizes.len() > 1, "{sizes:?}");
    assert!(sizes.iter().all(|&n| n <= MAX_DATAGRAM), "{sizes:?}");
}

/// The close replies in what `server` sends next.
fn close_replies(server: &mut Association, client: &mut Sealer, now: Instant) -> Vec<StreamId> {
    let mut replies = Vec::new();
    while let Some((_, datagram)) = server.transmit(now) {
        let (_, plain) = client.open(&datagram).unwrap();
        for chunk in Chunk::decode_all(&plain).unwrap() {
            if let Chunk::Close {
                stream,
                reply: true,
            } = chunk
            {
                replies.push(stream);
            }
        }
    }
    replies
}

#[test]
fn a_far_ahead_close_stays_recorded_as_the_numbers_catch_up() {
    let now = Instant::now();
    let later = now + Duration::from_secs(1);
    let mut server = Association::new(Side::Server, ID, &KEY, now, None);
    let mut client = Sealer::new(Side::Client, ID, &KEY);
    let far = MAX_STREAMS as u64 + 1;
    // Too far to pass as gaps, so recorded apart, and confirmed.
    assert!(server.datagram(now, addr(1), &forged(&mut client, &[close(far)])));
    assert_eq!(close_replies(&mut server, &mut client, later), [far]);
    // The numbers catch up; the closed one never opens.
    assert!(server.datagram(now, addr(1), &forged(&mut client, &[data(0)])));
    assert_eq!(server.accept(), Some(0));
    assert!(server.datagram(now, addr(1), &forged(&mut client, &[data(far)])));
    assert_eq!(server.accept(), None);
    // Passing it on the way to a higher number leaves no gap for it.
    let next = forged(&mut client, &[data(far + 1)]);
    assert!(server.datagram(now, addr(1), &next));
    assert_eq!(server.accept(), Some(far + 1));
    assert!(server.datagram(now, addr(1), &forged(&mut client, &[data(far)])));
    assert_eq!(server.accept(), None);
}

#[test]
fn a_far_ahead_close_with_no_room_to_record_is_not_confirmed() {
    let now = Instant::now();
    let later = now + Duration::from_secs(1);
    let mut server = Association::new(Side::Server, ID, &KEY, now, None);
    let mut client = Sealer::new(Side::Client, ID, &KEY);
    let base = 2 * MAX_STREAMS as u64;
    let room: Vec<Chunk> = (base..base + MAX_STREAMS as u64).map(close).collect();
    assert!(server.datagram(now, addr(1), &forged(&mut client, &room)));
    assert_eq!(
        close_replies(&mut server, &mut client, later).len(),
        MAX_STREAMS
    );
    let over = base + MAX_STREAMS as u64;
    assert!(server.datagram(now, addr(1), &forged(&mut client, &[close(over)])));
    assert_eq!(close_replies(&mut server, &mut client, later), []);
    // Once the numbers pass them they are plain retired numbers, and
    // the room comes back.
    for id in (0..base).chain([over + 1]) {
        assert!(server.datagram(now, addr(1), &forged(&mut client, &[data(id), close(id)])));
    }
    close_replies(&mut server, &mut client, later);
    let farther = over + 2 * MAX_STREAMS as u64;
    assert!(server.datagram(now, addr(1), &forged(&mut client, &[close(farther)])));
    assert_eq!(close_replies(&mut server, &mut client, later), [farther]);
}

#[test]
fn bytes_received_before_the_peer_closes_stay_readable() {
    let mut net = Net::new(CLEAN, 9);
    let id = net.client.open().unwrap();
    net.client.write(id, b"hello");
    while net.server.accept().is_none() {
        net.step();
    }
    // The server answers, waits for the ack, then closes, as an adapter
    // ending a stream gracefully does; the client has not read yet.
    net.server.write(id, b"last words");
    while net.server.queued(id) > 0 {
        net.step();
    }
    net.server.close(net.now, id);
    while net.client.is_open(id) {
        net.step();
    }
    assert_eq!(net.client.readable(id), 10);
    let mut buf = [0; 4];
    let mut got = Vec::new();
    loop {
        let n = net.client.read(id, &mut buf);
        if n == 0 {
            break;
        }
        got.extend_from_slice(&buf[..n]);
    }
    assert_eq!(got, b"last words");
    assert_eq!(net.client.readable(id), 0);
}
