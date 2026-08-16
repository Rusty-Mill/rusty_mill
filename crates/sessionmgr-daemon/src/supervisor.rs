//! The daemon: the long-running supervisor that owns the session
//! registry and the public socket, and that is **meant to be outlived by
//! its workers**.
//!
//! The inversion is the point. In an ordinary supervisor, the supervised
//! processes die with the supervisor. Here the workers are detached, and
//! this process is the disposable one -- the user closes the manager all
//! the time, and none of the running agent sessions should notice.
//!
//! That shapes everything below:
//!
//! - Startup **adopts** what it finds still running rather than starting
//!   it, and never respawns what it finds dead.
//! - `DaemonShutdown` deliberately does not stop any session.
//! - Liveness is answered from disk plus a pid probe, never from an owned
//!   `Child` handle, because after a restart there are no owned handles
//!   to consult.

use std::path::{Path, PathBuf};
use std::sync::Arc;
use std::time::Duration;

use rusty_tokio::sync::Notify;
use serde::{Deserialize, Serialize};
use sessionmgr_core::ports::GitPort;
use sessionmgr_core::{
    Disposition, RecoveryAction, SessionId, SessionKind, SessionStatus, Workspace,
};
use sessionmgr_git::SystemGit;
use sessionmgr_protocol::{Request, Response, SessionEvent};

use crate::error::{Error, Result};
use crate::{catalog, paths, transport, worker};

/// How long to wait for a freshly spawned worker to answer on its socket.
///
/// Generous, because this covers process creation on Windows with an
/// antivirus scanner in the path -- PLAN.md flags Defender interaction as
/// a real, Windows-native risk rather than a theoretical one.
const WORKER_READY_TIMEOUT: Duration = Duration::from_secs(20);

/// How long a worker gets to acknowledge a graceful shutdown before it is
/// terminated.
const GRACEFUL_CLOSE_TIMEOUT: Duration = Duration::from_secs(5);

/// The daemon's own pointer file.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct DaemonState {
    pub pid: u32,
    pub start_fingerprint: Option<String>,
}

/// Reads the recorded daemon pointer, if one exists and parses.
pub fn read_daemon_state(root: &Path) -> Option<DaemonState> {
    let text = std::fs::read_to_string(paths::daemon_state(root)).ok()?;
    serde_json::from_str(&text).ok()
}

/// Is a daemon currently running for this state root?
///
/// Asked as the pid-reuse-safe question, for the same reason session
/// workers are: a stale `daemon.json` whose pid has since been recycled
/// by an unrelated process would otherwise make every client refuse to
/// start a daemon, and the tool would appear permanently broken with no
/// obvious cause.
pub fn running_daemon(root: &Path) -> Option<DaemonState> {
    let state = read_daemon_state(root)?;
    sessionmgr_proc::is_same_process(state.pid, state.start_fingerprint.as_deref())
        .ok()
        .filter(|alive| *alive)
        .map(|_| state)
}

struct Supervisor {
    root: PathBuf,
    exe: PathBuf,
    shutdown: Notify,
}

