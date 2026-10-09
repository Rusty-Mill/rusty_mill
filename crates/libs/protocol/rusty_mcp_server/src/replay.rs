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
//! stateless call. Streams share a byte budget: the oldest finished stream is
//! dropped first, then the oldest running one (a resume of a dropped stream
//! is then a `404`, never a wrong answer).

use rusty_mcp_proto::{Message, Wire};
use std::collections::{HashMap, VecDeque};
use std::sync::{Arc, Condvar, Mutex, MutexGuard, PoisonError};
use std::time::{Duration, Instant};

fn lock<T>(m: &Mutex<T>) -> MutexGuard<'_, T> {
    // The guarded data stays valid if a holder panicked.
    m.lock().unwrap_or_else(PoisonError::into_inner)
}

/// What a stream keeps: its frames, and who is reading.
#[derive(Default)]
struct State {
    frames: Vec<Vec<u8>>,
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
    core: Arc<Core>,
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
            core: Arc::clone(&self.0),
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
        Some((shared, n))
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
            state.frames.push(frame);
        }
        self.changed.notify_all();
        self.core.account(size, &self.token);
    }

    /// Add `message` as the next frame.
    pub(crate) fn push(&self, message: &Message) {
        let n = lock(&self.state).frames.len();
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
        }
    }
}

impl Core {
    /// Count `size` new bytes and, over budget, drop the oldest streams: the
    /// finished ones first, never the stream that just grew.
    fn account(&self, size: usize, growing: &str) {
        let mut registry = lock(&self.registry);
        registry.bytes += size;
        while registry.bytes > self.budget {
            let victim = registry
                .order
                .iter()
                .filter(|t| t.as_str() != growing)
                .find(|t| {
                    registry
                        .streams
                        .get(*t)
                        .is_some_and(|s| lock(&s.state).done)
                })
                .or_else(|| registry.order.iter().find(|t| t.as_str() != growing))
                .cloned();
            let Some(token) = victim else { return };
            registry.order.retain(|t| *t != token);
            if let Some(gone) = registry.streams.remove(&token) {
                registry.bytes = registry.bytes.saturating_sub(lock(&gone.state).bytes);
            }
        }
    }
}

/// The body of a resumable reply, or of a resume: frames from `next` on, a
/// keep-alive comment while waiting, ending when the stream is finished.
pub(crate) struct ReplayStream {
    shared: Arc<Shared>,
    next: usize,
    keep_alive: Duration,
}

impl Iterator for ReplayStream {
    type Item = Vec<u8>;

    fn next(&mut self) -> Option<Vec<u8>> {
        let mut state = lock(&self.shared.state);
        loop {
            if let Some(frame) = state.frames.get(self.next) {
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
            if timeout.timed_out() && state.frames.get(self.next).is_none() && !state.done {
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
