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

use std::io::{BufRead, BufReader, Write};
use std::path::Path;

use serde::de::DeserializeOwned;
use serde::Serialize;
use sessionmgr_protocol::{Request, Response};

use crate::unix_stream::UnixStream;

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
    let read = reader
        .read_line(&mut line)
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
