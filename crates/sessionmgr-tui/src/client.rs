//! The TUI's own socket client.
//!
//! Deliberately not a reuse of `sessionmgr-daemon`'s `transport.rs`/
//! `client.rs`: this crate depends on `sessionmgr-protocol` only, never
//! `sessionmgr-daemon` (which would be circular anyway, since the daemon
//! binary depends on this crate to serve the `tui` subcommand). The
//! framing this replicates is the one `sessionmgr-protocol`'s own module
//! docs specify: one JSON value per `\n`-terminated line.

use std::path::Path;

use rusty_tokio::io::{AsyncBufReadExt, AsyncWriteExt, BufReader, UnixStream};
use rusty_tokio::sync::mpsc::UnboundedSender;
use serde::de::DeserializeOwned;
use serde::Serialize;
use sessionmgr_protocol::{Request, Response, SessionEvent, SessionId};

use crate::error::{Error, Result};

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
        let read = self
            .reader
            .read_line(&mut self.line)
            .await
            .map_err(|e| Error::io("reading from the daemon", e))?;
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
    let read = reader
        .read_line(&mut line)
        .await
        .map_err(|e| Error::io("reading from the daemon", e))?;
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
