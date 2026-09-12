//! The TUI's own socket client.
//!
//! Deliberately not a reuse of `sessionmgr-daemon`'s `transport.rs`/
//! `client.rs`: this crate depends on `sessionmgr-protocol` only, never
//! `sessionmgr-daemon` (which would be circular anyway, since the daemon
//! binary depends on this crate to serve the `tui` subcommand). The
//! framing this replicates is the one `sessionmgr-protocol`'s own module
//! docs specify: one JSON value per `\n`-terminated line.

use std::path::Path;
use std::pin::Pin;

use rusty_tokio::io::{AsyncBufRead, AsyncBufReadExt, AsyncWriteExt, BufReader, UnixStream};
use rusty_tokio::sync::mpsc::UnboundedSender;
use serde::de::DeserializeOwned;
use serde::Serialize;
use sessionmgr_protocol::{AgentKind, Request, Response, SessionEvent, SessionId};

use crate::error::{Error, Result};

/// A generous ceiling on one framed line's length. Mirrors
/// `sessionmgr-daemon::transport::MAX_LINE_LEN` exactly, for the same
/// reason the rest of this module's framing does -- see the module docs
/// for why that duplication is the actual architectural boundary, not
/// an oversight.
///
/// This protocol's messages are small and infrequent, with
/// `SessionEvent::Output` (a base64-encoded PTY read, a handful of KB
/// once JSON-escaped) as the single high-volume exception -- this
/// leaves over 20x that much headroom. Without a cap, any local process
/// that connects to the daemon and streams bytes with no `\n` grows
/// this TUI's heap without bound until it is OOM-killed.
const MAX_LINE_LEN: usize = 256 * 1024;

/// [`AsyncBufReadExt::read_line`]'s own contract, except it refuses to
/// grow `buf` past `max_len` bytes instead of buffering forever waiting
/// for a `\n` that may never come.
///
/// `Ok(n)` for `n > 0` where the appended bytes do not end in `\n` means
/// `max_len` was reached with no terminator found -- the caller's
/// contract (mirrored by both call sites below) is to treat that as a
/// protocol violation and close the connection, not call this again
/// expecting the rest of the line.
async fn read_line_capped<R>(
    reader: &mut R,
    buf: &mut String,
    max_len: usize,
) -> std::io::Result<usize>
where
    R: AsyncBufRead + Unpin,
{
    let mut bytes: Vec<u8> = Vec::new();
    let mut found_newline = false;
    while bytes.len() < max_len {
        let available = reader.fill_buf().await?;
        if available.is_empty() {
            break; // Clean EOF.
        }
        let budget = max_len - bytes.len();
        let (used, done) = match available.iter().position(|&b| b == b'\n') {
            Some(pos) if pos < budget => (pos + 1, true),
            _ => (available.len().min(budget), false),
        };
        bytes.extend_from_slice(&available[..used]);
        Pin::new(&mut *reader).consume(used);
        if done {
            found_newline = true;
            break;
        }
    }
    if !found_newline && bytes.len() >= max_len {
        return Err(std::io::Error::new(
            std::io::ErrorKind::InvalidData,
            format!("line exceeds the {max_len}-byte limit with no `\\n` terminator"),
        ));
    }
    let n = bytes.len();
    let text = String::from_utf8(bytes).map_err(|_| {
        std::io::Error::new(
            std::io::ErrorKind::InvalidData,
            "stream did not contain valid UTF-8",
        )
    })?;
    buf.push_str(&text);
    Ok(n)
}

/// A framed connection: one JSON value per `\n`-terminated line. Mirrors
/// `sessionmgr-daemon::transport::Connection` exactly (same wire format),
/// but is its own small type rather than a shared dependency -- see the
/// module docs for why that duplication is the actual architectural
/// boundary, not an oversight.
pub struct Connection {
    reader: BufReader<rusty_tokio::io::OwnedUnixReadHalf>,
    writer: rusty_tokio::io::OwnedUnixWriteHalf,
    line: String,
}

