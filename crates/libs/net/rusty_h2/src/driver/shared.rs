//! The state shared between a connection's driver task and the handles the
//! application holds. Everything is behind one mutex; I/O is never done
//! while waiting for anything but the lock.

use std::collections::{HashMap, VecDeque};
use std::ops::{Deref, DerefMut};
use std::sync::{Arc, Mutex, MutexGuard, PoisonError};
use std::task::Waker;

use super::{Body, Error};
use crate::connect::{Connection, Inbound};
use crate::error::{ErrorCode, H2Error};
use crate::frame::{Frame, GoAwayFrame, PingFrame, RstStreamFrame};
use crate::hpack::HeaderField;

/// Unwritten output above this stops the driver reading and the
/// application writing data, so a peer that never reads is not buffered
/// for without limit.
pub(super) const OUT_HIGH: usize = 512 * 1024;

/// Requests received and not yet accepted. Beyond this the server refuses
/// new streams, so an application that is slow to accept (or never does)
/// cannot be made to hold an unbounded backlog.
pub(super) const MAX_PENDING_ACCEPT: usize = 256;

pub(super) type Handle = Arc<Mutex<Shared>>;

/// The shared state, locked. Wakers collected while it was held are woken
/// only after the mutex is released: a wake can drop the last reference to
/// a task, and that task's destructors (a dropped stream handle, a dropped
/// driver) lock this mutex.
pub(super) struct Guard<'a> {
    /// `None` only inside `drop`.
    inner: Option<MutexGuard<'a, Shared>>,
}

pub(super) fn lock(h: &Handle) -> Guard<'_> {
    Guard {
        inner: Some(h.lock().unwrap_or_else(PoisonError::into_inner)),
    }
}

impl Deref for Guard<'_> {
    type Target = Shared;
    fn deref(&self) -> &Shared {
        self.inner
            .as_deref()
            .expect("the guard is live until dropped")
    }
}

impl DerefMut for Guard<'_> {
    fn deref_mut(&mut self) -> &mut Shared {
        self.inner
            .as_deref_mut()
            .expect("the guard is live until dropped")
    }
}

impl Drop for Guard<'_> {
    fn drop(&mut self) {
        let Some(mut inner) = self.inner.take() else {
            return;
        };
        let wakes = std::mem::take(&mut inner.wakes);
        let trash = std::mem::take(&mut inner.trash);
        drop(inner);
        for w in wakes {
            w.wake();
        }
        drop(trash);
    }
}

/// Why a stream ended without finishing.
#[derive(Debug, Clone, Copy)]
pub(super) enum Fail {
    Reset(ErrorCode),
    GoAway(ErrorCode),
    Closed,
}

impl Fail {
    pub(super) fn error(self) -> Error {
        match self {
            Fail::Reset(c) => Error::Reset(c),
            Fail::GoAway(c) => Error::GoAway(c),
            Fail::Closed => Error::Closed,
        }
    }
}

pub(super) enum Item {
    Body(Body),
    Fail(Fail),
}

#[derive(Default)]
pub(super) struct Stream {
    pub(super) inbox: VecDeque<Item>,
    pub(super) recv_waker: Option<Waker>,
    /// The first header block, until the application takes it.
    pub(super) headers: Option<Vec<HeaderField>>,
    pub(super) headers_waker: Option<Waker>,
    pub(super) local_closed: bool,
    pub(super) remote_closed: bool,
    /// Set when the stream ended abnormally; senders fail with it.
    pub(super) abort: Option<Fail>,
    pub(super) recv_dropped: bool,
    pub(super) headers_taken: bool,
    /// Tasks waiting in [`Closed`](super::Closed) for the stream to go away.
    pub(super) closed_wakers: Vec<Waker>,
    counted_done: bool,
}

pub(super) struct Ping {
    pub(super) acked: bool,
    pub(super) waker: Option<Waker>,
}

