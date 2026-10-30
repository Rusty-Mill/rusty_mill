//! An association: one key and id, any number of streams, one peer.

use std::collections::{BTreeMap, BTreeSet, VecDeque};
use std::net::SocketAddr;
use std::time::{Duration, Instant};

use rusty_aead::KEY_LEN;
use rusty_wire::Writer;

use crate::chunk::{Chunk, DATA_HEADER};
use crate::pace::{Rtt, Window};
use crate::seal::{ID_LEN, OVERHEAD, Sealer, Side};
use crate::stream::{Recv, Send, WINDOW};

/// The largest datagram sent: fits the IPv6 minimum MTU with its headers.
pub const MAX_DATAGRAM: usize = 1200;
/// The most streams open at once.
pub const MAX_STREAMS: usize = 64;
/// How long an ack may wait for data to ride on.
const ACK_DELAY: Duration = Duration::from_millis(20);
/// How long a side stays quiet before it sends a probe.
pub const KEEPALIVE: Duration = Duration::from_secs(3);
/// How many times a stream's close is sent before giving up on a reply.
const CLOSE_TRIES: u32 = 8;
/// How many times an association's end is sent.
const END_TRIES: u32 = 3;

/// Plaintext room for chunks after the stamp and echo, at most 31 bytes.
const PLAIN: usize = MAX_DATAGRAM - OVERHEAD - 31;

/// A stream's number. The client numbers them from 0, in order.
pub type StreamId = u64;

#[derive(Debug, Default)]
struct Stream {
    send: Send,
    recv: Recv,
    /// An ack for this stream should go out.
    ack: bool,
    /// The limit the peer was last told.
    told: u64,
}

/// Retries of a close the peer has not confirmed.
#[derive(Debug)]
struct Closing {
    due: Instant,
    tries: u32,
}

/// One end of an association, as a pure state machine: feed it datagrams
/// and the time, and it says what to send and when to wake.
pub struct Association {
    sealer: Sealer,
    side: Side,
    start: Instant,
    peer: Option<SocketAddr>,
    newest: Option<u64>,
    streams: BTreeMap<StreamId, Stream>,
    /// The next stream to open (client), or one past the highest seen
    /// (server).
    next_stream: StreamId,
    /// Numbers below `next_stream` not seen yet: their first bytes may
    /// still be on the way (server).
    gaps: BTreeSet<StreamId>,
    accepted: VecDeque<StreamId>,
    peer_closed: VecDeque<StreamId>,
    closing: BTreeMap<StreamId, Closing>,
    replies: Vec<StreamId>,
    rtt: Rtt,
    window: Window,
    backoff: u32,
    rto_at: Option<Instant>,
    ack_at: Option<Instant>,
    unacked: u32,
    probe: bool,
    echo: Option<(u64, Instant)>,
    last_sent: Instant,
    last_heard: Option<Instant>,
    ended: bool,
    ends_left: u32,
    /// Where the round-robin over streams picks up.
    turn: StreamId,
}

impl Association {
    /// An association for `side`, under `id` and `key`. A client knows its
    /// `peer`; a server learns it from the first authenticated datagram.
    pub fn new(
        side: Side,
        id: [u8; ID_LEN],
        key: &[u8; KEY_LEN],
        now: Instant,
        peer: Option<SocketAddr>,
    ) -> Self {
        Self {
            sealer: Sealer::new(side, id, key),
            side,
            start: now,
            peer,
            newest: None,
            streams: BTreeMap::new(),
            next_stream: 0,
            gaps: BTreeSet::new(),
            accepted: VecDeque::new(),
            peer_closed: VecDeque::new(),
            closing: BTreeMap::new(),
            replies: Vec::new(),
            rtt: Rtt::default(),
            window: Window::default(),
            backoff: 0,
            rto_at: None,
            ack_at: None,
            unacked: 0,
            probe: false,
            echo: None,
            last_sent: now,
            last_heard: None,
            ended: false,
            ends_left: 0,
            turn: 0,
        }
    }

    /// Where datagrams go: the source of the newest authenticated one, on
    /// a server.
    pub fn peer(&self) -> Option<SocketAddr> {
        self.peer
    }

    /// When the last authenticated datagram arrived.
    pub fn last_heard(&self) -> Option<Instant> {
        self.last_heard
    }

