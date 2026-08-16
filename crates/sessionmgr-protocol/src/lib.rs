//! Wire types for `sessionmgr`'s two local sockets.
//!
//! This crate exists to fix a concrete structural problem PLAN.md's
//! adversarial review identified: a bin-only daemon crate cannot be
//! depended on by the TUI crate, so the two would have to either
//! duplicate their message definitions or be fused into one crate. A
//! separate protocol crate with no I/O and no logic lets the daemon, the
//! workers, the CLI clients, and (Phase 4) the TUI all share exactly one
//! definition of every message.
//!
//! The TUI in particular depends on **this crate only** -- never on
//! `sessionmgr-proc` or `sessionmgr-agents`. That is the ports-and-
//! adapters boundary made structural: a UI that cannot name a process
//! type cannot accidentally spawn one.
//!
//! # Framing
//!
//! One JSON value per line, UTF-8, `\n`-terminated. Chosen over a
//! length-prefixed binary framing because the payloads are small and
//! infrequent, and because a transcript of a debugging session is
//! readable with `cat`. `SessionEvent`s are the only high-volume message
//! and are still one line each.

use std::path::PathBuf;

use serde::{Deserialize, Serialize};
use sessionmgr_core::{Disposition, SessionId, SessionKind, SessionStatus};

/// A request from a client to the daemon, or from the daemon to a worker.
///
/// The two transports share one enum rather than splitting into
/// `PublicRequest`/`WorkerRequest`. They are disjoint in practice, and
/// the daemon rejects a worker-only request arriving on its public socket
/// (and vice versa) rather than making that unrepresentable. The reason
/// is [`Request::HookFire`]: it is genuinely both -- Phase 4's
/// `__hook-fire` subcommand is an outside process talking to the daemon
/// about a worker's session. One enum keeps that from needing a
/// translation layer later.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(tag = "type", rename_all = "kebab-case")]
pub enum Request {
    /// Liveness probe. Answered by [`Response::Pong`].
    ///
    /// Load-bearing rather than decorative: a socket that accepts a
    /// connection is not the same as a daemon that is ready to serve, and
    /// clients wait for a real answer before proceeding.
    Ping,

    /// Create a session and spawn a detached worker for it.
    SessionNew {
        kind: SessionKind,
        /// The command to run. Empty means "this platform's default
        /// shell", resolved by the daemon rather than the client so every
        /// client role agrees.
        command: Vec<String>,
        /// The repository to create the session against, for the two
        /// kinds that need one.
        ///
        /// Sent as the **client's** working directory rather than a
        /// repository root: the client is the process standing in the
        /// user's directory, and the daemon is not. Resolving it to a
        /// root is the daemon's job, so that a session created from a
        /// subdirectory lands in the same repository as one created from
        /// the top.
        repo: Option<PathBuf>,
    },

    /// List every known session.
    SessionList,

    /// Stream a session's output. The connection stays open and carries
    /// [`SessionEvent`]s until the client disconnects.
    SessionAttach { id: SessionId },

    /// Send a line of input to a session's process.
    SessionInput { id: SessionId, data: String },

    /// Tear a session down: graceful shutdown first, then terminate the
    /// recorded worker **and** child pids if it does not ack in time.
    SessionClose {
        id: SessionId,
        /// What to do with the session's branch. `None` leaves the
        /// worktree and branch in place, tearing down only the processes
        /// -- the safe default, since discarding work is not something to
        /// infer from a bare `close`.
        disposition: Option<Disposition>,
    },

    /// Shut the daemon down. Deliberately does **not** stop running
    /// sessions: workers are detached precisely so they outlive this.
    DaemonShutdown,

    /// Daemon -> worker only: shut this worker down gracefully.
    WorkerShutdown,
}

/// The daemon's (or worker's) answer to a [`Request`].
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(tag = "type", rename_all = "kebab-case")]
pub enum Response {
    Pong {
        /// The answering daemon's own pid, so a client can tell a
        /// freshly-started daemon from a stale one it just replaced.
        pid: u32,
    },
    SessionCreated {
        id: SessionId,
    },
    Sessions {
        sessions: Vec<SessionSummary>,
    },
    /// The request succeeded and has no payload.
    Ok,
    /// The request failed. `message` is human-facing; `kind` is for
    /// clients that need to branch (notably "not found", which
    /// `__hook-fire` must treat as a silent no-op rather than an error).
    Error {
        kind: ErrorKind,
        message: String,
    },
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "kebab-case")]
pub enum ErrorKind {
    /// No session with that id.
    ///
    /// Its own variant because Phase 4's globally-installed agent-CLI
    /// hook fires for *every* session on the machine, including ones
    /// launched entirely outside this tool. `__hook-fire` must fast-path
    /// no-op on an unrecognised id rather than erroring or, worse,
    /// triggering the daemon auto-start sugar (PLAN.md, adversarial
    /// finding #13).
    NotFound,
    /// The request is not legal in the session's current state -- a
    /// double-close, say.
    Conflict,
    /// Malformed request, or a request on the wrong transport.
    Protocol,
    /// Something failed underneath: I/O, spawn, permissions.
    Internal,
}