impl Connection {
    pub async fn connect(path: &Path) -> Result<Self> {
        let stream = UnixStream::connect(path)
            .await
            .map_err(|e| Error::io("connecting to the daemon", e))?;
        let (read, write) = stream.into_split();
        Ok(Connection {
            reader: BufReader::new(read),
            writer: write,
            line: String::new(),
        })
    }

    pub async fn write<T: Serialize>(&mut self, value: &T) -> Result<()> {
        let mut encoded = serde_json::to_string(value)?;
        encoded.push('\n');
        self.writer
            .write_all(encoded.as_bytes())
            .await
            .map_err(|e| Error::io("writing to the daemon", e))?;
        self.writer
            .flush()
            .await
            .map_err(|e| Error::io("flushing the daemon socket", e))
    }

    pub async fn read<T: DeserializeOwned>(&mut self) -> Result<Option<T>> {
        self.line.clear();
        let read = read_line_capped(&mut self.reader, &mut self.line, MAX_LINE_LEN)
            .await
            .map_err(|e| match e.kind() {
                std::io::ErrorKind::InvalidData => Error::protocol(e.to_string()),
                _ => Error::io("reading from the daemon", e),
            })?;
        if read == 0 {
            return Ok(None);
        }
        let trimmed = self.line.trim_end_matches(['\r', '\n']);
        if trimmed.is_empty() {
            return Ok(None);
        }
        Ok(Some(serde_json::from_str(trimmed)?))
    }

    /// Sends a request and reads exactly one response.
    pub async fn request(&mut self, request: &Request) -> Result<Response> {
        self.write(request).await?;
        self.read()
            .await?
            .ok_or_else(|| Error::protocol("the daemon closed the connection without answering"))
    }

    pub fn into_parts(
        self,
    ) -> (
        BufReader<rusty_tokio::io::OwnedUnixReadHalf>,
        rusty_tokio::io::OwnedUnixWriteHalf,
    ) {
        (self.reader, self.writer)
    }
}

/// Turns a `Response` into a typed `Result`, for callers that expect one
/// specific success shape and treat everything else as an error.
///
/// `Response::Ok`/`Response::SessionCreated`/etc. arriving where a
/// caller expected `Response::Sessions`, say, is exactly as much a bug as
/// `Response::Error` -- both go through this one place rather than each
/// call site inventing its own "well, that's not what I expected" arm.
fn expect<T>(response: Response, extract: impl FnOnce(Response) -> Option<T>) -> Result<T> {
    if let Response::Error { message, .. } = &response {
        return Err(Error::Daemon {
            message: message.clone(),
        });
    }
    extract(response).ok_or_else(|| Error::protocol("unexpected answer from the daemon"))
}

pub async fn session_list(socket: &Path) -> Result<Vec<sessionmgr_protocol::SessionSummary>> {
    let mut conn = Connection::connect(socket).await?;
    let response = conn.request(&Request::SessionList).await?;
    expect(response, |r| match r {
        Response::Sessions { sessions } => Some(sessions),
        _ => None,
    })
}

pub async fn git_status(
    socket: &Path,
    id: SessionId,
) -> Result<Vec<sessionmgr_protocol::ChangedFile>> {
    let mut conn = Connection::connect(socket).await?;
    let response = conn.request(&Request::GitStatus { id }).await?;
    expect(response, |r| match r {
        Response::GitStatus { files } => Some(files),
        _ => None,
    })
}

pub async fn git_diff(socket: &Path, id: SessionId, path: Option<String>) -> Result<String> {
    let mut conn = Connection::connect(socket).await?;
    let response = conn.request(&Request::GitDiff { id, path }).await?;
    expect(response, |r| match r {
        Response::GitDiff { diff } => Some(diff),
        _ => None,
    })
}

