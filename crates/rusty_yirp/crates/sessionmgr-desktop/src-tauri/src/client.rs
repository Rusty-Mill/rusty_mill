//! The desktop app's own socket client: blocking `std` I/O, deliberately
//! not `rusty_tokio` -- see `Cargo.toml`'s own comment on why mixing
//! Tauri's real-`tokio` command runtime with a second, incompatible
//! async runtime is the wrong shape here. One-shot requests (list, new,
//! close, rename, fork, switch-agent, git status/diff) each open a
//! fresh connection, write one line, read one line, and close --
//! `attach.rs` is the one place that keeps a connection open.
//!
//! Framing matches `sessionmgr-protocol`'s own docs exactly (one JSON
//! value per `\n`-terminated line) and duplicates
//! `sessionmgr-tui::client::Connection` in spirit -- same reasoning as
//! `paths.rs`: this crate depends on `sessionmgr-protocol` only.

use std::io::{self, BufRead, BufReader, Write};
use std::path::Path;

use serde::de::DeserializeOwned;
use serde::Serialize;
use sessionmgr_protocol::{Request, Response};

use crate::unix_stream::UnixStream;

/// A generous ceiling on one framed line's length. Mirrors
/// `sessionmgr-daemon::transport::MAX_LINE_LEN` exactly, for the same
/// reason the rest of this module's framing does -- see that module's
/// own docs for why that duplication is the actual architectural
/// boundary, not an oversight.
///
/// This protocol's messages are small and infrequent, with
/// `SessionEvent::Output` (a base64-encoded PTY read, a handful of KB
/// once JSON-escaped) as the single high-volume exception -- this
/// leaves over 20x that much headroom. Without a cap, any local process
/// that connects to the daemon and streams bytes with no `\n` grows
/// this app's heap without bound until it is OOM-killed.
pub(crate) const MAX_LINE_LEN: usize = 256 * 1024;

/// [`BufRead::read_line`]'s own contract, except it refuses to grow
/// `buf` past `max_len` bytes instead of buffering forever waiting for
/// a `\n` that may never come. Mirrors
/// `sessionmgr-daemon::transport::read_line_capped` exactly, adapted
/// from tokio's `AsyncBufRead` to blocking `std::io::BufRead` -- see
/// this crate's own `Cargo.toml` comment on why this crate's socket
/// client is synchronous `std` I/O rather than `rusty_tokio`.
///
/// `Ok(n)` for `n > 0` where the appended bytes do not end in `\n` means
/// `max_len` was reached with no terminator found -- the caller's
/// contract (mirrored by both call sites, here and in `attach.rs`) is
/// to treat that as a protocol violation and close the connection, not
/// call this again expecting the rest of the line.
pub(crate) fn read_line_capped(
    reader: &mut impl BufRead,
    buf: &mut String,
    max_len: usize,
) -> io::Result<usize> {
    let mut bytes: Vec<u8> = Vec::new();
    let mut found_newline = false;
    while bytes.len() < max_len {
        let available = reader.fill_buf()?;
        if available.is_empty() {
            break; // Clean EOF.
        }
        let budget = max_len - bytes.len();
        let (used, done) = match available.iter().position(|&b| b == b'\n') {
            Some(pos) if pos < budget => (pos + 1, true),
            _ => (available.len().min(budget), false),
        };
        bytes.extend_from_slice(&available[..used]);
        reader.consume(used);
        if done {
            found_newline = true;
            break;
        }
    }
    if !found_newline && bytes.len() >= max_len {
        return Err(io::Error::new(
            io::ErrorKind::InvalidData,
            format!("line exceeds the {max_len}-byte limit with no `\\n` terminator"),
        ));
    }
    let n = bytes.len();
    let text = String::from_utf8(bytes).map_err(|_| {
        io::Error::new(
            io::ErrorKind::InvalidData,
            "stream did not contain valid UTF-8",
        )
    })?;
    buf.push_str(&text);
    Ok(n)
}

pub fn write_framed<T: Serialize>(stream: &mut UnixStream, value: &T) -> Result<(), String> {
    let mut encoded =
        serde_json::to_string(value).map_err(|e| format!("encoding a request: {e}"))?;
    encoded.push('\n');
    stream
        .write_all(encoded.as_bytes())
        .map_err(|e| format!("writing to the daemon: {e}"))?;
    stream
        .flush()
        .map_err(|e| format!("flushing the daemon socket: {e}"))
}

fn read_framed<T: DeserializeOwned>(reader: &mut impl BufRead) -> Result<Option<T>, String> {
    let mut line = String::new();
    let read = read_line_capped(reader, &mut line, MAX_LINE_LEN)
        .map_err(|e| format!("reading from the daemon: {e}"))?;
    if read == 0 {
        return Ok(None);
    }
    let trimmed = line.trim_end_matches(['\r', '\n']);
    if trimmed.is_empty() {
        return Ok(None);
    }
    serde_json::from_str(trimmed).map_err(|e| format!("decoding the daemon's answer: {e}"))
}

/// One request, one response, over a fresh connection.
pub fn request(socket: &Path, req: &Request) -> Result<Response, String> {
    let mut stream = UnixStream::connect(socket)
        .map_err(|e| format!("connecting to the daemon at {}: {e}", socket.display()))?;
    write_framed(&mut stream, req)?;
    let mut reader = BufReader::new(stream);
    read_framed(&mut reader)?
        .ok_or_else(|| "the daemon closed the connection without answering".to_owned())
}

/// Turns a `Response` into a typed result, erroring on `Response::Error`
/// and on any shape the caller did not ask for -- both are exactly as
/// much a bug as each other, so both go through this one place. Mirrors
/// `sessionmgr-tui::client::expect`.
pub fn expect<T>(
    response: Response,
    extract: impl FnOnce(Response) -> Option<T>,
) -> Result<T, String> {
    if let Response::Error { message, .. } = &response {
        return Err(message.clone());
    }
    extract(response).ok_or_else(|| "unexpected answer from the daemon".to_owned())
}

#[cfg(test)]
mod tests {
    use super::*;

    // `read_line_capped` is the one bounded-reader primitive shared by
    // both this module's `read_framed` and `attach.rs`'s reader thread
    // -- exercising it here covers both call sites, the same way
    // `sessionmgr-daemon::transport`'s own tests of its (async) twin do.

    #[test]
    fn an_oversized_unterminated_line_is_rejected_not_grown_without_bound() {
        // Both `client.rs::read_framed` and `attach.rs`'s reader thread
        // must not let a local peer that streams bytes with no `\n`
        // grow this app's heap without bound.
        let payload = vec![b'a'; MAX_LINE_LEN + 1024];
        let mut reader = std::io::Cursor::new(payload);
        let mut line = String::new();
        let err = read_line_capped(&mut reader, &mut line, MAX_LINE_LEN)
            .expect_err("a line past the cap with no terminator must be rejected");
        assert_eq!(err.kind(), io::ErrorKind::InvalidData);
        // Bounded: never grew past the configured cap even though the
        // peer sent far more than that.
        assert!(line.len() <= MAX_LINE_LEN);
    }

    #[test]
    fn a_line_within_the_cap_is_read_normally() {
        let mut reader = std::io::Cursor::new(b"{\"ok\":true}\n".to_vec());
        let mut line = String::new();
        let n = read_line_capped(&mut reader, &mut line, MAX_LINE_LEN)
            .expect("a small terminated line must succeed");
        assert_eq!(line, "{\"ok\":true}\n");
        assert_eq!(n, line.len());
    }
}
