//! Line-delimited JSON over `AF_UNIX`, on Windows as much as on Unix.
//!
//! `rusty_tokio::io::{UnixListener, UnixStream}` are `cfg(any(unix,
//! windows))` -- genuinely cross-platform, which is what makes one
//! transport rather than one-per-platform possible. Windows has had
//! `AF_UNIX` since 1803, and `rusty_prime_agent` already ships this
//! transport there.

use std::path::{Path, PathBuf};
use std::time::{Duration, Instant};

use rusty_tokio::io::{AsyncBufReadExt, AsyncWriteExt, BufReader, UnixStream};
use serde::de::DeserializeOwned;
use serde::Serialize;

use crate::error::{Error, Result};

/// A framed connection: one JSON value per `\n`-terminated line.
pub struct Connection {
    reader: BufReader<rusty_tokio::io::OwnedUnixReadHalf>,
    writer: rusty_tokio::io::OwnedUnixWriteHalf,
    line: String,
}

impl Connection {
    pub fn new(stream: UnixStream) -> Self {
        let (read, write) = stream.into_split();
        Connection {
            reader: BufReader::new(read),
            writer: write,
            line: String::new(),
        }
    }

    pub async fn connect(context: &'static str, path: &Path) -> Result<Self> {
        let stream = UnixStream::connect(path)
            .await
            .map_err(|e| Error::io(context, path.to_path_buf(), e))?;
        Ok(Connection::new(stream))
    }

    /// Reads one message. `Ok(None)` is a clean peer disconnect.
    /// Splits into independent read and write halves.
    ///
    /// Needed by the one genuinely bidirectional exchange in the
    /// protocol: an attached client concurrently receives output events
    /// and sends input. Everything else is strict request/response and
    /// uses [`Connection`] whole.
    pub fn into_parts(
        self,
    ) -> (
        BufReader<rusty_tokio::io::OwnedUnixReadHalf>,
        rusty_tokio::io::OwnedUnixWriteHalf,
    ) {
        (self.reader, self.writer)
    }

    pub async fn read<T: DeserializeOwned>(&mut self) -> Result<Option<T>> {
        self.line.clear();
        let read = self
            .reader
            .read_line(&mut self.line)
            .await
            .map_err(|e| Error::io("reading from a socket", None, e))?;
        if read == 0 {
            return Ok(None);
        }
        let trimmed = self.line.trim_end_matches(['\r', '\n']);
        if trimmed.is_empty() {
            return Ok(None);
        }
        let value = serde_json::from_str(trimmed).map_err(|e| {
            // Deliberately does not echo the offending line: it can carry
            // session output, and an error message is the last place that
            // should end up.
            Error::protocol(format!("malformed message ({e})"))
        })?;
        Ok(Some(value))
    }

    /// Writes one message and flushes, so a caller that then waits for a
    /// reply is not waiting on its own unflushed buffer.
    pub async fn write<T: Serialize>(&mut self, value: &T) -> Result<()> {
        let mut encoded = serde_json::to_string(value)?;
        encoded.push('\n');
        self.writer
            .write_all(encoded.as_bytes())
            .await
            .map_err(|e| Error::io("writing to a socket", None, e))?;
        self.writer
            .flush()
            .await
            .map_err(|e| Error::io("flushing a socket", None, e))
    }

    /// Sends a request and reads exactly one response.
    pub async fn request<Req: Serialize, Res: DeserializeOwned>(
        &mut self,
        request: &Req,
    ) -> Result<Res> {
        self.write(request).await?;
        self.read()
            .await?
            .ok_or_else(|| Error::protocol("peer closed the connection without answering"))
    }
}

/// Reads one framed message from a half-stream, for callers that used
/// [`Connection::into_parts`].
pub async fn read_framed<T: DeserializeOwned>(
    reader: &mut BufReader<rusty_tokio::io::OwnedUnixReadHalf>,
) -> Result<Option<T>> {
    let mut line = String::new();
    let read = reader
        .read_line(&mut line)
        .await
        .map_err(|e| Error::io("reading from a socket", None, e))?;
    if read == 0 {
        return Ok(None);
    }
    let trimmed = line.trim_end_matches(['\r', '\n']);
    if trimmed.is_empty() {
        return Ok(None);
    }
    serde_json::from_str(trimmed)
        .map(Some)
        .map_err(|e| Error::protocol(format!("malformed message ({e})")))
}

