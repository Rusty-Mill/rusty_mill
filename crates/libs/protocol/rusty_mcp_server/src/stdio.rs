//! The stdio transport: one JSON-RPC message per line on stdin, replies on
//! stdout, logs (yours) on stderr. Requests run concurrently, one thread
//! each, so a slow tool never blocks the next message, which is what lets a
//! cancellation reach it.

use crate::connection::{Connection, Job, Notifier, Started};
use crate::server::Server;
use rusty_mcp_proto::{Error as ProtoError, ErrorCode, ErrorData, Message, Wire};
use std::io::{self, BufRead, BufReader, Read, Write};
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{Arc, Condvar, Mutex, PoisonError};
use std::time::Duration;

/// The longest line accepted unless [`StdioConfig`] says otherwise.
pub const DEFAULT_MAX_LINE_BYTES: usize = 4 * 1024 * 1024;

/// How long running requests get to finish after end of input unless
/// [`StdioConfig`] says otherwise.
pub const DEFAULT_DRAIN_TIMEOUT: Duration = Duration::from_secs(10);

/// How long a cancelled request gets to notice before the server gives up
/// on it.
const CANCEL_GRACE: Duration = Duration::from_millis(500);

/// Bounds of the stdio loop.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct StdioConfig {
    /// A longer line is answered with an error and skipped.
    pub max_line_bytes: usize,
    /// After end of input, how long requests still running may take before
    /// they are cancelled. A tool that ignores cancellation is abandoned
    /// after a short grace period, so the server can always exit.
    pub drain_timeout: Duration,
}

impl Default for StdioConfig {
    fn default() -> Self {
        Self {
            max_line_bytes: DEFAULT_MAX_LINE_BYTES,
            drain_timeout: DEFAULT_DRAIN_TIMEOUT,
        }
    }
}

/// Writes one message per line; once a write fails it stops trying.
struct LineSink<W> {
    out: Mutex<W>,
    failed: AtomicBool,
    /// Requests whose reply has not been written yet.
    pending: Arc<Pending>,
}

/// A count of replies still owed, with a way to wait for it to reach zero.
/// Distinct from the connection's in-flight table, which frees a request's
/// id just before its reply is written.
#[derive(Default)]
struct Pending {
    count: Mutex<usize>,
    zero: Condvar,
}

/// One owed reply; dropping it (after the write, or if the worker never
/// started) settles the debt.
struct Ticket(Arc<Pending>);

impl Pending {
    fn enter(self: &Arc<Self>) -> Ticket {
        *self.count.lock().unwrap_or_else(PoisonError::into_inner) += 1;
        Ticket(Arc::clone(self))
    }

    /// Wait for no replies to be owed, for at most `timeout`.
    fn wait_zero(&self, timeout: Duration) -> bool {
        let count = self.count.lock().unwrap_or_else(PoisonError::into_inner);
        let (_count, result) = self
            .zero
            .wait_timeout_while(count, timeout, |n| *n > 0)
            .unwrap_or_else(PoisonError::into_inner);
        !result.timed_out()
    }
}

impl Drop for Ticket {
    fn drop(&mut self) {
        let mut count = self.0.count.lock().unwrap_or_else(PoisonError::into_inner);
        *count -= 1;
        if *count == 0 {
            self.0.zero.notify_all();
        }
    }
}

impl<W: Write> LineSink<W> {
    fn send(&self, message: &Message) {
        if self.failed.load(Ordering::SeqCst) {
            return;
        }
        let mut line = message.to_json();
        line.push('\n');
        let mut out = self.out.lock().unwrap_or_else(PoisonError::into_inner);
        if out
            .write_all(line.as_bytes())
            .and_then(|()| out.flush())
            .is_err()
        {
            self.failed.store(true, Ordering::SeqCst);
        }
    }
}

impl<W: Write + Send> Notifier for LineSink<W> {
    fn notify(&self, message: Message) {
        self.send(&message);
    }
}

/// Serve `server` on the process's stdin and stdout until stdin closes, then
/// let the requests still running finish (see [`StdioConfig::drain_timeout`]).
///
/// # Errors
/// An I/O error reading stdin, or [`io::ErrorKind::BrokenPipe`] when stdout
/// went away.
pub fn serve_stdio(server: Arc<Server>) -> io::Result<()> {
    serve_lines(
        server,
        BufReader::new(io::stdin()),
        io::stdout(),
        StdioConfig::default(),
    )
}