/// One row of `sessionmgr list`.
///
/// A projection of `sessionmgr_core::Session` rather than the session
/// itself: what a client needs to render is not the same as what the
/// daemon needs to persist, and pids in particular have no business on a
/// list a UI renders.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct SessionSummary {
    pub id: SessionId,
    pub kind: SessionKind,
    pub status: SessionStatus,
    pub command: Vec<String>,
    /// Where the session's command runs. `None` for a plain terminal.
    #[serde(default)]
    pub cwd: Option<PathBuf>,
    /// The branch a worktree session owns.
    #[serde(default)]
    pub branch: Option<String>,
    pub created_at_millis: u64,
    pub exit_code: Option<i32>,
}

/// A live event streamed to an attached client.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(tag = "type", rename_all = "kebab-case")]
pub enum SessionEvent {
    /// A chunk of the session process's output.
    ///
    /// `String`, not `Vec<u8>`: output is decoded lossily at the worker,
    /// once, rather than pushing the decision onto every consumer and
    /// bloating the JSON with a byte array. This is a real (and recorded)
    /// limitation for a future PTY-backed session carrying control
    /// sequences -- see the Phase 1 PTY spike, which is exactly what
    /// decides whether this type needs to become bytes.
    Output { data: String },

    /// The session's status changed.
    Status { status: SessionStatus },

    /// The session's process exited.
    Exited { code: Option<i32> },

    /// Emitted first when a client attaches to a session whose worker was
    /// adopted after a daemon restart -- i.e. the work survived the
    /// manager closing, which is the whole point of the architecture and
    /// is worth saying out loud rather than leaving the user to infer
    /// from a gap in the transcript.
    RecoveryMarker,
}

#[cfg(test)]
mod tests {
    use super::*;

    fn round_trip<T>(value: &T) -> T
    where
        T: Serialize + for<'de> Deserialize<'de>,
    {
        let line = serde_json::to_string(value).expect("serialize");
        assert!(
            !line.contains('\n'),
            "a framed message must never contain a newline, or it would be read as two messages"
        );
        serde_json::from_str(&line).expect("deserialize")
    }

    #[test]
    fn requests_round_trip() {
        let id = SessionId::new(1_700_000_000_000, 1);
        for request in [
            Request::Ping,
            Request::SessionNew {
                kind: SessionKind::PlainTerminal,
                command: vec!["sh".to_owned()],
                repo: None,
            },
            Request::SessionList,
            Request::SessionAttach { id: id.clone() },
            Request::SessionInput {
                id: id.clone(),
                data: "echo hi".to_owned(),
            },
            Request::SessionClose {
                id,
                disposition: Some(Disposition::Merge),
            },
            Request::DaemonShutdown,
            Request::WorkerShutdown,
        ] {
            assert_eq!(round_trip(&request), request);
        }
    }

    #[test]
    fn responses_and_events_round_trip() {
        for response in [
            Response::Pong { pid: 42 },
            Response::Ok,
            Response::Error {
                kind: ErrorKind::NotFound,
                message: "no such session".to_owned(),
            },
        ] {
            assert_eq!(round_trip(&response), response);
        }
        for event in [
            SessionEvent::Output {
                data: "hello".to_owned(),
            },
            SessionEvent::Status {
                status: SessionStatus::Running,
            },
            SessionEvent::Exited { code: Some(0) },
            SessionEvent::RecoveryMarker,
        ] {
            assert_eq!(round_trip(&event), event);
        }
    }

    #[test]
    fn output_containing_newlines_survives_framing() {
        // The framing is line-delimited, so any output chunk with a
        // newline in it is the obvious way to corrupt the stream. JSON
        // escaping is what prevents that, and this asserts it rather than
        // assuming it.
        let event = SessionEvent::Output {
            data: "line one\nline two\r\n".to_owned(),
        };
        assert_eq!(round_trip(&event), event);
    }

    #[test]
    fn an_invalid_session_id_on_the_wire_is_rejected() {
        // Ids are path components; see `SessionId`'s `FromStr`. A peer
        // that sends `../../etc` must be rejected at the boundary rather
        // than reaching the filesystem.
        let hostile = r#"{"type":"session-attach","id":"../../etc"}"#;
        assert!(serde_json::from_str::<Request>(hostile).is_err());
    }
}
