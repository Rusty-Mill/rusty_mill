//! The stdio transport: one JSON-RPC message per line on stdin, replies on
//! stdout, logs (yours) on stderr. Requests run concurrently, one thread
//! each, so a slow tool never blocks the next message, which is what lets a
//! cancellation reach it.

use crate::connection::{Connection, Job, Notifier, Started};
use crate::server::Server;
use rusty_mcp_proto::{Error as ProtoError, ErrorCode, ErrorData, Message, Wire};
use std::io::{self, BufRead, BufReader, Read, Write};
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::mpsc::{sync_channel, Receiver, SyncSender};
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

/// What the reader thread tells the serving loop.
enum Input {
    Line(Vec<u8>),
    TooLong,
    End,
    Failed(io::Error),
    /// A write failed: look at [`LineSink::failed`].
    Wake,
}

/// Writes one message per line; once a write fails it stops trying.
struct LineSink<W> {
    out: Mutex<W>,
    failed: AtomicBool,
    /// Wakes the serving loop out of its wait for input when a write fails,
    /// since a blocking read on stdin cannot be interrupted.
    wake: SyncSender<Input>,
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
            // A full channel means the loop has input to wake for anyway.
            let _ = self.wake.try_send(Input::Wake);
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
    reader: R,
    writer: W,
    config: StdioConfig,
) -> io::Result<()>
where
    R: BufRead + Send + 'static,
    W: Write + Send + 'static,
{
    let (tx, rx) = sync_channel(1);
    let sink = Arc::new(LineSink {
        out: Mutex::new(writer),
        failed: AtomicBool::new(false),
        wake: tx.clone(),
        pending: Arc::new(Pending::default()),
    });
    let conn = Arc::new(Connection::new(server, sink.clone()));
    // Reading happens off this thread so a failed write can end the session
    // while stdin stays open and silent; that thread is then abandoned and
    // ends with its next line or with the process.
    std::thread::Builder::new()
        .name("mcp-stdin".into())
        .spawn(move || read_input(reader, config.max_line_bytes, &tx))?;
    let read = read_loop(&conn, &sink, &rx);
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

/// Read lines until end of input or an error, handing each to `tx`.
fn read_input<R: BufRead>(mut reader: R, max_line_bytes: usize, tx: &SyncSender<Input>) {
    loop {
        let mut line = Vec::new();
        let read = reader
            .by_ref()
            .take(max_line_bytes as u64 + 1)
            .read_until(b'\n', &mut line);
        let input = match read {
            Err(e) => Input::Failed(e),
            Ok(0) => Input::End,
            Ok(n) if line.last() != Some(&b'\n') && n > max_line_bytes => {
                match skip_line(&mut reader) {
                    Ok(()) => Input::TooLong,
                    Err(e) => Input::Failed(e),
                }
            }
            Ok(_) => Input::Line(line),
        };
        let last = matches!(input, Input::End | Input::Failed(_));
        if tx.send(input).is_err() || last {
            return;
        }
    }
}

fn read_loop<W: Write + Send + 'static>(
    conn: &Arc<Connection>,
    sink: &Arc<LineSink<W>>,
    rx: &Receiver<Input>,
) -> io::Result<()> {
    loop {
        if sink.failed.load(Ordering::SeqCst) {
            return Err(io::ErrorKind::BrokenPipe.into());
        }
        let line = match rx.recv() {
            Ok(Input::Line(line)) => line,
            Ok(Input::Wake) => continue,
            Ok(Input::TooLong) => {
                sink.send(&Message::error(
                    None,
                    ErrorData::new(ErrorCode::INVALID_REQUEST, "line too long"),
                ));
                continue;
            }
            Ok(Input::Failed(e)) => return Err(e),
            Ok(Input::End) | Err(_) => return Ok(()),
        };
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