fn parse(line: &[u8]) -> Result<Message, Message> {
    let reject = |code, why: &str| Message::error(None, ErrorData::new(code, why));
    let text =
        std::str::from_utf8(line).map_err(|_| reject(ErrorCode::PARSE_ERROR, "not UTF-8"))?;
    Message::from_json(text).map_err(|e| match e {
        ProtoError::Json(_) => reject(ErrorCode::PARSE_ERROR, "not JSON"),
        ProtoError::Decode { .. } => reject(ErrorCode::INVALID_REQUEST, &e.to_string()),
    })
}

/// Discard input up to and including the next newline.
fn skip_line<R: BufRead>(reader: &mut R) -> io::Result<()> {
    let mut scratch = Vec::new();
    loop {
        scratch.clear();
        let n = reader.by_ref().take(8192).read_until(b'\n', &mut scratch)?;
        if n == 0 || scratch.last() == Some(&b'\n') {
            return Ok(());
        }
    }
}

/// [`serve_stdio`] over any reader and writer.
///
/// # Errors
/// As [`serve_stdio`].
pub fn serve_lines<R, W>(
    server: Arc<Server>,
    mut reader: R,
    writer: W,
    config: StdioConfig,
) -> io::Result<()>
where
    R: BufRead,
    W: Write + Send + 'static,
{
    let sink = Arc::new(LineSink {
        out: Mutex::new(writer),
        failed: AtomicBool::new(false),
        pending: Arc::new(Pending::default()),
    });
    let conn = Arc::new(Connection::new(server, sink.clone()));
    let read = read_loop(&conn, &sink, &mut reader, config.max_line_bytes);
    // End of input: let running requests finish, then cancel what is left. A
    // tool that still ignores the flag is abandoned; its thread dies with
    // the process.
    // Open listeners are not work to wait for.
    conn.close_streams();
    if !sink.pending.wait_zero(config.drain_timeout) {
        conn.cancel_all();
        sink.pending.wait_zero(CANCEL_GRACE);
    }
    read?;
    if sink.failed.load(Ordering::SeqCst) {
        return Err(io::ErrorKind::BrokenPipe.into());
    }
    Ok(())
}

fn read_loop<R: BufRead, W: Write + Send + 'static>(
    conn: &Arc<Connection>,
    sink: &Arc<LineSink<W>>,
    reader: &mut R,
    max_line_bytes: usize,
) -> io::Result<()> {
    let mut line = Vec::new();
    loop {
        if sink.failed.load(Ordering::SeqCst) {
            return Err(io::ErrorKind::BrokenPipe.into());
        }
        line.clear();
        let n = reader
            .by_ref()
            .take(max_line_bytes as u64 + 1)
            .read_until(b'\n', &mut line)?;
        if n == 0 {
            return Ok(());
        }
        if line.last() != Some(&b'\n') && n > max_line_bytes {
            skip_line(reader)?;
            sink.send(&Message::error(
                None,
                ErrorData::new(ErrorCode::INVALID_REQUEST, "line too long"),
            ));
            continue;
        }
        let text = line.trim_ascii();
        if text.is_empty() {
            continue;
        }
        match parse(text) {
            Err(reply) => sink.send(&reply),
            Ok(message) => match conn.start(message) {
                Started::Done(Some(reply)) => sink.send(&reply),
                Started::Done(None) => {}
                Started::Run(job) => spawn(job, sink),
            },
        }
    }
}

/// Run `job` on its own thread and write its reply.
fn spawn<W: Write + Send + 'static>(job: Job, sink: &Arc<LineSink<W>>) {
    let id = job.id().clone();
    let worker = Arc::clone(sink);
    let owed = sink.pending.enter();
    let started = std::thread::Builder::new().spawn(move || {
        if let Some(reply) = job.run() {
            worker.send(&reply);
        }
        drop(owed);
    });
    if started.is_err() {
        // The closure, and with it the job, was dropped, which freed its slot.
        sink.send(&Message::error(
            Some(id),
            ErrorData::new(ErrorCode::INTERNAL_ERROR, "could not start a worker thread"),
        ));
    }
}