/// Runs the daemon in the foreground. Returns when a `DaemonShutdown`
/// request is served.
pub async fn run(root: PathBuf) -> Result<()> {
    if let Some(existing) = running_daemon(&root) {
        return Err(Error::conflict(format!(
            "a sessionmgr daemon is already running for this state root (pid {})",
            existing.pid
        )));
    }

    paths::ensure_dir("creating the state root", &root)?;
    let exe = std::env::current_exe()
        .map_err(|e| Error::io("locating this executable", None, e))?;

    let supervisor = Arc::new(Supervisor {
        root: root.clone(),
        exe,
        shutdown: Notify::new(),
    });

    // Recovery runs **before** the socket exists, and the ordering is
    // load-bearing. Binding first opened a window where a client could
    // connect -- successfully, into the listen backlog -- while this
    // process was still probing pids and nothing was accepting yet. The
    // client then waited for an answer that could not come until recovery
    // finished. With no socket yet, a client simply fails to connect and
    // retries, which its readiness loop already handles correctly.
    //
    // It also means no client can ever observe the registry mid-recovery,
    // reading a session as `Running` a moment before it is marked crashed.
    supervisor.reconcile_all()?;

    let listener = transport::Listener::bind("binding the daemon socket", &paths::daemon_socket(&root))?;

    // Written after the bind succeeds, never before: a pointer file
    // advertising a daemon that then failed to bind would send every
    // client to a socket that does not exist.
    let me = std::process::id();
    let state = DaemonState {
        pid: me,
        start_fingerprint: sessionmgr_proc::start_fingerprint(me).ok().flatten(),
    };
    let state_path = paths::daemon_state(&root);
    std::fs::write(
        &state_path,
        serde_json::to_string_pretty(&state)?,
    )
    .map_err(|e| Error::io("writing the daemon pointer file", state_path.clone(), e))?;

    let accept_loop = rusty_tokio::spawn({
        let supervisor = Arc::clone(&supervisor);
        async move {
            loop {
                match listener.accept().await {
                    Ok(conn) => {
                        rusty_tokio::spawn(serve(Arc::clone(&supervisor), conn));
                    }
                    Err(e) => {
                        eprintln!("sessionmgr daemon: accept failed: {e}");
                        return;
                    }
                }
            }
        }
    });

    supervisor.shutdown.notified().await;
    accept_loop.abort();

    // Best-effort cleanup. The socket file goes with the `Listener`'s
    // own `Drop`; this removes the pointer file so the next client does
    // not have to probe a pid that is about to be gone.
    let _ = std::fs::remove_file(paths::daemon_state(&root));
    Ok(())
}

impl Supervisor {
    /// Applies the recovery rule to every session on disk.
    ///
    /// This runs once at startup and is the moment the whole persistence
    /// design either works or does not: sessions whose workers survived
    /// this daemon's predecessor are adopted, and the rest are honestly
    /// marked crashed.
    fn reconcile_all(&self) -> Result<()> {
        let mut adopted = 0usize;
        let mut crashed = 0usize;
        for session in catalog::list_sessions(&self.root)? {
            match catalog::recovery_for(&session) {
                RecoveryAction::Adopt => adopted += 1,
                RecoveryAction::MarkCrashed => crashed += 1,
                RecoveryAction::LeaveAsIs => {}
            }
            catalog::reconcile(&self.root, session)?;
        }
        if adopted > 0 || crashed > 0 {
            eprintln!(
                "sessionmgr daemon: adopted {adopted} running session(s), \
                 marked {crashed} crashed"
            );
        }
        Ok(())
    }

    async fn handle(&self, request: Request) -> Result<Response> {
        match request {
            Request::Ping => Ok(Response::Pong {
                pid: std::process::id(),
            }),
            Request::SessionNew {
                kind,
                command,
                repo,
                pty,
            } => self.session_new(kind, command, repo, pty).await,
            Request::SessionList => self.session_list(),
            Request::SessionInput { id, data } => self.session_input(id, data).await,
            Request::SessionResize { id, rows, cols } => self.session_resize(id, rows, cols).await,
            Request::SessionClose { id, disposition } => {
                self.session_close(id, disposition).await
            }
            Request::DaemonShutdown => {
                self.shutdown.notify_one();
                Ok(Response::Ok)
            }
            // Handled by `serve`, which needs the connection itself.
            Request::SessionAttach { .. } => Err(Error::protocol(
                "attach must be handled on its own connection",
            )),
            Request::WorkerShutdown => Err(Error::protocol(
                "worker-shutdown is not accepted on the public socket",
            )),
        }
    }