/// Writes one framed message to a half-stream, for callers that used
/// [`Connection::into_parts`].
pub async fn write_framed<T: Serialize>(
    writer: &mut rusty_tokio::io::OwnedUnixWriteHalf,
    value: &T,
) -> Result<()> {
    let mut encoded = serde_json::to_string(value)?;
    encoded.push('\n');
    writer
        .write_all(encoded.as_bytes())
        .await
        .map_err(|e| Error::io("writing to a socket", None, e))?;
    writer
        .flush()
        .await
        .map_err(|e| Error::io("flushing a socket", None, e))
}

/// Waits until `path` is a socket that answers, or `timeout` elapses.
///
/// Answers, not merely accepts. A listener can be bound and accepting
/// while the process behind it is still starting up, so a connect-only
/// probe reports ready too early -- the caller then sends a real request
/// into a void. `probe` therefore completes a full `Ping`/`Pong`
/// round-trip, which is the only thing that actually proves the peer is
/// serving.
pub async fn wait_ready<Req, Res>(
    path: &Path,
    ping: Req,
    is_pong: impl Fn(&Res) -> bool,
    timeout: Duration,
) -> Result<()>
where
    Req: Serialize + Clone,
    Res: DeserializeOwned,
{
    let deadline = Instant::now() + timeout;
    let mut last: Option<Error> = None;
    while Instant::now() < deadline {
        match probe(path, ping.clone(), &is_pong).await {
            Ok(true) => return Ok(()),
            Ok(false) => {
                last = Some(Error::protocol("peer answered a ping with something else"));
            }
            Err(e) => last = Some(e),
        }
        rusty_tokio::time::sleep(Duration::from_millis(25)).await;
    }
    Err(last.unwrap_or_else(|| {
        Error::io(
            "waiting for a socket to become ready",
            path.to_path_buf(),
            std::io::Error::new(std::io::ErrorKind::TimedOut, "timed out"),
        )
    }))
}

async fn probe<Req, Res>(path: &Path, ping: Req, is_pong: &impl Fn(&Res) -> bool) -> Result<bool>
where
    Req: Serialize,
    Res: DeserializeOwned,
{
    let mut conn = Connection::connect("probing a socket", path).await?;
    let response: Res = conn.request(&ping).await?;
    Ok(is_pong(&response))
}

/// A listener plus the path it is bound to, so shutdown can remove the
/// socket file it created.
pub struct Listener {
    inner: rusty_tokio::io::UnixListener,
    path: PathBuf,
}

impl Listener {
    /// Binds `path`, clearing any stale socket file left behind by a
    /// process that exited uncleanly.
    ///
    /// Clearing unconditionally is safe here because both callers hold a
    /// stronger claim first: the daemon checks for a live daemon via
    /// `daemon.json` before binding, and a worker owns a path derived
    /// from its own unique session id.
    pub fn bind(context: &'static str, path: &Path) -> Result<Self> {
        if let Some(parent) = path.parent() {
            crate::paths::ensure_dir(context, parent)?;
        }
        crate::paths::warn_if_socket_path_is_long(path);
        crate::paths::clear_socket(path);
        let inner = match rusty_tokio::io::UnixListener::bind(path) {
            Ok(listener) => listener,
            // One retry, because the interesting failure here is a race
            // rather than a permanent condition: a socket file left by a
            // process that was killed moments ago can briefly refuse to
            // be deleted or rebound while the OS finishes releasing it.
            // A tool built around surviving unclean exits meets exactly
            // that case on every restart after a crash, so failing on the
            // first attempt would make the common path the fragile one.
            Err(first) => {
                std::thread::sleep(std::time::Duration::from_millis(250));
                crate::paths::clear_socket(path);
                rusty_tokio::io::UnixListener::bind(path).map_err(|second| {
                    Error::io(
                        context,
                        path.to_path_buf(),
                        std::io::Error::new(
                            second.kind(),
                            format!(
                                "{second} (first attempt: {first}). If no sessionmgr daemon is \
                                 running, delete this file and retry."
                            ),
                        ),
                    )
                })?
            }
        };
        Ok(Listener {
            inner,
            path: path.to_path_buf(),
        })
    }

    pub async fn accept(&self) -> Result<Connection> {
        let (stream, _addr) = self
            .inner
            .accept()
            .await
            .map_err(|e| Error::io("accepting a connection", self.path.clone(), e))?;
        Ok(Connection::new(stream))
    }

    pub fn path(&self) -> &Path {
        &self.path
    }
}

impl Drop for Listener {
    fn drop(&mut self) {
        // Best-effort: a socket file left behind is a nuisance the next
        // `bind` clears anyway, and this runs on shutdown paths where a
        // failure has nowhere useful to go.
        let _ = std::fs::remove_file(&self.path);
    }
}
