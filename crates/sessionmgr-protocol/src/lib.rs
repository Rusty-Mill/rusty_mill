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

pub mod base64;

use std::path::PathBuf;

use serde::{Deserialize, Serialize};
/// Re-exported, not just `use`d privately: a crate that depends on
/// `sessionmgr-protocol` only (the TUI, deliberately -- see the
/// crate-level docs) needs to name these types too, to hold a
/// `SessionId` or match on a `SessionStatus`, without adding
/// `sessionmgr-core` as a second dependency.
pub use sessionmgr_core::ports::ChangedFile;
pub use sessionmgr_core::{AgentKind, Disposition, SessionId, SessionKind, SessionStatus};

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
        /// Run the session's process on a real terminal.
        ///
        /// Defaults to true at the CLI. `false` selects the piped
        /// backend, which cannot host an interactive agent CLI but whose
        /// survives-the-manager-closing behaviour is proven on Windows.
        pty: bool,
        /// Which agent CLI to run, if any. `Some` resolves `command`
        /// through that agent's `launch_args` (an initial prompt, say)
        /// rather than treating it as the literal program to run, and
        /// turns on tier-3 `needs_input` detection for the session.
        agent: Option<AgentKind>,
        /// Install `agent`'s own hook config into the session's
        /// worktree, calling back into `sessionmgr __hook-fire` for
        /// tier-1 status detection and webhook dispatch. Requires
        /// `agent` to be `Some` and `kind` to be `Worktree` -- opt-in,
        /// not automatic, since it also changes the launched command
        /// (some CLIs need an extra flag to skip an interactive
        /// hook-trust gate) and writes a file into the session's own
        /// directory.
        hooks: bool,
        /// Phase 5: the session this one depends on. `Some` requires
        /// `kind` to be [`SessionKind::Dependent`] and `repo` to be
        /// `None` -- a dependent session's workspace is derived entirely
        /// from the parent's own workspace, not from a repository path
        /// the caller supplies (see
        /// `sessionmgr_core::workspace::Workspace::dependent`).
        parent: Option<SessionId>,
        /// Should this session hold off starting until `parent` finishes,
        /// rather than starting immediately? Meaningless unless `parent`
        /// is `Some`. `sessionmgr new --parent <id>` defaults this
        /// `true`, matching CAPABILITIES.md's observed default ("the
        /// child can be configured to wait... with a start now
        /// override"); `--start-now` sets it `false`.
        wait_for_parent: bool,
    },

    /// List every known session.
    SessionList,

    /// Stream a session's output. The connection stays open and carries
    /// [`SessionEvent`]s until the client disconnects.
    SessionAttach { id: SessionId },

    /// Send input to a session's process.
    ///
    /// Bytes, not text, for the same reason [`SessionEvent::Output`] is:
    /// a terminal's input stream carries control bytes and escape
    /// sequences (arrow keys, Ctrl-C, bracketed paste), and a `String`
    /// cannot represent a lone `0x03`.
    SessionInput {
        id: SessionId,
        #[serde(with = "crate::base64::bytes")]
        data: Vec<u8>,
    },

    /// Tell a session's terminal it has been resized.
    ///
    /// A PTY-hosted program lays its output out to the size it was told,
    /// so a session whose terminal is never resized renders to whatever
    /// size it was given at creation -- which for a session created by a
    /// background client is a default, not the user's actual window.
    SessionResize { id: SessionId, rows: u16, cols: u16 },

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

    /// Sets (`Some`) or clears (`None`) a session's purely cosmetic
    /// display label -- distinct from its worktree branch, which never
    /// changes. CAPABILITIES.md's Xirp-observed "renaming a session",
    /// wired to the TUI command palette.
    SessionRename { id: SessionId, name: Option<String> },

    /// The CAPABILITIES.md-observed "start now" override, applied to a
    /// session that is **already** [`SessionStatus::Waiting`] rather than
    /// at creation time (`SessionNew`'s own `wait_for_parent: false`
    /// covers the creation-time case). A no-op error, not a silent
    /// success, on a session that is not currently `Waiting` -- there is
    /// nothing to skip ahead of.
    SessionStartNow { id: SessionId },

    /// CAPABILITIES.md's "Fork session": clones `id`'s own conversation
    /// history into a brand-new, independent session with its own
    /// worktree (branched from `id`'s own branch tip, so the new
    /// session's code state matches the conversation it starts with) --
    /// see `docs/decisions/0003-resume-fork-spike.md` for how this is
    /// possible at all and `docs/phase-6-report.md` for the full design
    /// and which agents support it today.
    SessionFork {
        id: SessionId,
        /// Same meaning as `SessionNew`'s own `pty` field.
        pty: bool,
    },

    /// CAPABILITIES.md's "Switch agent mid-session": stops `id`'s live
    /// agent conversation and creates a brand-new session running
    /// `agent` instead, in the **same** workspace (there is one line of
    /// work here, not two, unlike `SessionFork`) -- seeded with a
    /// rendered handoff of `id`'s own transcript as the new agent's
    /// initial prompt rather than a native per-CLI state translation
    /// (see `sessionmgr_agents::handoff`'s own docs for why). `id`
    /// transitions to [`SessionStatus::SwitchedAway`] and keeps its
    /// workspace on disk, now owned by the new session. See
    /// `docs/phase-7-report.md`.
    SessionSwitchAgent {
        id: SessionId,
        agent: AgentKind,
        /// Same meaning as `SessionNew`'s own `pty` field.
        pty: bool,
    },

    /// The files changed in a session's workspace, for the diff pane.
    /// `NotFound` if the session has no workspace (a `PlainTerminal`
    /// session, or one that failed before a workspace was set up).
    GitStatus { id: SessionId },

    /// A unified diff of a session's workspace. `path` narrows it to one
    /// file, matching [`sessionmgr_core::ports::GitPort::diff`].
    GitDiff { id: SessionId, path: Option<String> },

    /// Shut the daemon down. Deliberately does **not** stop running
    /// sessions: workers are detached precisely so they outlive this.
    DaemonShutdown,

    /// Daemon -> worker only: shut this worker down gracefully.
    WorkerShutdown,

    /// A CLI's own hook fired. Sent by `sessionmgr __hook-fire` (an
    /// installed hook command invokes the `sessionmgr` binary directly,
    /// which then talks over this same socket protocol) to the public
    /// socket, which forwards it to the named session's worker -- the
    /// same proxy path `SessionInput`/`SessionResize` already use.
    ///
    /// `session_id` is a raw `String`, not `SessionId`: an unrecognized
    /// or malformed id must be a fast, silent no-op (PLAN.md's own
    /// requirement -- a hook installed by this tool only ever fires for
    /// a session this tool created, but a stray or copied hook config
    /// firing for something else must never error, block, or trigger
    /// the daemon's auto-start sugar), and a typed `SessionId` would
    /// reject a malformed one at deserialization instead of letting the
    /// handler apply that rule.
    HookFire { session_id: String, event: String },
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
    /// Answer to [`Request::GitStatus`].
    GitStatus {
        files: Vec<sessionmgr_core::ports::ChangedFile>,
    },
    /// Answer to [`Request::GitDiff`].
    GitDiff {
        diff: String,
    },
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
    /// Which agent CLI this session runs, if any -- surfaced so a
    /// client (the TUI's status badge, eventually) can tell an
    /// adapter-backed session from a plain command.
    #[serde(default)]
    pub agent: Option<AgentKind>,
    /// The user-chosen display label set by the TUI command palette's
    /// `rename` action, if any.
    #[serde(default)]
    pub name: Option<String>,
    /// The session this one depends on, for a [`SessionKind::Dependent`]
    /// session -- surfaced so a client can group a dependent session
    /// with its parent (CAPABILITIES.md: "grouped together in the
    /// sidebar") and, together with `status ==` [`SessionStatus::Waiting`],
    /// render "waiting on parent".
    #[serde(default)]
    pub parent: Option<SessionId>,
    /// The session this one was forked from, if it was -- a distinct
    /// relationship from `parent` (workspace-sharing); see
    /// `sessionmgr_core::Session::forked_from`'s own docs for why.
    #[serde(default)]
    pub forked_from: Option<SessionId>,
    /// The session this one took over from via switch-agent-mid-session,
    /// if it was -- a third relationship, distinct from both `parent`
    /// and `forked_from`; see `sessionmgr_core::Session::switched_from`'s
    /// own docs for why.
    #[serde(default)]
    pub switched_from: Option<SessionId>,
}