    async fn session_new(
        &self,
        kind: SessionKind,
        command: Vec<String>,
        repo: Option<PathBuf>,
        pty: bool,
    ) -> Result<Response> {
        let command = if command.is_empty() {
            default_shell()
        } else {
            command
        };
        let id = sessionmgr_proc::session_id()
            .map_err(|e| Error::io("generating a session id", None, e))?;

        let workspace = self.prepare_workspace(kind, repo, &id)?;
        let session = sessionmgr_core::Session::new(
            id.clone(),
            kind,
            command,
            workspace,
            pty,
            sessionmgr_proc::now_millis(),
        );
        // Written before the spawn, never after: if this process dies in
        // the window between the two, a record with no worker is
        // recoverable (it reconciles to `Crashed`), whereas a running
        // worker with no record on disk is unreachable garbage.
        catalog::write_session(&self.root, &session)?;

        let worker_pid = worker::spawn_detached(&self.exe, &self.root, &id)?;

        // Readiness is the **session record**, not the worker's socket.
        //
        // Probing the socket instead is the obvious design and it is
        // wrong: a session whose command exits immediately (`echo`, or a
        // typo'd executable) has a worker that has already recorded the
        // outcome and exited by the time the probe arrives, so `new`
        // would report a connection failure for a session that ran
        // perfectly. Caught by `session_lifecycle`'s finished/errored
        // tests, which failed exactly this way.
        //
        // The record covers both outcomes, because the worker leaves
        // `Created` whether it starts serving or exits first. And the
        // worker binds its socket *before* publishing `Running`, so a
        // record past `Created` also guarantees the socket exists for an
        // attach that follows immediately.
        let deadline = std::time::Instant::now() + WORKER_READY_TIMEOUT;
        loop {
            if catalog::read_session(&self.root, &id)?.status != SessionStatus::Created {
                return Ok(Response::SessionCreated { id });
            }
            // The worker died without recording anything -- fail with a
            // pointer to the only place its reason was written.
            if !sessionmgr_proc::is_alive(worker_pid).unwrap_or(false) {
                return Err(Error::conflict(format!(
                    "the worker for session {id} exited before starting it; see {}",
                    paths::worker_log(&self.root, &id).display()
                )));
            }
            if std::time::Instant::now() >= deadline {
                return Err(Error::conflict(format!(
                    "the worker for session {id} did not start it in time; see {}",
                    paths::worker_log(&self.root, &id).display()
                )));
            }
            rusty_tokio::time::sleep(Duration::from_millis(25)).await;
        }
    }

    /// Resolves the repository and creates the worktree, if the session's
    /// kind calls for one.
    ///
    /// Done **before** the session record is written, so a failure here
    /// leaves nothing behind at all. Writing the record first would leave
    /// a session pointing at a worktree that does not exist -- visible in
    /// `list`, impossible to attach to, and needing its own cleanup path.
    fn prepare_workspace(
        &self,
        kind: SessionKind,
        repo: Option<PathBuf>,
        id: &SessionId,
    ) -> Result<Option<Workspace>> {
        if !kind.needs_repo() {
            return Ok(None);
        }
        let from = repo.ok_or_else(|| {
            Error::usage(format!(
                "a {kind:?} session needs a repository; run this from inside one \
                 or pass --repo <path>"
            ))
        })?;
        let git = SystemGit;
        // Resolved from the client's directory to a repository root, so a
        // session created deep inside a repo lands in the same place as
        // one created at the top.
        let root = git
            .repo_root(&from)
            .map_err(|e| Error::usage(e.to_string()))?;

        match kind {
            SessionKind::SameDirectory => Ok(Some(Workspace::same_directory(root))),
            SessionKind::Worktree => {
                let workspace = Workspace::worktree(root.clone(), id);
                let branch = workspace.branch.clone().unwrap_or_default();
                git.worktree_add(&root, &workspace.cwd, &branch)
                    .map_err(|e| Error::conflict(e.to_string()))?;
                Ok(Some(workspace))
            }
            SessionKind::PlainTerminal => Ok(None),
        }
    }