pub async fn session_close(
    socket: &Path,
    id: SessionId,
    disposition: Option<sessionmgr_protocol::Disposition>,
) -> Result<()> {
    let mut conn = Connection::connect(socket).await?;
    let response = conn
        .request(&Request::SessionClose { id, disposition })
        .await?;
    expect(response, |r| matches!(r, Response::Ok).then_some(()))
}

/// Creates a plain worktree session against `repo`, with no agent and
/// this platform's default shell -- the command palette's `new session`
/// action is a fast, keyboard-only shortcut for the single most common
/// case, not a replacement for `sessionmgr new`'s full flag surface.
pub async fn session_new(socket: &Path, repo: std::path::PathBuf) -> Result<SessionId> {
    let mut conn = Connection::connect(socket).await?;
    let response = conn
        .request(&Request::SessionNew {
            kind: sessionmgr_protocol::SessionKind::Worktree,
            command: Vec::new(),
            repo: Some(repo),
            pty: true,
            agent: None,
            hooks: false,
            parent: None,
            wait_for_parent: false,
        })
        .await?;
    expect(response, |r| match r {
        Response::SessionCreated { id } => Some(id),
        _ => None,
    })
}

/// Sets (or, given `None`, clears) a session's display label.
pub async fn session_rename(socket: &Path, id: SessionId, name: Option<String>) -> Result<()> {
    let mut conn = Connection::connect(socket).await?;
    let response = conn.request(&Request::SessionRename { id, name }).await?;
    expect(response, |r| matches!(r, Response::Ok).then_some(()))
}

/// CAPABILITIES.md's "Fork session", always requesting a PTY-backed new
/// session -- the palette action is a fast keyboard shortcut for the
/// common interactive case, same reasoning as `session_new`'s own
/// defaults, not a replacement for `sessionmgr fork <id> --no-pty`.
pub async fn session_fork(socket: &Path, id: SessionId) -> Result<SessionId> {
    let mut conn = Connection::connect(socket).await?;
    let response = conn
        .request(&Request::SessionFork { id, pty: true })
        .await?;
    expect(response, |r| match r {
        Response::SessionCreated { id } => Some(id),
        _ => None,
    })
}

/// CAPABILITIES.md's "Switch agent mid-session", always requesting a
/// PTY-backed new session -- same reasoning as [`session_fork`].
pub async fn session_switch_agent(
    socket: &Path,
    id: SessionId,
    agent: AgentKind,
) -> Result<SessionId> {
    let mut conn = Connection::connect(socket).await?;
    let response = conn
        .request(&Request::SessionSwitchAgent {
            id,
            agent,
            pty: true,
        })
        .await?;
    expect(response, |r| match r {
        Response::SessionCreated { id } => Some(id),
        _ => None,
    })
}

/// A live attach: the write half stays open for `SessionInput`/
/// `SessionResize`, and a background task pumps `SessionEvent`s from the
/// read half into `events` until the connection closes.
///
/// One socket does both directions -- see
/// `sessionmgr-daemon::supervisor::proxy_attach`'s own doc comment: after
/// the initial `SessionAttach`, the daemon forwards any further request
/// on the same connection straight to the worker, and streams the
/// worker's events straight back.
pub struct Attached {
    id: SessionId,
    writer: rusty_tokio::io::OwnedUnixWriteHalf,
}

impl Attached {
    /// Opens the attach connection and returns it alongside the
    /// `JoinHandle` of the task pumping `SessionEvent`s into `events`.
    /// The handle is the caller's only way to stop that task: it holds
    /// the read half itself, not reachable through `Attached`, so
    /// dropping `Attached` alone (which only holds the write half) would
    /// otherwise leak it running forever.
    pub async fn open(
        path: &Path,
        id: SessionId,
        events: UnboundedSender<(SessionId, SessionEvent)>,
    ) -> Result<(Self, rusty_tokio::task::JoinHandle<()>)> {
        let mut conn = Connection::connect(path).await?;
        conn.write(&Request::SessionAttach { id: id.clone() })
            .await?;
        let (mut reader, writer) = conn.into_parts();
        let pump_id = id.clone();
        let pump = rusty_tokio::spawn(async move {
            loop {
                let event: Option<SessionEvent> = match read_framed(&mut reader).await {
                    Ok(event) => event,
                    Err(_) => break,
                };
                let Some(event) = event else { break };
                if events.send((pump_id.clone(), event)).is_err() {
                    break;
                }
            }
        });
        Ok((Attached { id, writer }, pump))
    }

