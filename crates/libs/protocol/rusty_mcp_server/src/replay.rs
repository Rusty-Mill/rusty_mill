//! Resumable replies: the event-stream answer to a `POST`, kept for a
//! while so a client whose connection dropped can fetch what it missed.
//!
//! Off unless [`HttpConfig::resume_buffer`](crate::HttpConfig) is set. Then
//! every answer to a request is an event stream that opens with a priming
//! event (an id and no data, so the client has something to resume from), and
//! every frame carries an id. A `GET` with `Last-Event-ID` and no session
//! replays the frames after that id, then follows the stream live until it
//! ends.
//!
//! An id is `s<token>-<n>`: the token is 64 random bits naming the stream,
//! so the id is the credential for the stream (a guessed id finds nothing).
//! The request keeps running when its connection drops; if no reader is
//! attached for [`HttpConfig::resume_grace`](crate::HttpConfig) it is
//! cancelled, which is how a client that hangs up on purpose still stops a
//! stateless call. Streams share a byte budget, and it bounds memory even for
//! streams still running: over budget, the oldest finished stream is released
//! whole, then the oldest frames of running streams (always keeping each one's
//! newest frame). A resume that needs a released frame is a `404`, and a
//! reader that had not yet read a released frame is ended (with a comment
//! line saying so), never skipped past silently.

use rusty_mcp_proto::{Message, Wire};
use std::collections::{HashMap, VecDeque};
use std::sync::{Arc, Condvar, Mutex, MutexGuard, PoisonError, Weak};
use std::time::{Duration, Instant};

fn lock<T>(m: &Mutex<T>) -> MutexGuard<'_, T> {
    // The guarded data stays valid if a holder panicked.
    m.lock().unwrap_or_else(PoisonError::into_inner)
}

/// What a stream keeps: its frames, and who is reading.
#[derive(Default)]
struct State {
    /// The frames still held; `frames[0]` is frame number `base`.
    frames: VecDeque<Vec<u8>>,
    base: usize,
    /// Bytes held in `frames`.
    bytes: usize,
    done: bool,
    readers: usize,
    /// When the last reader left, if none is attached now.
    detached: Option<Instant>,
}

/// One stream's shared record.
pub(crate) struct Shared {
    token: String,
    state: Mutex<State>,
    changed: Condvar,
    /// Weak, so a registry entry (which holds this) does not keep the
    /// registry alive: dropping the [`Replay`] frees everything.
    core: Weak<Core>,
}

#[derive(Default)]
struct Registry {
    streams: HashMap<String, Arc<Shared>>,
    /// Tokens, oldest first.
    order: VecDeque<String>,
    bytes: usize,
}

struct Core {
    budget: usize,
    grace: Duration,
    registry: Mutex<Registry>,
}

/// The streams of one handler.
pub(crate) struct Replay(Arc<Core>);

impl Replay {
    pub(crate) fn new(budget: usize, grace: Duration) -> Self {
        Self(Arc::new(Core {
            budget,
            grace,
            registry: Mutex::new(Registry::default()),
        }))
    }

    pub(crate) fn grace(&self) -> Duration {
        self.0.grace
    }

    /// A new stream, holding its priming event. `None` without randomness.
    pub(crate) fn start(&self) -> Option<Arc<Shared>> {
        let mut raw = [0u8; 8];
        rusty_rand::fill(&mut raw).ok()?;
        let token: String = raw.iter().map(|b| format!("{b:02x}")).collect();
        let shared = Arc::new(Shared {
            token: token.clone(),
            state: Mutex::new(State::default()),
            changed: Condvar::new(),
            core: Arc::downgrade(&self.0),
        });
        shared.append(format!("id: {}\ndata: \n\n", shared.event_id(0)).into_bytes());
        let mut registry = lock(&self.0.registry);
        registry.streams.insert(token.clone(), Arc::clone(&shared));
        registry.order.push_back(token);
        Some(shared)
    }

    /// The stream an event id belongs to, and the number of that event.
    pub(crate) fn find(&self, id: &str) -> Option<(Arc<Shared>, usize)> {
        let (token, n) = id.strip_prefix('s')?.split_once('-')?;
        let n: usize = n.parse().ok()?;
        let shared = lock(&self.0.registry).streams.get(token).cloned()?;
        // The frames after `n` must still be held; if some were released to
        // stay in budget, the resume cannot be honoured.
        let held = n + 1 >= lock(&shared.state).base;
        held.then_some((shared, n))
    }
}