pub(super) struct Shared {
    pub(super) conn: Connection,
    pub(super) server: bool,
    pub(super) out: Vec<u8>,
    /// Bytes the driver has taken out of `out` and not yet written.
    pub(super) inflight: usize,
    /// Every byte ever queued, and the position just after the last frame
    /// that carries an application's data (HEADERS, DATA, RST_STREAM). Lets
    /// the driver tell whether what it failed to deliver was only protocol
    /// housekeeping (SETTINGS, acks, the closing GOAWAY).
    pub(super) queued_total: u64,
    pub(super) app_end: u64,
    pub(super) driver_waker: Option<Waker>,
    pub(super) streams: HashMap<u32, Stream>,
    /// Streams not yet closed in both directions.
    pub(super) active: usize,
    pub(super) accept_queue: VecDeque<u32>,
    pub(super) accept_waker: Option<Waker>,
    pub(super) settings_received: bool,
    /// Tasks waiting for capacity, a stream slot, settings or output room.
    pub(super) waiters: Vec<Waker>,
    pub(super) pings: HashMap<u64, Ping>,
    pub(super) next_ping: u64,
    /// No more streams are started or accepted; the connection ends once
    /// the active ones finish.
    pub(super) closing: bool,
    pub(super) goaway_sent: Option<u32>,
    pub(super) last_peer_stream: u32,
    /// The driver has ended.
    pub(super) closed: bool,
    /// Wakers to run once the lock is released (see [`Guard`]).
    pub(super) wakes: Vec<Waker>,
    /// Wakers displaced or forgotten while the lock was held; their
    /// destructors may lock it, so they are dropped after it is released.
    pub(super) trash: Vec<Waker>,
}

pub(super) fn register(list: &mut Vec<Waker>, w: &Waker) {
    if !list.iter().any(|x| x.will_wake(w)) {
        list.push(w.clone());
    }
}

impl Shared {
    pub(super) fn new(conn: Connection, server: bool, preface: bool) -> Shared {
        let mut s = Shared {
            conn,
            server,
            out: Vec::new(),
            inflight: 0,
            queued_total: 0,
            app_end: 0,
            driver_waker: None,
            streams: HashMap::new(),
            active: 0,
            accept_queue: VecDeque::new(),
            accept_waker: None,
            settings_received: false,
            waiters: Vec::new(),
            pings: HashMap::new(),
            next_ping: 1,
            closing: false,
            goaway_sent: None,
            last_peer_stream: 0,
            closed: false,
            wakes: Vec::new(),
            trash: Vec::new(),
        };
        if preface {
            s.out.extend_from_slice(crate::CONNECTION_PREFACE);
            s.queued_total += crate::CONNECTION_PREFACE.len() as u64;
        }
        let settings = s.conn.settings_frame();
        s.queue([settings]);
        s
    }

    /// Output queued or in the driver's hands and not yet written.
    pub(super) fn backlog(&self) -> usize {
        self.out.len() + self.inflight
    }

    pub(super) fn queue(&mut self, frames: impl IntoIterator<Item = Frame>) {
        for f in frames {
            let before = self.out.len();
            f.encode(&mut self.out);
            self.queued_total += (self.out.len() - before) as u64;
            if matches!(f, Frame::Headers(_) | Frame::Data(_) | Frame::RstStream(_)) {
                self.app_end = self.queued_total;
            }
        }
        self.wake_driver();
    }

    pub(super) fn wake_driver(&mut self) {
        if let Some(w) = self.driver_waker.take() {
            self.wakes.push(w);
        }
    }

    pub(super) fn wake_waiters(&mut self) {
        for w in self.waiters.drain(..) {
            self.wakes.push(w);
        }
    }

    pub(super) fn release(&mut self, id: u32, n: usize) {
        let n = u32::try_from(n).unwrap_or(u32::MAX);
        if n > 0 {
            let frames = self.conn.release_capacity(id, n);
            self.queue(frames);
        }
    }

    /// Starts tracking a stream the application will see.
    pub(super) fn add_stream(&mut self, id: u32, local_closed: bool) {
        self.active += 1;
        self.streams.insert(
            id,
            Stream {
                local_closed,
                ..Stream::default()
            },
        );
    }

    pub(super) fn close_local(&mut self, id: u32) {
        if let Some(s) = self.streams.get_mut(&id) {
            s.local_closed = true;
        }
        self.settle(id);
    }

    fn close_remote(&mut self, id: u32) {
        if let Some(s) = self.streams.get_mut(&id) {
            s.remote_closed = true;
            // A reader waiting for more must see the end, whatever the
            // final frame carried (an empty DATA ends a body too).
            if let Some(w) = s.recv_waker.take() {
                self.wakes.push(w);
            }
        }
        self.settle(id);
    }

