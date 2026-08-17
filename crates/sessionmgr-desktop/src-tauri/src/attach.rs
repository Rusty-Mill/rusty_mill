//! Live-attach connections: one per open pane, kept open for as long as
//! the pane is, unlike every other request in `client.rs`.
//!
//! Mirrors `sessionmgr-tui::client::Attached` -- write the initial
//! `SessionAttach`, then hand the read half to a dedicated blocking
//! thread that decodes `SessionEvent`s and forwards each one to the
//! frontend as a Tauri event, while the write half stays available for
//! `SessionInput`/`SessionResize` on the same connection (the daemon's
//! `proxy_attach` forwards any further request on an attached connection
//! straight to the worker -- see `supervisor.rs`'s own doc comment).

use std::io::BufReader;
use std::net::Shutdown;
use std::os::unix::net::UnixStream;
use std::path::Path;

use sessionmgr_protocol::{Request, SessionEvent, SessionId};
use tauri::{AppHandle, Emitter};

use crate::client::write_framed;

pub struct AttachHandle {
    writer: UnixStream,
}

/// Payload for the `"session-event"` event emitted to the frontend --
/// `SessionEvent` carries no id of its own (the attach connection it
/// arrived on implies it), so this pairs the two back up for a frontend
/// listening on one shared event name across every open pane.
#[derive(serde::Serialize, Clone)]
struct SessionEventPayload {
    id: String,
    event: SessionEvent,
}

pub fn start(socket: &Path, id: SessionId, app: AppHandle) -> Result<AttachHandle, String> {
    let stream = UnixStream::connect(socket)
        .map_err(|e| format!("connecting to the daemon at {}: {e}", socket.display()))?;
    let mut writer = stream
        .try_clone()
        .map_err(|e| format!("cloning the attach socket: {e}"))?;
    write_framed(&mut writer, &Request::SessionAttach { id: id.clone() })?;

    let mut reader = BufReader::new(stream);
    let thread_id = id.clone();
    std::thread::spawn(move || loop {
        let mut line = String::new();
        use std::io::BufRead;
        let read = match reader.read_line(&mut line) {
            Ok(n) => n,
            Err(_) => break,
        };
        if read == 0 {
            break;
        }
        let trimmed = line.trim_end_matches(['\r', '\n']);
        if trimmed.is_empty() {
            continue;
        }
        let Ok(event) = serde_json::from_str::<SessionEvent>(trimmed) else {
            continue;
        };
        let payload = SessionEventPayload {
            id: thread_id.to_string(),
            event,
        };
        if app.emit("session-event", payload).is_err() {
            break;
        }
    });

    Ok(AttachHandle { writer })
}

impl AttachHandle {
    pub fn send_input(&mut self, id: SessionId, data: Vec<u8>) -> Result<(), String> {
        write_framed(&mut self.writer, &Request::SessionInput { id, data })
    }

    pub fn send_resize(&mut self, id: SessionId, rows: u16, cols: u16) -> Result<(), String> {
        write_framed(&mut self.writer, &Request::SessionResize { id, rows, cols })
    }

    /// Shuts the socket down in both directions, which unblocks the
    /// reader thread's own blocking `read_line` with a clean EOF rather
    /// than leaving it parked forever on a pane that no longer exists.
    pub fn close(&self) {
        let _ = self.writer.shutdown(Shutdown::Both);
    }
}
