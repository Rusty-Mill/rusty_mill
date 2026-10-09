//! Newline-delimited JSON over a reader and a writer, as MCP's stdio
//! transport specifies.
//!
//! One thread reads and decodes lines; the calling thread runs handlers one
//! request at a time. Because the reader keeps running while a handler does,
//! `notifications/cancelled` reaches the handler's [`Context`] in time for it
//! to stop. A response to a cancelled request is not sent.

use crate::dispatch::parse_failure;
use crate::{Context, Dispatcher, Service};
use rusty_mcp_proto::{Message, RequestId};
use std::collections::HashMap;
use std::io::{self, BufRead, Write};
use std::sync::mpsc;
use std::sync::{Arc, Mutex};
use std::thread;

/// Limits for the stdio transport.
#[derive(Clone, Copy, Debug)]
pub struct Options {
    /// The longest accepted line, in bytes. A longer line is answered with an
    /// error and skipped, so one hostile peer cannot make the server buffer
    /// without bound.
    pub max_line_bytes: usize,
}

impl Default for Options {
    fn default() -> Self {
        Options {
            max_line_bytes: 4 * 1024 * 1024,
        }
    }
}

type InFlight = Arc<Mutex<HashMap<RequestId, Context>>>;

enum Line {
    Eof,
    TooLong,
    Text(String),
}

/// Serve on the process's standard input and output until stdin closes.
pub fn serve_stdio<S: Service + 'static>(
    dispatcher: Dispatcher<S>,
    options: Options,
) -> io::Result<()> {
    serve(
        dispatcher,
        io::BufReader::new(io::stdin()),
        io::stdout(),
        options,
    )
}

/// Serve on any reader and writer until the reader reaches end of input.
pub fn serve<S, R, W>(
    dispatcher: Dispatcher<S>,
    input: R,
    output: W,
    options: Options,
) -> io::Result<()>
where
    S: Service + 'static,
    R: BufRead + Send + 'static,
    W: Write + Send + 'static,
{
    let output = Arc::new(Mutex::new(output));
    let in_flight: InFlight = Arc::new(Mutex::new(HashMap::new()));
    let (work_tx, work_rx) = mpsc::channel::<(Message, Context)>();

    let reader = {
        let output = Arc::clone(&output);
        let in_flight = Arc::clone(&in_flight);
        thread::spawn(move || read_loop(input, &output, &in_flight, &work_tx, options))
    };

    let mut result = Ok(());
    for (message, ctx) in work_rx {
        let id = match &message {
            Message::Request { id, .. } => Some(id.clone()),
            _ => None,
        };
        let response = dispatcher.handle(&ctx, message);
        if let Some(id) = &id {
            lock(&in_flight).remove(id);
        }
        if let (Some(response), false) = (response, ctx.is_cancelled()) {
            if let Err(error) = write_message(&output, &response) {
                result = Err(error);
                break;
            }
        }
    }
    // Dropping `work_rx` (end of the loop) makes a blocked reader's next send
    // fail, so it exits; a reader blocked in `read` ends at end of input.
    let reader_result = reader
        .join()
        .unwrap_or_else(|_| Err(io::Error::other("reader thread panicked")));
    result.and(reader_result)
}

fn read_loop<R: BufRead, W: Write>(
    mut input: R,
    output: &Arc<Mutex<W>>,
    in_flight: &InFlight,
    work: &mpsc::Sender<(Message, Context)>,
    options: Options,
) -> io::Result<()> {
    loop {
        let text = match read_line(&mut input, options.max_line_bytes)? {
            Line::Eof => return Ok(()),
            Line::TooLong => {
                let error = rusty_mcp_proto::Error::Decode {
                    what: "message",
                    reason: format!("line longer than {} bytes", options.max_line_bytes),
                };
                write_message(output, &parse_failure(&error, None))?;
                continue;
            }
            Line::Text(text) if text.trim().is_empty() => continue,
            Line::Text(text) => text,
        };
        match Message::from_json(&text) {
            Err(error) => write_message(output, &parse_failure(&error, None))?,
            Ok(Message::Notification { method, params }) if method == "notifications/cancelled" => {
                let target = params
                    .as_ref()
                    .and_then(|p| p.get("requestId"))
                    .and_then(|v| RequestId::from_value(v).ok());
                if let Some(ctx) = target.and_then(|id| lock(in_flight).get(&id).cloned()) {
                    ctx.cancel();
                }
            }
            Ok(message) => {
                let ctx = Context::new();
                if let Message::Request { id, .. } = &message {
                    lock(in_flight).insert(id.clone(), ctx.clone());
                }
                if work.send((message, ctx)).is_err() {
                    return Ok(());
                }
            }
        }
    }
}

/// Read one line of at most `max` bytes, without the newline. A longer line
/// is consumed to its end and reported as [`Line::TooLong`].
fn read_line<R: BufRead>(input: &mut R, max: usize) -> io::Result<Line> {
    let mut line = Vec::new();
    let mut too_long = false;
    loop {
        let (consumed, done) = {
            let chunk = input.fill_buf()?;
            if chunk.is_empty() {
                if line.is_empty() && !too_long {
                    return Ok(Line::Eof);
                }
                (0, true)
            } else {
                match chunk.iter().position(|b| *b == b'\n') {
                    Some(end) => {
                        if !too_long && line.len() + end <= max {
                            line.extend_from_slice(&chunk[..end]);
                        } else {
                            too_long = true;
                        }
                        (end + 1, true)
                    }
                    None => {
                        if !too_long && line.len() + chunk.len() <= max {
                            line.extend_from_slice(chunk);
                        } else {
                            too_long = true;
                            line.clear();
                        }
                        (chunk.len(), false)
                    }
                }
            }
        };
        input.consume(consumed);
        if done {
            break;
        }
    }
    if too_long {
        return Ok(Line::TooLong);
    }
    Ok(Line::Text(
        String::from_utf8_lossy(&line)
            .trim_end_matches('\r')
            .to_string(),
    ))
}

fn write_message<W: Write>(output: &Arc<Mutex<W>>, message: &Message) -> io::Result<()> {
    let mut text = message.to_json();
    text.push('\n');
    let mut out = lock(output);
    out.write_all(text.as_bytes())?;
    out.flush()
}

fn lock<T>(mutex: &Mutex<T>) -> std::sync::MutexGuard<'_, T> {
    // A poisoned lock only means another thread panicked; the data is still
    // usable for a map of cancel flags and a byte sink.
    mutex
        .lock()
        .unwrap_or_else(|poisoned| poisoned.into_inner())
}