    /// The smoothed round trip, once measured.
    pub fn rtt(&self) -> Option<Duration> {
        self.rtt.smoothed()
    }

    /// Whether either side ended the association.
    pub fn is_ended(&self) -> bool {
        self.ended
    }

    /// Takes in a datagram from `from`. Returns whether it was authentic;
    /// one that is not changes nothing.
    pub fn datagram(&mut self, now: Instant, from: SocketAddr, datagram: &[u8]) -> bool {
        let Some((seq, plain)) = self.sealer.open(datagram) else {
            return false;
        };
        let Some(chunks) = Chunk::decode_all(&plain) else {
            return false;
        };
        if self.side == Side::Server && self.newest.is_none_or(|top| seq > top) {
            self.newest = Some(seq);
            self.peer = Some(from);
        }
        self.last_heard = Some(now);
        for chunk in chunks {
            self.chunk(now, chunk);
        }
        true
    }

    fn chunk(&mut self, now: Instant, chunk: Chunk) {
        if chunk.eliciting() {
            self.unacked += 1;
            let due = if self.unacked >= 2 {
                now
            } else {
                now + ACK_DELAY
            };
            self.ack_at = Some(self.ack_at.map_or(due, |at| at.min(due)));
        }
        match chunk {
            Chunk::Data {
                stream,
                offset,
                bytes,
            } => self.data(stream, offset, &bytes),
            Chunk::Ack {
                stream,
                cumulative,
                limit,
                ranges,
            } => self.ack(now, stream, cumulative, limit, &ranges),
            Chunk::Stamp(stamp) => self.echo = Some((stamp, now)),
            Chunk::Echo { stamp, held } => self.sample(now, stamp, held),
            Chunk::Probe => self.streams.values_mut().for_each(|s| s.ack = true),
            Chunk::Close { stream, reply } => self.close_chunk(stream, reply),
            Chunk::End => self.ended = true,
        }
    }

    fn data(&mut self, id: StreamId, offset: u64, bytes: &[u8]) {
        if !self.streams.contains_key(&id) && !self.accept_new(id) {
            return;
        }
        if let Some(stream) = self.streams.get_mut(&id) {
            stream.recv.data(offset, bytes);
            stream.ack = true;
        }
    }

    /// A server takes a stream it has not seen if there is room: a number
    /// past the highest, at most [`MAX_STREAMS`] past it, or one skipped
    /// on the way there. A closed stream's number never comes back.
    fn accept_new(&mut self, id: StreamId) -> bool {
        if self.side != Side::Server || self.streams.len() >= MAX_STREAMS {
            return false;
        }
        if id >= self.next_stream {
            if id - self.next_stream >= MAX_STREAMS as u64 {
                return false;
            }
            self.gaps.extend(self.next_stream..id);
            self.next_stream = id + 1;
        } else if !self.gaps.remove(&id) {
            return false;
        }
        self.streams.insert(id, Stream::default());
        self.accepted.push_back(id);
        true
    }

    fn ack(
        &mut self,
        now: Instant,
        id: StreamId,
        cumulative: u64,
        limit: u64,
        ranges: &[(u64, u64)],
    ) {
        let Some(stream) = self.streams.get_mut(&id) else {
            return;
        };
        let acked = stream.send.ack(cumulative, limit, &ranges.to_vec());
        if acked.lost > 0 {
            self.window.lost();
        }
        if acked.delivered > 0 {
            self.window.delivered(acked.delivered);
            self.backoff = 0;
            // Bytes in flight, or a window to probe once it closes.
            let waiting = self.in_flight() > 0 || self.streams.values().any(|s| s.send.blocked());
            self.rto_at = waiting.then(|| now + self.rtt.rto(0));
        }
    }

    fn sample(&mut self, now: Instant, stamp: u64, held: u64) {
        let elapsed = self.millis(now);
        if let Some(rtt) = elapsed.checked_sub(stamp).and_then(|d| d.checked_sub(held)) {
            self.rtt.sample(Duration::from_millis(rtt));
        }
    }

    fn close_chunk(&mut self, id: StreamId, reply: bool) {
        if reply {
            self.closing.remove(&id);
            return;
        }
        if self.streams.remove(&id).is_some() {
            self.peer_closed.push_back(id);
        }
        // A stream closed before its first bytes arrived never opens.
        self.gaps.remove(&id);
        self.replies.push(id);
    }

