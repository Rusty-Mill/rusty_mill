//! The stdio transport: one JSON-RPC message per line on stdin, replies on
//! stdout, logs (yours) on stderr. Requests run concurrently, one thread
//! each, so a slow tool never blocks the next message, which is what lets a
//! cancellation reach it.

use crate::connection::{Connection, Notifier, Started};
use crate::server::Server;
use rusty_mcp_proto::{Error as ProtoError, ErrorCode, ErrorData, Message, Wire};
use std::io::{self, BufRead, BufReader, Read, Write};
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{Arc, Mutex, PoisonError};

/// The longest line accepted unless [`serve_lines`] is given another limit.
pub const DEFAULT_MAX_LINE_BYTES: usize = 4 * 1024 * 1024;

/// Writes one message per line; once a write fails it stops trying.
struct LineSink<W> {
    out: Mutex<W>,
    failed: AtomicBool,
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
/// finish the requests still running.
///
/// # Errors
/// An I/O error reading stdin, or [`io::ErrorKind::BrokenPipe`] when stdout
/// went away.
pub fn serve_stdio(server: Arc<Server>) -> io::Result<()> {
    serve_lines(
        server,
        BufReader::new(io::stdin()),
        io::stdout(),
        DEFAULT_MAX_LINE_BYTES,
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

/// [`serve_stdio`] over any reader and writer. A line longer than
/// `max_line_bytes` is answered with an error and skipped; the connection
/// carries on.
///
/// # Errors
/// As [`serve_stdio`].
pub fn serve_lines<R, W>(
    server: Arc<Server>,
    mut reader: R,
    writer: W,
    max_line_bytes: usize,
) -> io::Result<()>
where
    R: BufRead,
    W: Write + Send + 'static,
{
    let sink = Arc::new(LineSink {
        out: Mutex::new(writer),
        failed: AtomicBool::new(false),
    });
    let conn = Arc::new(Connection::new(server, sink.clone()));
    let mut line = Vec::new();
    // Leaving the scope waits for the workers, so end of input drains the
    // requests still running instead of dropping their answers.
    let served = std::thread::scope(|scope| -> io::Result<()> {
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
                skip_line(&mut reader)?;
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
                    Started::Run(job) => {
                        let sink = Arc::clone(&sink);
                        scope.spawn(move || {
                            if let Some(reply) = job.run() {
                                sink.send(&reply);
                            }
                        });
                    }
                },
            }
        }
    });
    // Workers have joined: a reply that could not be written is an error even
    // when the input ended first.
    served?;
    if sink.failed.load(Ordering::SeqCst) {
        return Err(io::ErrorKind::BrokenPipe.into());
    }
    Ok(())
}