    /// Counts the stream done once both directions are closed, and forgets
    /// it once nothing is left for the application to read.
    pub(super) fn settle(&mut self, id: u32) {
        let Some(s) = self.streams.get_mut(&id) else {
            return;
        };
        if s.local_closed && s.remote_closed && !s.counted_done {
            s.counted_done = true;
            self.active = self.active.saturating_sub(1);
            self.wake_waiters();
            self.wake_driver();
        }
        let s = &self.streams[&id];
        if s.counted_done && s.headers_taken && (s.recv_dropped || s.inbox.is_empty()) {
            self.forget_stream(id);
        }
    }

    /// Stops tracking a stream. Unread body is thrown away first, so the
    /// connection credit it held goes back to the peer exactly once.
    pub(super) fn forget_stream(&mut self, id: u32) {
        self.discard(id);
        if let Some(mut s) = self.streams.remove(&id) {
            self.trash.extend(s.recv_waker.take());
            self.trash.extend(s.headers_waker.take());
            // `Closed` treats a stream that is no longer tracked as gone.
            self.wakes.append(&mut s.closed_wakers);
        }
    }

    /// Stops tracking a ping, wherever it got to.
    pub(super) fn forget_ping(&mut self, key: u64) {
        if let Some(mut p) = self.pings.remove(&key) {
            self.trash.extend(p.waker.take());
        }
    }

    /// Throws away unread body, handing its capacity back to the peer.
    pub(super) fn discard(&mut self, id: u32) {
        let Some(s) = self.streams.get_mut(&id) else {
            return;
        };
        let bytes: usize = s
            .inbox
            .drain(..)
            .map(|i| match i {
                Item::Body(Body::Data(d)) => d.len(),
                _ => 0,
            })
            .sum();
        self.release(id, bytes);
    }

    /// Ends the stream abnormally: the reader sees `fail` (unless it had
    /// already received everything), and senders fail with it.
    pub(super) fn fail_stream(&mut self, id: u32, fail: Fail) {
        let Some(s) = self.streams.get_mut(&id) else {
            return;
        };
        if s.local_closed && s.remote_closed {
            return;
        }
        let push = !s.remote_closed;
        s.abort = Some(fail);
        s.local_closed = true;
        s.remote_closed = true;
        if push {
            self.discard(id);
            if let Some(s) = self.streams.get_mut(&id) {
                s.inbox.push_back(Item::Fail(fail));
            }
        }
        if let Some(s) = self.streams.get_mut(&id) {
            for w in [s.recv_waker.take(), s.headers_waker.take()]
                .into_iter()
                .flatten()
            {
                self.wakes.push(w);
            }
            self.wakes.append(&mut s.closed_wakers);
        }
        self.settle(id);
        // A request that ended before the application accepted it is never
        // offered: reclaim it and its place in the accept queue.
        if self.server && self.streams.get(&id).is_some_and(|s| !s.headers_taken) {
            self.forget_stream(id);
            self.accept_queue.retain(|queued| *queued != id);
        }
    }

    /// Resets `id` toward the peer and fails it here.
    pub(super) fn reset(&mut self, id: u32, code: ErrorCode) {
        let frame = Frame::RstStream(RstStreamFrame {
            stream_id: id,
            error_code: code,
        });
        let _ = self.conn.send_frame(&frame);
        self.queue([frame]);
        self.fail_stream(id, Fail::Reset(code));
    }

    pub(super) fn send_goaway(&mut self, code: ErrorCode) {
        if self.goaway_sent.is_some() {
            return;
        }
        let last = if self.server {
            self.last_peer_stream
        } else {
            0
        };
        self.goaway_sent = Some(last);
        self.queue([Frame::GoAway(GoAwayFrame {
            last_stream_id: last,
            error_code: code,
            debug_data: Vec::new(),
        })]);
    }

    /// The driver is done: nothing more will arrive.
    pub(super) fn close_all(&mut self) {
        self.closed = true;
        let ids: Vec<u32> = self.streams.keys().copied().collect();
        for id in ids {
            self.fail_stream(id, Fail::Closed);
        }
        // Completed streams may remain for unread headers or body. `fail_stream`
        // preserves those reads, but their Closed waiters must also learn the
        // connection ended, even though the stream itself did not fail.
        for s in self.streams.values_mut() {
            self.wakes.append(&mut s.closed_wakers);
        }
        for p in self.pings.values_mut() {
            if let Some(w) = p.waker.take() {
                self.wakes.push(w);
            }
        }
        if let Some(w) = self.accept_waker.take() {
            self.wakes.push(w);
        }
        self.wake_waiters();
    }

