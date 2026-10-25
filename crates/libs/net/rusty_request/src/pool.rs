//! A simple per-[`crate::Client`] idle-connection pool: hands back a
//! [`Conn`] (plain or TLS-wrapped) for reuse on a subsequent request to
//! the same (scheme, host, port) origin, when the previous response left
//! it in a reusable state (see `http1::RawResponse::keep_alive`).
//!
//! No pipelining -- a pooled connection is only ever handed to one
//! in-flight request at a time, exactly matching `http1::read_body`'s
//! own "never more than one response in flight" assumption.

use crate::stream::Conn;
use rusty_tokio::io::{AsyncRead, ReadBuf};
use std::collections::HashMap;
use std::pin::Pin;
use std::sync::Mutex;
use std::task::{Context, Poll, Waker};
use std::time::{Duration, Instant};

pub(crate) type PoolKey = (String, String, u16);

struct Idle {
    stream: Conn,
    since: Instant,
}

pub(crate) struct ConnectionPool {
    idle: Mutex<HashMap<PoolKey, Vec<Idle>>>,
    max_idle_per_origin: usize,
    idle_timeout: Duration,
}

impl ConnectionPool {
    pub(crate) fn new(max_idle_per_origin: usize, idle_timeout: Duration) -> Self {
        ConnectionPool {
            idle: Mutex::new(HashMap::new()),
            max_idle_per_origin,
            idle_timeout,
        }
    }

    /// Takes the most recently idled connection for `key`, if one
    /// exists, hasn't sat idle past the configured timeout, and still
    /// looks open (see [`still_open`]). LIFO, not FIFO: the most recently
    /// returned connection is the one least likely to have been closed by
    /// the peer's own idle timeout in the meantime.
    pub(crate) fn take(&self, key: &PoolKey) -> Option<Conn> {
        let mut idle = self.idle.lock().unwrap();
        let entries = idle.get_mut(key)?;
        let now = Instant::now();
        while let Some(entry) = entries.pop() {
            let mut stream = entry.stream;
            if now.duration_since(entry.since) < self.idle_timeout && still_open(&mut stream) {
                return Some(stream);
            }
            // Past the idle timeout, or closed by the peer while idle --
            // drop it (closing the socket) and keep looking at the
            // next-most-recently-idled entry.
        }
        None
    }

    /// Returns a connection to the pool for reuse, unless the
    /// per-origin idle list is already at capacity or pooling is
    /// disabled (`max_idle_per_origin == 0`) -- either way, `stream` is
    /// simply dropped (closing it), bounding how many idle connections
    /// a `Client` can accumulate.
    pub(crate) fn put(&self, key: PoolKey, stream: Conn) {
        if self.max_idle_per_origin == 0 {
            return;
        }
        let mut idle = self.idle.lock().unwrap();
        let entries = idle.entry(key).or_default();
        if entries.len() < self.max_idle_per_origin {
            entries.push(Idle {
                stream,
                since: Instant::now(),
            });
        }
    }
}

/// Whether an idle connection can still carry a request: one read
/// attempt that must find nothing waiting. A peer that closed the
/// connection while it sat idle shows as end of stream (or an error), and
/// one that sent bytes nobody asked for has left it out of step; either
/// way it is not reused. This catches the common stale case -- the
/// server's own idle timeout fired first -- without ever sending a
/// request on a connection already known to be dead. The read never
/// waits: the no-op waker it registers is replaced by the next real poll.
fn still_open(stream: &mut Conn) -> bool {
    let mut byte = [0u8; 1];
    let mut buf = ReadBuf::new(&mut byte);
    let mut cx = Context::from_waker(Waker::noop());
    matches!(Pin::new(stream).poll_read(&mut cx, &mut buf), Poll::Pending)
}

/// Manual impl -- `Conn` doesn't implement `Debug` (neither of its
/// variants do), so `#[derive(Debug)]` isn't an option here.
impl std::fmt::Debug for ConnectionPool {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("ConnectionPool")
            .field("max_idle_per_origin", &self.max_idle_per_origin)
            .field("idle_timeout", &self.idle_timeout)
            .finish_non_exhaustive()
    }
}