    /// Removes a worktree session's worktree and branch according to
    /// `disposition`.
    ///
    /// Runs **after** the session's processes are dead. A worktree cannot
    /// be removed while something still holds a file open inside it, and
    /// on Windows that is not advisory -- an open handle makes the
    /// directory genuinely undeletable. Ordering teardown as
    /// processes-then-files is what makes the removal likely to succeed
    /// at all.
    fn dispose_workspace(
        &self,
        session: &sessionmgr_core::Session,
        disposition: Option<Disposition>,
    ) -> Result<()> {
        let Some(workspace) = session.workspace.as_ref() else {
            return Ok(());
        };
        if !workspace.owns_worktree() {
            // A same-directory session's "workspace" is the user's own
            // repository. There is nothing here this tool created and
            // nothing it may remove.
            return Ok(());
        }
        let Some(disposition) = disposition else {
            // A bare `close` stops the processes and leaves the worktree
            // and branch in place. Work is not thrown away on an
            // ambiguous instruction.
            return Ok(());
        };
        let git = SystemGit;
        let branch = workspace.branch.clone().unwrap_or_default();

        if disposition == Disposition::Merge {
            // Merge first, and propagate a failure. A fast-forward-only
            // merge that fails means the branch has diverged -- and
            // removing the worktree anyway would destroy exactly the work
            // that could not be merged.
            git.merge_fast_forward_only(&workspace.repo, &branch)
                .map_err(|e| {
                    Error::conflict(format!(
                        "{e}\nThe session's worktree and branch have been left in place. \
                         Merge `{branch}` by hand, or close with --discard to throw it away."
                    ))
                })?;
        }

        let force = disposition == Disposition::Discard;
        if let Err(e) = git.worktree_remove(&workspace.repo, &workspace.cwd, force) {
            return Err(Error::conflict(format!(
                "{e}\nSomething may still be holding a file open in {}.",
                workspace.cwd.display()
            )));
        }
        // The branch outlives the worktree unless it was merged (nothing
        // left to lose) or explicitly discarded (the user said so).
        if let Err(e) = git.branch_delete(&workspace.repo, &branch, force) {
            // Not fatal: the worktree is gone, which is the part that
            // matters, and a branch left behind is recoverable by hand
            // whereas failing the whole close here is not.
            eprintln!("sessionmgr daemon: could not delete branch {branch}: {e}");
        }
        Ok(())
    }

    fn session_list(&self) -> Result<Response> {
        let mut sessions = Vec::new();
        for session in catalog::list_sessions(&self.root)? {
            // Reconciled on every list, not only at startup: a worker can
            // die at any moment, and a status this tool reports as
            // `Running` when the process is gone is worse than useless.
            sessions.push(catalog::summarize(&catalog::reconcile(&self.root, session)?));
        }
        Ok(Response::Sessions { sessions })
    }

    async fn session_input(&self, id: SessionId, data: Vec<u8>) -> Result<Response> {
        let session = catalog::read_session(&self.root, &id)?;
        if !session.status.expects_live_worker() {
            return Err(Error::conflict(format!(
                "session {id} is {:?} and cannot accept input",
                session.status
            )));
        }
        let mut conn = transport::Connection::connect(
            "connecting to a worker",
            &paths::worker_socket(&self.root, &id),
        )
        .await?;
        conn.request(&Request::SessionInput { id, data }).await
    }

    /// Forwards a terminal resize to the session's worker.
    async fn session_resize(&self, id: SessionId, rows: u16, cols: u16) -> Result<Response> {
        let session = catalog::read_session(&self.root, &id)?;
        if !session.status.expects_live_worker() {
            // Not an error: a UI resizing every session it displays
            // should not have to filter out the finished ones first.
            return Ok(Response::Ok);
        }
        let mut conn = transport::Connection::connect(
            "connecting to a worker",
            &paths::worker_socket(&self.root, &id),
        )
        .await?;
        conn.request(&Request::SessionResize { id, rows, cols }).await
    }