    pub async fn send_input(&mut self, data: Vec<u8>) -> Result<()> {
        write_framed(
            &mut self.writer,
            &Request::SessionInput {
                id: self.id.clone(),
                data,
            },
        )
        .await
    }

    pub async fn send_resize(&mut self, rows: u16, cols: u16) -> Result<()> {
        write_framed(
            &mut self.writer,
            &Request::SessionResize {
                id: self.id.clone(),
                rows,
                cols,
            },
        )
        .await
    }
}

async fn read_framed<T: DeserializeOwned>(
    reader: &mut BufReader<rusty_tokio::io::OwnedUnixReadHalf>,
) -> Result<Option<T>> {
    let mut line = String::new();
    let read = read_line_capped(reader, &mut line, MAX_LINE_LEN)
        .await
        .map_err(|e| match e.kind() {
            std::io::ErrorKind::InvalidData => Error::protocol(e.to_string()),
            _ => Error::io("reading from the daemon", e),
        })?;
    if read == 0 {
        return Ok(None);
    }
    let trimmed = line.trim_end_matches(['\r', '\n']);
    if trimmed.is_empty() {
        return Ok(None);
    }
    Ok(Some(serde_json::from_str(trimmed)?))
}

async fn write_framed<T: Serialize>(
    writer: &mut rusty_tokio::io::OwnedUnixWriteHalf,
    value: &T,
) -> Result<()> {
    let mut encoded = serde_json::to_string(value)?;
    encoded.push('\n');
    writer
        .write_all(encoded.as_bytes())
        .await
        .map_err(|e| Error::io("writing to the daemon", e))?;
    writer
        .flush()
        .await
        .map_err(|e| Error::io("flushing the daemon socket", e))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn an_oversized_unterminated_line_is_rejected_not_grown_without_bound() {
        // Finding 25's trigger: the daemon (or anything else on the
        // other end of this socket) streaming bytes with no `\n` must
        // not be able to grow the TUI's heap without bound.
        let rt = rusty_tokio::Runtime::new().expect("runtime");
        rt.block_on(async {
            let (mut tx, rx) = rusty_tokio::io::simplex(MAX_LINE_LEN + 4096);
            let payload = vec![b'a'; MAX_LINE_LEN + 1024];
            tx.write_all(&payload)
                .await
                .expect("write the oversized, unterminated payload");
            drop(tx); // EOF once the reader drains what is buffered.

            let mut reader = BufReader::new(rx);
            let mut line = String::new();
            let err = read_line_capped(&mut reader, &mut line, MAX_LINE_LEN)
                .await
                .expect_err("a line past the cap with no terminator must be rejected");
            assert_eq!(err.kind(), std::io::ErrorKind::InvalidData);
            // Bounded: never grew past the configured cap even though
            // the peer sent far more than that.
            assert!(line.len() <= MAX_LINE_LEN);
        });
    }

    #[test]
    fn a_line_within_the_cap_is_read_normally() {
        let rt = rusty_tokio::Runtime::new().expect("runtime");
        rt.block_on(async {
            let (mut tx, rx) = rusty_tokio::io::simplex(4096);
            tx.write_all(b"{\"ok\":true}\n")
                .await
                .expect("write a small terminated line");
            drop(tx);

            let mut reader = BufReader::new(rx);
            let mut line = String::new();
            let n = read_line_capped(&mut reader, &mut line, MAX_LINE_LEN)
                .await
                .expect("a small terminated line must succeed");
            assert_eq!(line, "{\"ok\":true}\n");
            assert_eq!(n, line.len());
        });
    }
}
