//! Line-delimited JSON over `AF_UNIX`, on Windows as much as on Unix.
//!
//! `rusty_tokio::io::{UnixListener, UnixStream}` are `cfg(any(unix,
//! windows))` -- genuinely cross-platform, which is what makes one
//! transport rather than one-per-platform possible. Windows has had
//! `AF_UNIX` since 1803, and `rusty_prime_agent` already ships this
//! transport there.

use std::path::{Path, PathBuf};
use std::pin::Pin;
use std::time::{Duration, Instant};

use rusty_tokio::io::{AsyncBufRead, AsyncBufReadExt, AsyncWriteExt, BufReader, UnixStream};
use serde::de::DeserializeOwned;
use serde::Serialize;

use crate::error::{Error, Result};

/// A generous ceiling on one framed line's length.
///
/// This module's own doc comment plus `sessionmgr-protocol`'s docs both
/// describe this protocol's messages as small and infrequent, with
/// `SessionEvent::Output` (a base64-encoded, `worker.rs`-sized PTY read,
/// a handful of KB once JSON-escaped) as the single high-volume
/// exception -- this leaves over 20x that much headroom. Without a cap,
/// any local process that connects to the daemon's public socket or a
/// worker's private socket and streams bytes with no `\n` grows this
/// process's heap without bound until it is OOM-killed.
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
        let read = read_line_capped(&mut self.reader, &mut self.line, MAX_LINE_LEN)
            .await
            .map_err(|e| match e.kind() {
                std::io::ErrorKind::InvalidData => Error::protocol(e.to_string()),
                _ => Error::io("reading from a socket", None, e),
            })?;
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
    let read = read_line_capped(reader, &mut line, MAX_LINE_LEN)
        .await
        .map_err(|e| match e.kind() {
            std::io::ErrorKind::InvalidData => Error::protocol(e.to_string()),
            _ => Error::io("reading from a socket", None, e),
        })?;
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

/// How long one readiness probe may take before it is abandoned and
/// retried.
///
/// **Load-bearing, not a tidy-up.** Connecting to a bound socket succeeds
/// as soon as the listener exists, whether or not anything is accepting
/// yet -- the connection simply sits in the listen backlog. A probe that
/// then waits for a reply with no timeout blocks forever, which makes the
/// caller's own deadline meaningless because it is only checked between
/// probes. That is not hypothetical: it hung `supervisor_restart_recovery`
/// indefinitely on Windows, where a client connected to a daemon that had
/// bound its socket but had not yet started accepting.
const PROBE_TIMEOUT: Duration = Duration::from_secs(2);

async fn probe<Req, Res>(path: &Path, ping: Req, is_pong: &impl Fn(&Res) -> bool) -> Result<bool>
where
    Req: Serialize,
    Res: DeserializeOwned,
{
    let attempt = async {
        let mut conn = Connection::connect("probing a socket", path).await?;
        let response: Res = conn.request(&ping).await?;
        Ok::<_, Error>(is_pong(&response))
    };
    match rusty_tokio::time::timeout(PROBE_TIMEOUT, attempt).await {
        Ok(result) => result,
        Err(_) => Err(Error::io(
            "probing a socket",
            path.to_path_buf(),
            std::io::Error::new(
                std::io::ErrorKind::TimedOut,
                "the peer accepted a connection but did not answer",
            ),
        )),
    }
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

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn an_oversized_unterminated_line_is_rejected_not_grown_without_bound() {
        // Finding 25's trigger: any local peer that connects and streams
        // bytes with no `\n` must not be able to grow this process's
        // heap without bound.
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