impl Shared {
    fn event_id(&self, n: usize) -> String {
        format!("s{}-{n}", self.token)
    }

    fn append(&self, frame: Vec<u8>) {
        let size = frame.len();
        {
            let mut state = lock(&self.state);
            state.bytes += size;
            state.frames.push_back(frame);
        }
        self.changed.notify_all();
        if let Some(core) = self.core.upgrade() {
            core.account(size);
        }
    }

    /// Add `message` as the next frame.
    pub(crate) fn push(&self, message: &Message) {
        let n = {
            let state = lock(&self.state);
            state.base + state.frames.len()
        };
        self.append(
            format!(
                "id: {}\nevent: message\ndata: {}\n\n",
                self.event_id(n),
                message.to_json()
            )
            .into_bytes(),
        );
    }

    /// No more frames will come.
    pub(crate) fn finish(&self) {
        lock(&self.state).done = true;
        self.changed.notify_all();
    }

    /// How long nobody has been reading, while the stream is unfinished.
    pub(crate) fn abandoned_for(&self) -> Option<Duration> {
        let state = lock(&self.state);
        if state.done || state.readers > 0 {
            return None;
        }
        state.detached.map(|since| since.elapsed())
    }

    /// A reader starting at frame `from`.
    pub(crate) fn reader(self: &Arc<Self>, from: usize, keep_alive: Duration) -> ReplayStream {
        {
            let mut state = lock(&self.state);
            state.readers += 1;
            state.detached = None;
        }
        ReplayStream {
            shared: Arc::clone(self),
            next: from,
            keep_alive,
            ended: false,
        }
    }
}

impl Core {
    /// Count `size` new bytes and, over budget, release the oldest data: first
    /// a whole finished stream, then the oldest frames of running ones (each
    /// keeps its newest frame). Stops when nothing more can be released.
    fn account(&self, size: usize) {
        let mut registry = lock(&self.registry);
        registry.bytes += size;
        while registry.bytes > self.budget {
            let finished = registry
                .order
                .iter()
                .find(|t| {
                    registry
                        .streams
                        .get(*t)
                        .is_some_and(|s| lock(&s.state).done)
                })
                .cloned();
            if let Some(token) = finished {
                registry.order.retain(|t| *t != token);
                if let Some(gone) = registry.streams.remove(&token) {
                    registry.bytes = registry.bytes.saturating_sub(gone.release_all());
                }
                continue;
            }
            let trimmed = registry.order.iter().find_map(|t| {
                registry
                    .streams
                    .get(t)
                    .and_then(|s| s.release_oldest_frame())
            });
            match trimmed {
                Some(freed) => registry.bytes = registry.bytes.saturating_sub(freed),
                None => return,
            }
        }
    }
}

impl Shared {
    /// Drop every frame; readers behind see a gap. Returns the bytes freed.
    fn release_all(&self) -> usize {
        let mut state = lock(&self.state);
        let freed = state.bytes;
        state.base += state.frames.len();
        state.frames.clear();
        state.bytes = 0;
        drop(state);
        self.changed.notify_all();
        freed
    }

    /// Drop the oldest frame if a newer one remains. Returns the bytes freed.
    fn release_oldest_frame(&self) -> Option<usize> {
        let mut state = lock(&self.state);
        if state.frames.len() < 2 {
            return None;
        }
        let frame = state.frames.pop_front()?;
        state.base += 1;
        state.bytes = state.bytes.saturating_sub(frame.len());
        Some(frame.len())
    }
}

/// The body of a resumable reply, or of a resume: frames from `next` on, a
/// keep-alive comment while waiting, ending when the stream is finished.
pub(crate) struct ReplayStream {
    shared: Arc<Shared>,
    next: usize,
    keep_alive: Duration,
    ended: bool,
}

impl Iterator for ReplayStream {
    type Item = Vec<u8>;

    fn next(&mut self) -> Option<Vec<u8>> {
        if self.ended {
            return None;
        }
        let mut state = lock(&self.shared.state);
        loop {
            if self.next < state.base {
                // The frame this reader needs was released to stay in budget.
                self.ended = true;
                return Some(b": stream truncated (over the replay budget)\n\n".to_vec());
            }
            if let Some(frame) = state.frames.get(self.next - state.base) {
                self.next += 1;
                return Some(frame.clone());
            }
            if state.done {
                return None;
            }
            let (guard, timeout) = self
                .shared
                .changed
                .wait_timeout(state, self.keep_alive)
                .unwrap_or_else(PoisonError::into_inner);
            state = guard;
            if timeout.timed_out()
                && self.next >= state.base
                && state.frames.get(self.next - state.base).is_none()
                && !state.done
            {
                return Some(b": ping\n\n".to_vec());
            }
        }
    }
}

