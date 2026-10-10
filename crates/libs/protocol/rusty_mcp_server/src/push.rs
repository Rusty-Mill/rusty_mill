//! The standalone server-to-client stream of a classic HTTP session: what a
//! `GET` with `Accept: text/event-stream` and an `Mcp-Session-Id` opens.
//!
//! It carries the server's own notifications (list changes, and updates to
//! the resources the session followed with `resources/subscribe`). Events are
//! collected from the session's creation on, in a bounded buffer, so a client
//! that connects late or reconnects loses nothing unless it was away longer
//! than the buffer: then it is told everything it follows may have changed.
//! Each frame has an event id; a reconnect with `Last-Event-ID` replays the
//! frames it missed from a short ring of recent ones. One stream per session:
//! a new `GET` takes over from the old one, which ends. (A client reconnecting
//! after a network drop must not be refused because the server has not yet
//! noticed the old connection die.) Delivery is at most once unless the client
//! resumes with `Last-Event-ID`: a frame written to a connection that was
//! already dead is gone.

use crate::changes::{ChangeEvent, ChangeKinds, Subscription};
use crate::methods::{notification_for, resync_events, session_filter};
use rusty_mcp_proto::{Message, Wire};
use std::collections::{BTreeSet, VecDeque};
use std::sync::atomic::{AtomicBool, AtomicU64, Ordering};
use std::sync::mpsc::RecvTimeoutError;
use std::sync::{Arc, Mutex, MutexGuard, PoisonError};
use std::time::{Duration, Instant};

/// Recent frames kept for `Last-Event-ID` replay.
const RING: usize = 64;
/// How often an idle stream looks up to check for a closed session.
const POLL: Duration = Duration::from_millis(50);

fn lock<T>(m: &Mutex<T>) -> MutexGuard<'_, T> {
    m.lock().unwrap_or_else(PoisonError::into_inner)
}

#[derive(Default)]
struct Ring {
    next_id: u64,
    frames: VecDeque<(u64, Vec<u8>)>,
}

impl Ring {
    fn push(&mut self, message: &Message) -> Vec<u8> {
        self.next_id += 1;
        let frame = format!(
            "id: {}\nevent: message\ndata: {}\n\n",
            self.next_id,
            message.to_json()
        )
        .into_bytes();
        if self.frames.len() == RING {
            self.frames.pop_front();
        }
        self.frames.push_back((self.next_id, frame.clone()));
        frame
    }

    /// Frames after `last`, and whether some were lost (older than the ring).
    fn after(&self, last: u64) -> (Vec<Vec<u8>>, bool) {
        let newest = self.next_id;
        let oldest = self.frames.front().map_or(newest + 1, |(id, _)| *id);
        let lost = last + 1 < oldest;
        let frames = self
            .frames
            .iter()
            .filter(|(id, _)| *id > last)
            .map(|(_, f)| f.clone())
            .collect();
        (frames, lost)
    }
}

/// The push side of one session.
pub(crate) struct Push {
    kinds: ChangeKinds,
    subscription: Mutex<Option<Subscription>>,
    subscribed: Mutex<BTreeSet<String>>,
    ring: Mutex<Ring>,
    /// Counts streams; only the newest one runs.
    generation: AtomicU64,
    closed: AtomicBool,
}

impl Push {
    /// `subscription` is `None` for a server that announces no changes: then
    /// there is nothing to push and no stream is offered.
    pub(crate) fn new(subscription: Option<Subscription>, kinds: ChangeKinds) -> Arc<Self> {
        Arc::new(Self {
            kinds,
            subscription: Mutex::new(subscription),
            subscribed: Mutex::new(BTreeSet::new()),
            ring: Mutex::new(Ring::default()),
            generation: AtomicU64::new(0),
            closed: AtomicBool::new(false),
        })
    }

    pub(crate) fn subscribe(&self, uri: &str) {
        lock(&self.subscribed).insert(uri.to_owned());
    }

    pub(crate) fn unsubscribe(&self, uri: &str) {
        lock(&self.subscribed).remove(uri);
    }

    /// End the stream (the session was deleted).
    pub(crate) fn close(&self) {
        self.closed.store(true, Ordering::SeqCst);
    }