    /// Opens a stream. Only a client opens them, at most [`MAX_STREAMS`].
    pub fn open(&mut self) -> Option<StreamId> {
        if self.side != Side::Client || self.streams.len() >= MAX_STREAMS {
            return None;
        }
        let id = self.next_stream;
        self.next_stream += 1;
        self.streams.insert(id, Stream::default());
        Some(id)
    }

    /// A stream the client opened that this server has not handed out yet.
    pub fn accept(&mut self) -> Option<StreamId> {
        self.accepted.pop_front()
    }

    /// A stream the peer closed, not yet reported.
    pub fn closed(&mut self) -> Option<StreamId> {
        self.peer_closed.pop_front()
    }

    /// Queues as much of `bytes` as fits; returns how much. Nothing fits in
    /// a stream that is not open.
    pub fn write(&mut self, id: StreamId, bytes: &[u8]) -> usize {
        self.streams.get_mut(&id).map_or(0, |s| s.send.write(bytes))
    }

    /// Bytes written to `id` and not yet acknowledged.
    pub fn queued(&self, id: StreamId) -> usize {
        self.streams.get(&id).map_or(0, |s| s.send.queued())
    }

    /// Reads bytes of `id` in order into `buf`; returns how many.
    pub fn read(&mut self, id: StreamId, buf: &mut [u8]) -> usize {
        let Some(stream) = self.streams.get_mut(&id) else {
            return 0;
        };
        let n = stream.recv.read(buf);
        // Reopen a window the peer may be waiting on: an ack due at once
        // (any time already past will do).
        if stream.recv.limit() - stream.told >= WINDOW / 2 {
            stream.ack = true;
            self.ack_at = Some(self.start);
        }
        n
    }

    /// Bytes of `id` ready to read.
    pub fn readable(&self, id: StreamId) -> usize {
        self.streams.get(&id).map_or(0, |s| s.recv.readable())
    }

    /// Whether `id` is open.
    pub fn is_open(&self, id: StreamId) -> bool {
        self.streams.contains_key(&id)
    }

    /// Closes `id`, dropping what it still had to send, and tells the peer
    /// until it confirms.
    pub fn close(&mut self, now: Instant, id: StreamId) {
        if self.streams.remove(&id).is_some() {
            self.closing.insert(id, Closing { due: now, tries: 0 });
        }
    }

    /// Asks the peer for an answer now: a client's first probe, or a check
    /// that the path is alive.
    pub fn probe(&mut self) {
        self.probe = true;
    }

    /// Ends the whole association; the end is sent a few times.
    pub fn end(&mut self) {
        self.ended = true;
        self.ends_left = END_TRIES;
    }

    fn millis(&self, now: Instant) -> u64 {
        now.saturating_duration_since(self.start).as_millis() as u64
    }

    fn in_flight(&self) -> u64 {
        self.streams.values().map(|s| s.send.in_flight()).sum()
    }

    /// When [`Self::transmit`] should next be called if nothing arrives.
    pub fn timeout(&self) -> Option<Instant> {
        if self.ended {
            return None;
        }
        let keepalive = self.peer.map(|_| self.last_sent + KEEPALIVE);
        let close = self.closing.values().map(|c| c.due).min();
        [self.ack_at, self.rto_at, keepalive, close]
            .into_iter()
            .flatten()
            .min()
    }

    /// The next datagram to send and where, or `None` when there is
    /// nothing to send now. Call it until it returns `None` after anything
    /// changes and at [`Self::timeout`].
    pub fn transmit(&mut self, now: Instant) -> Option<(SocketAddr, Vec<u8>)> {
        let peer = self.peer?;
        if self.ended {
            return self.send_end(peer);
        }
        self.fire_timers(now);
        let mut w = Writer::with_capacity(PLAIN);
        let mut chunks = 0;
        let mut eliciting = false;
        chunks += self.put_acks(now, &mut w);
        let (closes, closes_elicit) = self.put_closes(now, &mut w);
        chunks += closes;
        eliciting |= closes_elicit;
        if self.probe {
            self.probe = false;
            Chunk::Probe.encode(&mut w);
            chunks += 1;
            eliciting = true;
        }
        let data = self.put_data(&mut w);
        chunks += data;
        eliciting |= data > 0;
        if chunks == 0 {
            return None;
        }
        if eliciting && self.rto_at.is_none() {
            self.rto_at = Some(now + self.rtt.rto(self.backoff));
        }
        self.last_sent = now;
        let mut plain = Writer::with_capacity(PLAIN);
        Chunk::Stamp(self.millis(now)).encode(&mut plain);
        if let Some((stamp, heard)) = self.echo.take() {
            let held = now.saturating_duration_since(heard).as_millis() as u64;
            Chunk::Echo { stamp, held }.encode(&mut plain);
        }
        plain.write_bytes(w.as_slice());
        let datagram = self.sealer.seal(plain.as_slice())?;
        Some((peer, datagram))
    }