/// A live event streamed to an attached client.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(tag = "type", rename_all = "kebab-case")]
pub enum SessionEvent {
    /// A chunk of the session process's output.
    ///
    /// **Bytes, deliberately.** This was a `String` with a per-chunk
    /// lossy decode until the Phase 1 PTY spike settled the question (see
    /// `docs/decisions/0002-pty-required-for-agent-sessions.md`): a real
    /// terminal is mandatory for interactive agent CLIs, and terminal
    /// output is a byte stream of text interleaved with ANSI and
    /// cursor-positioning sequences. Two things broke under `String`:
    ///
    /// 1. A multi-byte character split across a read boundary decoded to
    ///    replacement characters -- permanent corruption, since the
    ///    transcript is append-only.
    /// 2. Escape sequences are not text and have no business being
    ///    validated as UTF-8.
    ///
    /// Carried over the wire as base64 (see [`crate::base64`]), because
    /// the framing is line-delimited JSON and serde's default encoding
    /// for `Vec<u8>` is a number-per-byte array.
    Output {
        #[serde(with = "crate::base64::bytes")]
        data: Vec<u8>,
    },

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
                pty: true,
                agent: Some(AgentKind::Codex),
                hooks: true,
                parent: None,
                wait_for_parent: false,
            },
            Request::SessionNew {
                kind: SessionKind::Dependent,
                command: vec![],
                repo: None,
                pty: true,
                agent: None,
                hooks: false,
                parent: Some(id.clone()),
                wait_for_parent: true,
            },
            Request::SessionList,
            Request::SessionAttach { id: id.clone() },
            Request::SessionInput {
                id: id.clone(),
                data: b"echo hi\n".to_vec(),
            },
            Request::SessionResize {
                id: id.clone(),
                rows: 40,
                cols: 120,
            },
            Request::GitStatus { id: id.clone() },
            Request::GitDiff {
                id: id.clone(),
                path: Some("src/lib.rs".to_owned()),
            },
            Request::SessionClose {
                id: id.clone(),
                disposition: Some(Disposition::Merge),
            },
            Request::SessionRename {
                id: id.clone(),
                name: Some("my session".to_owned()),
            },
            Request::SessionStartNow { id: id.clone() },
            Request::SessionFork {
                id: id.clone(),
                pty: true,
            },
            Request::SessionSwitchAgent {
                id,
                agent: AgentKind::Codex,
                pty: true,
            },
            Request::DaemonShutdown,
            Request::WorkerShutdown,
            Request::HookFire {
                session_id: "not-a-real-sessionid-shape".to_owned(),
                event: "Stop".to_owned(),
            },
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
            Response::GitStatus {
                files: vec![ChangedFile {
                    status: " M".to_owned(),
                    path: "src/lib.rs".to_owned(),
                }],
            },
            Response::GitDiff {
                diff: "diff --git a/x b/x".to_owned(),
            },
        ] {
            assert_eq!(round_trip(&response), response);
        }
        for event in [
            SessionEvent::Output {
                data: b"hello".to_vec(),
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
            data: b"line one\nline two\r\n\x1b[31m\x00\xff".to_vec(),
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