impl Drop for ReplayStream {
    fn drop(&mut self) {
        let mut state = lock(&self.shared.state);
        state.readers = state.readers.saturating_sub(1);
        if state.readers == 0 {
            state.detached = Some(Instant::now());
        }
    }
}

#[cfg(test)]
#[allow(clippy::unwrap_used)]
mod tests {
    use super::*;

    fn note() -> Message {
        Message::Notification {
            method: "notifications/progress".to_owned(),
            params: None,
        }
    }

    fn held_bytes(shared: &Shared) -> usize {
        lock(&shared.state).frames.iter().map(Vec::len).sum()
    }

    fn registry_bytes(replay: &Replay) -> usize {
        lock(&replay.0.registry).bytes
    }

    #[test]
    fn running_streams_stay_within_the_budget_with_readers_attached() {
        let budget = 2_000;
        let replay = Replay::new(budget, Duration::from_secs(30));
        let streams: Vec<_> = (0..3).map(|_| replay.start().unwrap()).collect();
        // Readers are attached and never read: the case that used to pin
        // every frame in memory.
        let readers: Vec<_> = streams
            .iter()
            .map(|s| s.reader(0, Duration::from_millis(50)))
            .collect();
        for _ in 0..500 {
            for s in &streams {
                s.push(&note());
            }
        }
        let frame = format!("{}", 200); // a generous bound on one frame's size
        let one_frame = frame.len() + 150;
        let accounted = registry_bytes(&replay);
        let actual: usize = streams.iter().map(|s| held_bytes(s)).sum();
        assert_eq!(
            accounted, actual,
            "the budget count drifted from what is held"
        );
        assert!(
            accounted <= budget + streams.len() * one_frame,
            "{accounted} bytes held over a budget of {budget}"
        );
        assert!(
            streams.iter().all(|s| !lock(&s.state).frames.is_empty()),
            "a running stream lost its newest frame"
        );
        drop(readers);
    }

    #[test]
    fn a_reader_behind_a_released_frame_is_ended_not_skipped_ahead() {
        let replay = Replay::new(600, Duration::from_secs(30));
        let stream = replay.start().unwrap();
        let mut reader = stream.reader(0, Duration::from_millis(20));
        for _ in 0..100 {
            stream.push(&note());
        }
        let first = reader.next().unwrap();
        assert!(
            String::from_utf8_lossy(&first).starts_with(": stream truncated"),
            "got {:?}",
            String::from_utf8_lossy(&first)
        );
        assert!(reader.next().is_none(), "the stream went on after the gap");
    }

    #[test]
    fn a_resume_that_needs_a_released_frame_is_not_found() {
        let replay = Replay::new(600, Duration::from_secs(30));
        let stream = replay.start().unwrap();
        let early = stream.event_id(0);
        for _ in 0..100 {
            stream.push(&note());
        }
        assert!(
            replay.find(&early).is_none(),
            "an unservable resume was accepted"
        );
        let last = stream.event_id(lock(&stream.state).base + lock(&stream.state).frames.len() - 1);
        assert!(
            replay.find(&last).is_some(),
            "the newest frame is still resumable"
        );
    }

    #[test]
    fn a_finished_stream_is_released_whole_and_its_readers_end() {
        let replay = Replay::new(400, Duration::from_secs(30));
        let old = replay.start().unwrap();
        let mut old_reader = old.reader(0, Duration::from_millis(20));
        old.push(&note());
        old.finish();
        let newer = replay.start().unwrap();
        for _ in 0..50 {
            newer.push(&note());
        }
        assert_eq!(held_bytes(&old), 0, "the finished stream kept its frames");
        let first = old_reader.next().unwrap();
        assert!(String::from_utf8_lossy(&first).starts_with(": stream truncated"));
        assert!(replay.find(&old.event_id(0)).is_none());
    }

    #[test]
    fn dropping_the_replay_releases_its_streams() {
        let replay = Replay::new(1_000, Duration::from_secs(30));
        let stream = replay.start().unwrap();
        stream.push(&note());
        let weak = Arc::downgrade(&stream);
        drop(stream);
        // Only the registry holds it now; dropping the replay must free it.
        assert!(weak.upgrade().is_some());
        drop(replay);
        assert!(
            weak.upgrade().is_none(),
            "a stream outlived its replay (a reference cycle)"
        );
    }
}