    fn send_end(&mut self, peer: SocketAddr) -> Option<(SocketAddr, Vec<u8>)> {
        if self.ends_left == 0 {
            return None;
        }
        self.ends_left -= 1;
        let mut w = Writer::new();
        Chunk::End.encode(&mut w);
        Some((peer, self.sealer.seal(w.as_slice())?))
    }

    fn fire_timers(&mut self, now: Instant) {
        if self.rto_at.is_some_and(|at| at <= now) {
            self.rto_at = None;
            let lost: u64 = self.streams.values_mut().map(|s| s.send.timeout()).sum();
            let blocked = self.streams.values().any(|s| s.send.blocked());
            if lost > 0 {
                self.window.timed_out();
            }
            if lost > 0 || blocked {
                self.backoff += 1;
            }
            // A closed window is probed, so the peer's next ack reopens it.
            self.probe |= blocked && self.in_flight() == 0;
        }
        if now >= self.last_sent + KEEPALIVE {
            self.probe = true;
        }
    }

    fn put_acks(&mut self, now: Instant, w: &mut Writer) -> usize {
        if self.ack_at.is_none_or(|at| at > now) {
            return 0;
        }
        let mut put = 0;
        let mut left = false;
        for (&id, stream) in self.streams.iter_mut().filter(|(_, s)| s.ack) {
            let ack = Chunk::Ack {
                stream: id,
                cumulative: stream.recv.cumulative(),
                limit: stream.recv.limit(),
                ranges: stream.recv.ranges(),
            };
            let mut one = Writer::new();
            ack.encode(&mut one);
            if w.len() + one.len() > PLAIN - DATA_HEADER {
                left = true;
                continue;
            }
            w.write_bytes(one.as_slice());
            stream.ack = false;
            stream.told = stream.recv.limit();
            put += 1;
        }
        self.unacked = 0;
        self.ack_at = left.then_some(now);
        put
    }

    fn put_closes(&mut self, now: Instant, w: &mut Writer) -> (usize, bool) {
        let mut put = 0;
        for id in self.replies.drain(..) {
            Chunk::Close {
                stream: id,
                reply: true,
            }
            .encode(w);
            put += 1;
        }
        let rto = self.rtt.rto(0);
        let mut eliciting = false;
        self.closing.retain(|&id, closing| {
            if closing.due > now {
                return true;
            }
            Chunk::Close {
                stream: id,
                reply: false,
            }
            .encode(w);
            put += 1;
            eliciting = true;
            closing.tries += 1;
            closing.due = now + rto;
            closing.tries < CLOSE_TRIES
        });
        (put, eliciting)
    }

    /// Fills the rest of the datagram with stream bytes, lost ones first,
    /// taking streams in turn and keeping within the congestion window.
    fn put_data(&mut self, w: &mut Writer) -> usize {
        let mut put = 0;
        let ids: Vec<StreamId> = self
            .streams
            .range(self.turn..)
            .chain(self.streams.range(..self.turn))
            .map(|(&id, _)| id)
            .collect();
        for id in ids {
            let room = PLAIN.saturating_sub(w.len() + DATA_HEADER);
            let flight = self.in_flight();
            let budget = self.window.bytes().saturating_sub(flight) as usize;
            let max = room.min(budget);
            if max == 0 {
                break;
            }
            let Some(stream) = self.streams.get_mut(&id) else {
                continue;
            };
            let Some((offset, bytes)) = stream.send.next(max) else {
                continue;
            };
            Chunk::Data {
                stream: id,
                offset,
                bytes,
            }
            .encode(w);
            put += 1;
            self.turn = id + 1;
        }
        put
    }
}