    /// Graceful first, then force, then record.
    ///
    /// The ordering is deliberate: processes are terminated **before**
    /// the record is written, because that is the one window where the
    /// daemon and a worker could otherwise both write `state.json`. Once
    /// the pids are gone there is provably no other writer.
    async fn session_close(
        &self,
        id: SessionId,
        disposition: Option<Disposition>,
    ) -> Result<Response> {
        let mut session = catalog::read_session(&self.root, &id)?;
        if session.status.is_terminal() {
            return Err(Error::conflict(format!("session {id} is already closed")));
        }

        // 1. Ask nicely. A worker that acks shuts its own child down and
        //    exits, which is cleaner than anything done from outside.
        let socket = paths::worker_socket(&self.root, &id);
        let graceful = rusty_tokio::time::timeout(GRACEFUL_CLOSE_TIMEOUT, async {
            let mut conn = transport::Connection::connect("connecting to a worker", &socket).await?;
            let response: Response = conn.request(&Request::WorkerShutdown).await?;
            Ok::<_, Error>(response)
        })
        .await;
        if !matches!(graceful, Ok(Ok(Response::Ok))) {
            // Not an error worth failing the close over -- a worker that
            // already exited, or is wedged, is exactly why the forced
            // path below exists.
            eprintln!("sessionmgr daemon: session {id} did not acknowledge a graceful shutdown");
        }

        // 2. Terminate whatever is left, worker **and** child. Killing
        //    only the worker would leave its child running as an orphan
        //    with nothing tracking it and no way for the user to reach
        //    it. This is why both pids are recorded.
        for pid in sessionmgr_core::recovery::teardown_pids(&session) {
            if let Err(e) = sessionmgr_proc::terminate(pid) {
                eprintln!("sessionmgr daemon: could not terminate pid {pid}: {e}");
            }
        }

        // 3. Only once nothing is running: dispose of the worktree. A
        //    live process holding a file open inside it would make the
        //    removal fail, and on Windows that is not advisory.
        self.dispose_workspace(&session, disposition)?;

        // 4. Now, with no other possible writer, record the outcome.
        session.transition_to(session.teardown_status(disposition))?;
        catalog::write_session(&self.root, &session)?;
        Ok(Response::Ok)
    }
}

/// Serves one client connection.
async fn serve(supervisor: Arc<Supervisor>, mut conn: transport::Connection) {
    let request: Option<Request> = match conn.read().await {
        Ok(request) => request,
        Err(e) => {
            let _ = conn
                .write(&Response::Error {
                    kind: e.kind(),
                    message: e.to_string(),
                })
                .await;
            return;
        }
    };
    let Some(request) = request else { return };

    // Attach owns its connection for the rest of its life, so it is
    // dispatched before the ordinary request/response path.
    if let Request::SessionAttach { id } = request {
        proxy_attach(supervisor, conn, id).await;
        return;
    }

    let response = match supervisor.handle(request).await {
        Ok(response) => response,
        Err(e) => Response::Error {
            kind: e.kind(),
            message: e.to_string(),
        },
    };
    let _ = conn.write(&response).await;
}