    /// Open the stream, replaying what `last_event_id` missed, and end any
    /// older one. `touch` marks the session in use.
    pub(crate) fn attach(
        self: &Arc<Self>,
        last_event_id: Option<u64>,
        keep_alive: Duration,
        touch: Box<dyn Fn() + Send>,
    ) -> PushStream {
        let mine = self.generation.fetch_add(1, Ordering::SeqCst) + 1;
        let mut ready = VecDeque::new();
        if let Some(last) = last_event_id {
            let (frames, lost) = lock(&self.ring).after(last);
            ready.extend(frames);
            if lost {
                ready.extend(self.resync());
            }
        }
        PushStream {
            push: Arc::clone(self),
            mine,
            ready,
            keep_alive,
            last_write: Instant::now(),
            touch,
        }
    }

    fn filter(&self) -> rusty_mcp_proto::SubscriptionFilter {
        session_filter(&self.kinds, &lock(&self.subscribed))
    }

    /// Frames saying everything the session follows may have changed.
    fn resync(&self) -> Vec<Vec<u8>> {
        let filter = self.filter();
        resync_events(&filter)
            .iter()
            .filter_map(|e| notification_for(e, &filter, None))
            .map(|m| lock(&self.ring).push(&m))
            .collect()
    }

    fn frame_for(&self, event: &ChangeEvent) -> Option<Vec<u8>> {
        let message = notification_for(event, &self.filter(), None)?;
        Some(lock(&self.ring).push(&message))
    }
}

/// The body of the `GET` reply. Ends when the session is deleted, a newer
/// stream replaces it, or the broadcaster closes.
pub(crate) struct PushStream {
    push: Arc<Push>,
    mine: u64,
    ready: VecDeque<Vec<u8>>,
    keep_alive: Duration,
    last_write: Instant,
    touch: Box<dyn Fn() + Send>,
}

impl Iterator for PushStream {
    type Item = Vec<u8>;

    fn next(&mut self) -> Option<Vec<u8>> {
        loop {
            if let Some(frame) = self.ready.pop_front() {
                self.last_write = Instant::now();
                return Some(frame);
            }
            if self.push.closed.load(Ordering::SeqCst)
                || self.push.generation.load(Ordering::SeqCst) != self.mine
            {
                return None;
            }
            (self.touch)();
            let guard = lock(&self.push.subscription);
            let subscription = guard.as_ref()?;
            match subscription.events.recv_timeout(POLL) {
                Ok(event) => {
                    drop(guard);
                    if let Some(frame) = self.push.frame_for(&event) {
                        self.last_write = Instant::now();
                        return Some(frame);
                    }
                }
                Err(RecvTimeoutError::Timeout) => {
                    let lagged = subscription.lagged.swap(false, Ordering::SeqCst);
                    drop(guard);
                    if lagged {
                        self.ready.extend(self.push.resync());
                    } else if self.last_write.elapsed() >= self.keep_alive {
                        self.last_write = Instant::now();
                        return Some(b": ping\n\n".to_vec());
                    }
                }
                Err(RecvTimeoutError::Disconnected) => return None,
            }
        }
    }
}

#[cfg(test)]
#[allow(clippy::unwrap_used)]
mod tests {
    use super::*;

    fn note(n: &str) -> Message {
        Message::Notification {
            method: n.to_owned(),
            params: None,
        }
    }

    #[test]
    fn frames_have_increasing_ids_and_replay_after_a_given_one() {
        let mut ring = Ring::default();
        for n in ["a", "b", "c"] {
            ring.push(&note(n));
        }
        let (frames, lost) = ring.after(1);
        assert!(!lost);
        assert_eq!(frames.len(), 2);
        assert!(String::from_utf8_lossy(&frames[0]).starts_with("id: 2\n"));
        let (all, lost) = ring.after(0);
        assert_eq!((all.len(), lost), (3, false));
        let (none, lost) = ring.after(3);
        assert_eq!((none.len(), lost), (0, false));
    }

    #[test]
    fn a_gap_older_than_the_ring_is_reported_as_lost() {
        let mut ring = Ring::default();
        for i in 0..(RING + 5) {
            ring.push(&note(&format!("n{i}")));
        }
        let (frames, lost) = ring.after(2);
        assert!(lost, "ids 3..=5 were pushed out of the ring");
        assert_eq!(frames.len(), RING);
        let (_, lost) = ring.after(10);
        assert!(!lost, "id 11 is still held");
    }
}