    /// Applies one received frame.
    pub(super) fn on_frame(&mut self, frame: Frame) -> Result<(), Error> {
        if let Frame::Ping(PingFrame {
            ack: true,
            opaque_data,
        }) = &frame
        {
            let id = u64::from_be_bytes(*opaque_data);
            if let Some(p) = self.pings.get_mut(&id) {
                p.acked = true;
                if let Some(w) = p.waker.take() {
                    self.wakes.push(w);
                }
            }
            return Ok(());
        }
        let is_settings = matches!(&frame, Frame::Settings(s) if !s.ack);
        let data_len = match &frame {
            Frame::Data(d) => Some((d.stream_id, d.data.len())),
            _ => None,
        };
        match self.conn.apply(frame) {
            Ok(responses) => self.queue(responses),
            Err(H2Error::Stream(id, code, _)) => {
                if let Some((sid, n)) = data_len {
                    self.release(sid, n);
                }
                self.reset(id, code);
            }
            Err(e) => return Err(Error::Protocol(e)),
        }
        if is_settings {
            self.settings_received = true;
        }
        self.dispatch();
        self.wake_waiters();
        Ok(())
    }

    fn dispatch(&mut self) {
        for event in self.conn.take_inbound() {
            match event {
                Inbound::Headers {
                    stream_id,
                    fields,
                    end_stream,
                } => self.on_headers(stream_id, fields, end_stream),
                Inbound::Data {
                    stream_id,
                    data,
                    end_stream,
                } => {
                    match self.streams.get_mut(&stream_id) {
                        Some(s) if !s.recv_dropped && !data.is_empty() => {
                            s.inbox.push_back(Item::Body(Body::Data(data)));
                            if let Some(w) = s.recv_waker.take() {
                                self.wakes.push(w);
                            }
                        }
                        _ => self.release(stream_id, data.len()),
                    }
                    if end_stream {
                        self.close_remote(stream_id);
                    }
                }
                Inbound::Trailers { stream_id, fields } => {
                    if let Some(s) = self.streams.get_mut(&stream_id) {
                        s.inbox.push_back(Item::Body(Body::Trailers(fields)));
                        if let Some(w) = s.recv_waker.take() {
                            self.wakes.push(w);
                        }
                    }
                    self.close_remote(stream_id);
                }
                Inbound::Reset { stream_id, code } => {
                    self.fail_stream(stream_id, Fail::Reset(code))
                }
                Inbound::GoAway {
                    last_stream_id,
                    code,
                } => {
                    // Streams the peer never processed will not be answered.
                    if !self.server {
                        let ids: Vec<u32> = self
                            .streams
                            .keys()
                            .copied()
                            .filter(|id| *id > last_stream_id)
                            .collect();
                        for id in ids {
                            self.fail_stream(id, Fail::GoAway(code));
                        }
                    }
                    if let Some(w) = self.accept_waker.take() {
                        self.wakes.push(w);
                    }
                }
            }
        }
    }