/// Bridges an attached client to the session's worker.
///
/// The daemon proxies rather than handing the client the worker's socket
/// path, which keeps the worker socket genuinely private and leaves the
/// daemon as the only thing a client needs to know how to reach.
///
/// Forwarded as typed messages rather than a raw byte copy, deliberately:
/// the client's first request has already been read through a buffered
/// reader, so a byte-level splice would silently drop anything that
/// arrived in the same packet.
async fn proxy_attach(supervisor: Arc<Supervisor>, client: transport::Connection, id: SessionId) {
    let session = match catalog::read_session(&supervisor.root, &id) {
        Ok(session) => session,
        Err(e) => {
            let (_, mut writer) = client.into_parts();
            let _ = transport::write_framed(
                &mut writer,
                &Response::Error {
                    kind: e.kind(),
                    message: e.to_string(),
                },
            )
            .await;
            return;
        }
    };

    let recovered = matches!(catalog::recovery_for(&session), RecoveryAction::Adopt)
        && session.status == SessionStatus::Running;

    let worker_conn = match transport::Connection::connect(
        "connecting to a worker",
        &paths::worker_socket(&supervisor.root, &id),
    )
    .await
    {
        Ok(conn) => conn,
        Err(_) => {
            // No live worker: replay what the session did produce, so a
            // crashed or finished session is still readable rather than
            // just refusing to open.
            let (_, mut writer) = client.into_parts();
            if let Ok(history) = catalog::read_transcript(&supervisor.root, &id) {
                for event in history {
                    if transport::write_framed(&mut writer, &event).await.is_err() {
                        return;
                    }
                }
            }
            let _ = transport::write_framed(
                &mut writer,
                &SessionEvent::Status {
                    status: session.status,
                },
            )
            .await;
            return;
        }
    };

    let (mut client_reader, mut client_writer) = client.into_parts();
    let (mut worker_reader, mut worker_writer) = worker_conn.into_parts();

    if transport::write_framed(&mut worker_writer, &Request::SessionAttach { id })
        .await
        .is_err()
    {
        return;
    }

    if recovered {
        // Said out loud rather than left for the user to infer from a gap
        // in the output: this session survived the manager closing, which
        // is the entire promise of the architecture.
        if transport::write_framed(&mut client_writer, &SessionEvent::RecoveryMarker)
            .await
            .is_err()
        {
            return;
        }
    }

    // Client -> worker (input).
    let inbound = rusty_tokio::spawn(async move {
        while let Ok(Some(request)) = transport::read_framed::<Request>(&mut client_reader).await {
            if transport::write_framed(&mut worker_writer, &request)
                .await
                .is_err()
            {
                return;
            }
        }
    });

    // Worker -> client (output events).
    while let Ok(Some(event)) = transport::read_framed::<SessionEvent>(&mut worker_reader).await {
        if transport::write_framed(&mut client_writer, &event)
            .await
            .is_err()
        {
            break;
        }
    }
    inbound.abort();
}

/// The platform's interactive shell, for a `PlainTerminal` session.
///
/// Resolved by the daemon rather than by each client, so every client
/// role agrees about what "a terminal" means, and honours the user's own
/// configured shell rather than imposing one.
pub fn default_shell() -> Vec<String> {
    #[cfg(windows)]
    {
        // `COMSPEC` is how Windows itself names the command processor;
        // falling back to a bare `cmd.exe` lets `PATH` resolve it.
        let shell = std::env::var("COMSPEC").unwrap_or_else(|_| "cmd.exe".to_owned());
        vec![shell]
    }
    #[cfg(unix)]
    {
        let shell = std::env::var("SHELL").unwrap_or_else(|_| "/bin/sh".to_owned());
        vec![shell]
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_default_shell_is_never_empty() {
        // `session_new` splits this into program + args and fails on an
        // empty command, so an empty result here would be a session that
        // can never start.
        assert!(!default_shell().is_empty());
        assert!(!default_shell()[0].is_empty());
    }

    #[test]
    fn a_missing_daemon_pointer_means_no_running_daemon() {
        let dir = std::env::temp_dir().join("sessionmgr-no-daemon-here");
        assert!(running_daemon(&dir).is_none());
    }

    #[test]
    fn a_stale_daemon_pointer_does_not_read_as_a_running_daemon() {
        // Otherwise a crashed daemon whose pid was later recycled would
        // make every client refuse to start a new one, permanently.
        let dir = std::env::temp_dir().join(format!("sessionmgr-stale-{}", std::process::id()));
        std::fs::create_dir_all(&dir).expect("mkdir");
        std::fs::write(
            paths::daemon_state(&dir),
            serde_json::to_string(&DaemonState {
                pid: std::process::id(),
                start_fingerprint: Some("not-this-processes-real-fingerprint".to_owned()),
            })
            .expect("encode"),
        )
        .expect("write");
        assert!(running_daemon(&dir).is_none());
        let _ = std::fs::remove_dir_all(&dir);
    }
}
