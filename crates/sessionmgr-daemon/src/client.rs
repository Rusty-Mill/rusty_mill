//! The client roles: `new`, `list`, `attach`, `close`, and the daemon
//! lifecycle subcommands.
//!
//! Clients transparently start a daemon if none is running. That sugar is
//! what makes `sessionmgr new` useful on its own, before any TUI exists
//! -- the user should not have to know this tool has a daemon at all.

use std::path::{Path, PathBuf};
use std::time::Duration;

use rusty_tokio::io::AsyncBufReadExt;
use sessionmgr_core::{Disposition, SessionId, SessionKind};
use sessionmgr_protocol::{Request, Response, SessionEvent, SessionSummary};

use crate::error::{Error, Result};
use crate::{paths, supervisor, transport};

const DAEMON_READY_TIMEOUT: Duration = Duration::from_secs(20);

/// Starts a daemon if none is running, then returns a connection to it.
pub async fn connect(root: &Path) -> Result<transport::Connection> {
    ensure_daemon(root).await?;
    transport::Connection::connect("connecting to the daemon", &paths::daemon_socket(root)).await
}

/// Starts a detached daemon unless one is already running.
pub async fn ensure_daemon(root: &Path) -> Result<()> {
    if supervisor::running_daemon(root).is_some() {
        return Ok(());
    }
    start_daemon_detached(root).await
}

/// Spawns `sessionmgr daemon run` as a detached process and waits until
/// it answers.
pub async fn start_daemon_detached(root: &Path) -> Result<()> {
    use rusty_tokio::process::{Command, Stdio};

    paths::ensure_dir("creating the state root", root)?;
    let exe = std::env::current_exe().map_err(|e| Error::io("locating this executable", None, e))?;
    let log_path = paths::daemon_log(root);
    let log = std::fs::File::create(&log_path)
        .map_err(|e| Error::io("creating the daemon log", log_path, e))?;

    let mut cmd = Command::new(&exe);
    cmd.arg("daemon")
        .arg("run")
        // Passed explicitly rather than relying on the child inheriting
        // this process's environment: the daemon outlives whatever shell
        // started it, and a state root that depended on an inherited
        // variable would be a different directory the next time it was
        // auto-started from somewhere else.
        .arg("--state-root")
        .arg(root)
        .stdin(Stdio::null())
        .stdout(Stdio::null())
        .stderr(Stdio::from(log));
    sessionmgr_proc::prepare_detached(&mut cmd);

    let mut child = cmd
        .spawn()
        .map_err(|e| Error::io("starting the daemon", exe.clone(), e))?;
    rusty_tokio::spawn(async move {
        let _ = child.wait().await;
    });

    transport::wait_ready(
        &paths::daemon_socket(root),
        Request::Ping,
        |response: &Response| matches!(response, Response::Pong { .. }),
        DAEMON_READY_TIMEOUT,
    )
    .await
    .map_err(|e| {
        // A daemon that died before binding leaves its reason in the log
        // and nothing on the socket, so point at the log rather than
        // reporting a bare timeout.
        Error::conflict(format!(
            "the daemon did not become ready ({e}); see {}",
            paths::daemon_log(root).display()
        ))
    })
}

/// How long a one-shot client command waits for the daemon to answer.
///
/// Generous, because `SessionNew` legitimately takes as long as it takes
/// a worker to start (up to `WORKER_READY_TIMEOUT`), and on Windows that
/// covers process creation with an antivirus scanner in the path.
///
/// But bounded, which the socket reads underneath are not. A client that
/// hangs forever against a wedged daemon gives the user nothing to act on
/// and nothing to report; a client that gives up after a minute names the
/// daemon and its log. This is a backstop for a bug rather than an
/// expected path -- if it ever fires, something is wrong that this
/// message should help find.
const REQUEST_TIMEOUT: Duration = Duration::from_secs(60);

/// One request, one response, against a possibly-auto-started daemon.
async fn request(root: &Path, request: Request) -> Result<Response> {
    let mut conn = connect(root).await?;
    let response: Response = match rusty_tokio::time::timeout(
        REQUEST_TIMEOUT,
        conn.request(&request),
    )
    .await
    {
        Ok(result) => result?,
        Err(_) => {
            return Err(Error::conflict(format!(
                "the daemon accepted the request but did not answer within {}s; see {}",
                REQUEST_TIMEOUT.as_secs(),
                paths::daemon_log(root).display()
            )))
        }
    };
    match response {
        Response::Error { kind, message } => Err(match kind {
            sessionmgr_protocol::ErrorKind::NotFound => Error::NotFound { id: message },
            sessionmgr_protocol::ErrorKind::Conflict => Error::conflict(message),
            _ => Error::protocol(message),
        }),
        other => Ok(other),
    }
}

pub async fn session_new(
    root: &Path,
    kind: SessionKind,
    command: Vec<String>,
    repo: Option<PathBuf>,
    pty: bool,
) -> Result<SessionId> {
    match request(
        root,
        Request::SessionNew {
            kind,
            command,
            repo,
            pty,
        },
    )
    .await?
    {
        Response::SessionCreated { id } => Ok(id),
        other => Err(Error::protocol(format!(
            "unexpected answer to session-new: {other:?}"
        ))),
    }
}

pub async fn session_list(root: &Path) -> Result<Vec<SessionSummary>> {
    match request(root, Request::SessionList).await? {
        Response::Sessions { sessions } => Ok(sessions),
        other => Err(Error::protocol(format!(
            "unexpected answer to session-list: {other:?}"
        ))),
    }
}