    fn on_headers(&mut self, id: u32, fields: Vec<HeaderField>, end_stream: bool) {
        if self.server {
            let refused = self.closing
                || self.goaway_sent.is_some_and(|last| id > last)
                || self.accept_queue.len() >= MAX_PENDING_ACCEPT;
            if refused {
                self.reset(id, ErrorCode::RefusedStream);
                return;
            }
            self.last_peer_stream = self.last_peer_stream.max(id);
            self.add_stream(id, false);
            self.accept_queue.push_back(id);
            if let Some(w) = self.accept_waker.take() {
                self.wakes.push(w);
            }
        }
        let Some(s) = self.streams.get_mut(&id) else {
            return;
        };
        if s.headers.is_none() && !s.headers_taken {
            s.headers = Some(fields);
        }
        if let Some(w) = s.headers_waker.take() {
            self.wakes.push(w);
        }
        if end_stream {
            self.close_remote(id);
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::connect::{PeerType, ServerSettings};
    use std::sync::atomic::{AtomicBool, Ordering};
    use std::task::Wake;

    /// A waker that, when woken, notes whether the shared mutex is free. A
    /// real waker can drop the last reference to a task, whose destructors
    /// lock this mutex; waking with it held would deadlock.
    struct ProbeWake {
        shared: Handle,
        lock_was_free: AtomicBool,
        woken: AtomicBool,
    }

    impl Wake for ProbeWake {
        fn wake(self: Arc<Self>) {
            self.woken.store(true, Ordering::SeqCst);
            let free = self.shared.try_lock().is_ok();
            self.lock_was_free.store(free, Ordering::SeqCst);
        }
    }

    fn probe() -> (Handle, Arc<ProbeWake>) {
        let conn = Connection::new(ServerSettings::default(), PeerType::Server);
        let shared: Handle = Arc::new(Mutex::new(Shared::new(conn, true, false)));
        let waker = Arc::new(ProbeWake {
            shared: shared.clone(),
            lock_was_free: AtomicBool::new(false),
            woken: AtomicBool::new(false),
        });
        (shared, waker)
    }

    #[test]
    fn wakers_run_after_the_lock_is_released() {
        let (shared, probe) = probe();
        {
            let mut sh = lock(&shared);
            sh.add_stream(1, false);
            sh.streams.get_mut(&1).unwrap().recv_waker = Some(Waker::from(probe.clone()));
            sh.fail_stream(1, Fail::Closed);
        }
        assert!(probe.woken.load(Ordering::SeqCst), "the reader is woken");
        assert!(
            probe.lock_was_free.load(Ordering::SeqCst),
            "it must be woken only once the mutex is free"
        );
    }

    #[test]
    fn every_kind_of_wake_waits_for_the_lock_to_be_released() {
        let (shared, probe) = probe();
        {
            let mut sh = lock(&shared);
            sh.driver_waker = Some(Waker::from(probe.clone()));
            sh.queue([]);
        }
        assert!(probe.woken.load(Ordering::SeqCst));
        assert!(probe.lock_was_free.load(Ordering::SeqCst), "driver wake");

        let (shared, probe) = self::probe();
        {
            let mut sh = lock(&shared);
            register(&mut sh.waiters, &Waker::from(probe.clone()));
            sh.wake_waiters();
        }
        assert!(probe.woken.load(Ordering::SeqCst));
        assert!(probe.lock_was_free.load(Ordering::SeqCst), "waiter wake");

        let (shared, probe) = self::probe();
        {
            let mut sh = lock(&shared);
            sh.accept_waker = Some(Waker::from(probe.clone()));
            sh.close_all();
        }
        assert!(probe.woken.load(Ordering::SeqCst));
        assert!(probe.lock_was_free.load(Ordering::SeqCst), "accept wake");
    }

    #[test]
    fn a_forgotten_streams_wakers_are_dropped_outside_the_lock() {
        use std::sync::atomic::{AtomicBool, Ordering};
        struct Probe {
            shared: Handle,
            free: Arc<AtomicBool>,
            dropped: Arc<AtomicBool>,
        }
        // Not `Waker::noop()`: the point is what happens when it is destroyed.
        #[allow(clippy::manual_noop_waker)]
        impl Wake for Probe {
            fn wake(self: Arc<Self>) {}
        }
        impl Drop for Probe {
            fn drop(&mut self) {
                self.free
                    .store(self.shared.try_lock().is_ok(), Ordering::SeqCst);
                self.dropped.store(true, Ordering::SeqCst);
            }
        }
        let (shared, _) = probe();
        let (free, dropped) = (
            Arc::new(AtomicBool::new(false)),
            Arc::new(AtomicBool::new(false)),
        );
        {
            let mut sh = lock(&shared);
            sh.add_stream(1, true);
            let s = sh.streams.get_mut(&1).unwrap();
            s.remote_closed = true;
            s.headers_taken = true;
            s.recv_waker = Some(Waker::from(Arc::new(Probe {
                shared: shared.clone(),
                free: free.clone(),
                dropped: dropped.clone(),
            })));
            sh.settle(1); // both halves closed and nothing to read: forgotten
            assert!(!sh.streams.contains_key(&1));
        }
        assert!(dropped.load(Ordering::SeqCst));
        assert!(
            free.load(Ordering::SeqCst),
            "dropped while the mutex was held"
        );
    }
}