pub async fn session_close(
    root: &Path,
    id: SessionId,
    disposition: Option<Disposition>,
) -> Result<()> {
    match request(root, Request::SessionClose { id, disposition }).await? {
        Response::Ok => Ok(()),
        other => Err(Error::protocol(format!(
            "unexpected answer to session-close: {other:?}"
        ))),
    }
}

/// Attaches to a session: streams its output to stdout and forwards this
/// process's stdin to it, until the stream ends or the user disconnects.
pub async fn session_attach(root: &Path, id: SessionId) -> Result<()> {
    let conn = connect(root).await?;
    let (mut reader, mut writer) = conn.into_parts();
    transport::write_framed(&mut writer, &Request::SessionAttach { id: id.clone() }).await?;

    // Forward stdin. Line-oriented: this is a CLI attach, not the TUI,
    // and the terminal is line-buffered anyway until Phase 4 puts a real
    // raw-mode front end on top.
    rusty_tokio::spawn(async move {
        let stdin = rusty_tokio::io::BufReader::new(rusty_tokio::io::stdin());
        let mut lines = stdin.lines();
        while let Ok(Some(line)) = lines.next_line().await {
            let request = Request::SessionInput {
                id: id.clone(),
                data: format!("{line}\n").into_bytes(),
            };
            if transport::write_framed(&mut writer, &request).await.is_err() {
                return;
            }
        }
    });

    use std::io::Write;
    loop {
        let event: Option<SessionEvent> = transport::read_framed(&mut reader).await?;
        let Some(event) = event else { return Ok(()) };
        match event {
            SessionEvent::Output { data } => {
                // Written as raw bytes, not through `print!`: session
                // output is a terminal byte stream carrying escape
                // sequences, and forcing it through a `String` would
                // corrupt any multi-byte character split across a chunk
                // boundary -- the exact defect that made this type bytes.
                let _ = std::io::stdout().write_all(&data);
                // Explicitly flushed: output frequently arrives without a
                // trailing newline (a prompt waiting for an answer is the
                // important case), and line-buffered stdout would hold
                // exactly those back.
                let _ = std::io::stdout().flush();
            }
            SessionEvent::Status { status } => eprintln!("[session {status:?}]"),
            SessionEvent::Exited { code } => {
                eprintln!("[session exited with {code:?}]");
                return Ok(());
            }
            SessionEvent::RecoveryMarker => {
                eprintln!("[reattached to a session that survived a manager restart]")
            }
        }
    }
}

pub async fn daemon_status(root: &Path) -> Result<String> {
    match supervisor::running_daemon(root) {
        Some(state) => Ok(format!("running pid={}", state.pid)),
        None => Ok("not running".to_owned()),
    }
}

/// Shuts the daemon down. **Running sessions are deliberately left
/// running** -- that is the whole point of detached workers, and is
/// exactly what the manual verification in PLAN.md exercises.
pub async fn daemon_shutdown(root: &Path) -> Result<()> {
    if supervisor::running_daemon(root).is_none() {
        return Ok(());
    }
    let mut conn =
        transport::Connection::connect("connecting to the daemon", &paths::daemon_socket(root))
            .await?;
    let _: Response = conn.request(&Request::DaemonShutdown).await?;
    Ok(())
}

/// Renders `sessionmgr list` as a plain table.
pub fn render_sessions(sessions: &[SessionSummary]) -> String {
    if sessions.is_empty() {
        return "no sessions".to_owned();
    }
    let mut out = String::from("ID            STATUS       KIND            BRANCH                    COMMAND\n");
    for session in sessions {
        out.push_str(&format!(
            "{:<13} {:<12} {:<15} {:<25} {}\n",
            session.id,
            format!("{:?}", session.status),
            format!("{:?}", session.kind),
            // A same-directory session deliberately shows no branch: it
            // works on whatever the repository is already on, which is
            // exactly the property that makes it the unisolated choice.
            session.branch.as_deref().unwrap_or("-"),
            session.command.join(" "),
        ));
    }
    out.trim_end().to_owned()
}

/// Where a client resolves its state root from, honouring an explicit
/// `--state-root` over the environment.
pub fn resolve_root(explicit: Option<PathBuf>) -> Result<PathBuf> {
    match explicit {
        Some(path) => Ok(path),
        None => paths::state_root(),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn an_empty_session_list_says_so_rather_than_printing_a_bare_header() {
        assert_eq!(render_sessions(&[]), "no sessions");
    }

    #[test]
    fn rendering_a_session_includes_its_id_status_and_command() {
        let summary = SessionSummary {
            id: SessionId::new(1_700_000_000_000, 1),
            kind: SessionKind::PlainTerminal,
            status: sessionmgr_core::SessionStatus::Running,
            command: vec!["/bin/sh".to_owned()],
            cwd: None,
            branch: None,
            created_at_millis: 1_700_000_000_000,
            exit_code: None,
        };
        let rendered = render_sessions(std::slice::from_ref(&summary));
        assert!(rendered.contains(summary.id.as_str()));
        assert!(rendered.contains("Running"));
        assert!(rendered.contains("/bin/sh"));
    }

    #[test]
    fn an_explicit_state_root_beats_the_environment() {
        let explicit = PathBuf::from("/explicitly/here");
        assert_eq!(
            resolve_root(Some(explicit.clone())).expect("resolve"),
            explicit
        );
    }
}
