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

use rusty_tokio::sync::{Mutex, Notify};
use serde::{Deserialize, Serialize};
use sessionmgr_core::ports::GitPort;
use sessionmgr_core::{
    Disposition, ParentReadiness, RecoveryAction, Session, SessionId, SessionKind, SessionStatus,
    Workspace,
};
use sessionmgr_git::SystemGit;
use sessionmgr_protocol::{Request, Response, SessionEvent, SessionSummary};

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

/// How often the daemon re-checks a `Waiting` dependent session's
/// parent.
///
/// Matches the TUI's own session-list refresh cadence (`sessionmgr-tui`'s
/// `app.rs`), which is the shortest interval anything in this project
/// already treats as "prompt enough" -- there is no reason a parent's own
/// status becoming visible in `sessionmgr list` and a dependent session
/// noticing it should be tuned any tighter than that.
const DEPENDENT_POLL_INTERVAL: Duration = Duration::from_secs(2);

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
    /// Serializes a `Waiting` dependent session's two competing writers:
    /// the background poller ([`poll_parent_then_start`]) promoting it to
    /// `Running`, and a user closing it before that happens.
    ///
    /// Coarse-grained -- one lock for every waiting session in the
    /// daemon, not one per session -- deliberately: promoting a session
    /// out of `Waiting` is rare (it happens once, ever, per dependent
    /// session) and already involves spawning a worker and waiting for it
    /// to report in, which is a slower operation than acquiring this lock
    /// could ever meaningfully contend with. A per-session lock registry
    /// would be real complexity bought for a case that does not need it.
    ///
    /// Without this, [`Supervisor::session_close`] could read a `Waiting`
    /// session (no worker recorded), the poller could win a race and
    /// spawn a real worker for it, and `session_close`'s own stale
    /// in-memory copy would then overwrite `state.json` with `Closed` and
    /// no worker/child pids -- leaking the freshly spawned process with
    /// nothing left tracking it. Both sides take this lock before acting
    /// on a `Waiting` session, so "is it still Waiting?" and "act on it"
    /// happen atomically with respect to each other.
    dependent_lock: Mutex<()>,
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
    let exe =
        std::env::current_exe().map_err(|e| Error::io("locating this executable", None, e))?;

    let supervisor = Arc::new(Supervisor {
        root: root.clone(),
        exe,
        shutdown: Notify::new(),
        dependent_lock: Mutex::new(()),
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
    let still_waiting = supervisor.reconcile_all()?;

    // A `Waiting` dependent session has no worker, so `reconcile_all`'s
    // ordinary adopt/crash pass above never touches it -- its only
    // "liveness" is an in-memory poller task, which does not survive a
    // daemon restart. Restart one here for anything still found
    // `Waiting`, or a session created just before the daemon died would
    // wait forever with nothing ever checking its parent again.
    for id in still_waiting {
        rusty_tokio::spawn(poll_parent_then_start(Arc::clone(&supervisor), id));
    }

    let listener =
        transport::Listener::bind("binding the daemon socket", &paths::daemon_socket(&root))?;

    // Written after the bind succeeds, never before: a pointer file
    // advertising a daemon that then failed to bind would send every
    // client to a socket that does not exist.
    let me = std::process::id();
    let state = DaemonState {
        pid: me,
        start_fingerprint: sessionmgr_proc::start_fingerprint(me).ok().flatten(),
    };
    let state_path = paths::daemon_state(&root);
    std::fs::write(&state_path, serde_json::to_string_pretty(&state)?)
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
    /// Applies the recovery rule to every session on disk, and returns
    /// the ids of every session found still [`SessionStatus::Waiting`] --
    /// which `recovery_for` deliberately leaves untouched (see
    /// `SessionStatus::expects_live_worker`'s own docs) but which still
    /// needs its poller task restarted, since that task lived only in the
    /// previous daemon process's memory.
    ///
    /// This runs once at startup and is the moment the whole persistence
    /// design either works or does not: sessions whose workers survived
    /// this daemon's predecessor are adopted, and the rest are honestly
    /// marked crashed.
    fn reconcile_all(&self) -> Result<Vec<SessionId>> {
        let mut adopted = 0usize;
        let mut crashed = 0usize;
        let mut waiting = Vec::new();
        for session in catalog::list_sessions(&self.root)? {
            match catalog::recovery_for(&session) {
                RecoveryAction::Adopt => adopted += 1,
                RecoveryAction::MarkCrashed => crashed += 1,
                RecoveryAction::LeaveAsIs => {
                    if session.status == SessionStatus::Waiting {
                        waiting.push(session.id.clone());
                    }
                }
            }
            catalog::reconcile(&self.root, session)?;
        }
        if adopted > 0 || crashed > 0 {
            eprintln!(
                "sessionmgr daemon: adopted {adopted} running session(s), \
                 marked {crashed} crashed"
            );
        }
        Ok(waiting)
    }

    async fn handle(self: &Arc<Self>, request: Request) -> Result<Response> {
        match request {
            Request::Ping => Ok(Response::Pong {
                pid: std::process::id(),
            }),
            Request::SessionNew {
                kind,
                command,
                repo,
                pty,
                agent,
                hooks,
                parent,
                wait_for_parent,
            } => {
                self.session_new(
                    kind,
                    command,
                    repo,
                    pty,
                    agent,
                    hooks,
                    parent,
                    wait_for_parent,
                )
                .await
            }
            Request::SessionList => self.session_list().await,
            Request::SessionInput { id, data } => self.session_input(id, data).await,
            Request::SessionResize { id, rows, cols } => self.session_resize(id, rows, cols).await,
            Request::SessionClose { id, disposition } => self.session_close(id, disposition).await,
            Request::SessionRename { id, name } => self.session_rename(id, name).await,
            Request::SessionStartNow { id } => self.session_start_now(id).await,
            Request::SessionFork { id, pty } => self.session_fork(id, pty).await,
            Request::GitStatus { id } => self.session_git_status(id).await,
            Request::GitDiff { id, path } => self.session_git_diff(id, path).await,
            Request::HookFire { session_id, event } => self.hook_fire(session_id, event).await,
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

    #[allow(clippy::too_many_arguments)]
    async fn session_new(
        self: &Arc<Self>,
        kind: SessionKind,
        command: Vec<String>,
        repo: Option<PathBuf>,
        pty: bool,
        agent: Option<sessionmgr_core::AgentKind>,
        hooks: bool,
        parent: Option<SessionId>,
        wait_for_parent: bool,
    ) -> Result<Response> {
        // `parent` and `kind` must agree with each other -- checked here,
        // on the daemon side, rather than trusted from the client: the
        // public socket is the actual trust boundary, and the CLI layer's
        // own `--parent`/`--kind` mutual-exclusion check is a courtesy to
        // the user, not a security control.
        if parent.is_some() != (kind == SessionKind::Dependent) {
            return Err(Error::usage(
                "a dependent session needs --parent together with a dependent kind, \
                 and nothing else may set a dependent kind",
            ));
        }
        if kind == SessionKind::Dependent && repo.is_some() {
            return Err(Error::usage(
                "--repo is meaningless for a dependent session; its workspace comes \
                 from --parent's own worktree",
            ));
        }
        // A session whose adapter supports Fork gets its own native id
        // pinned up front, unconditionally -- not only when the caller
        // asks for it -- so that *any* Claude Code session this daemon
        // creates is already forkable later with no extra machinery. See
        // `AgentAdapterPort::supports_fork`'s own docs for why this is a
        // creation-time decision `fork_args` alone cannot make.
        let native_session_id = match agent {
            Some(kind) if sessionmgr_agents::adapter_for(kind).supports_fork() => Some(
                sessionmgr_proc::native_session_uuid()
                    .map_err(|e| Error::io("generating a native session id", None, e))?,
            ),
            _ => None,
        };
        // An agent's own `launch_args` decides the real command line --
        // `command` becomes its `extra` (an initial prompt, typically),
        // not the literal program to run. Without an agent, behaviour is
        // exactly what it always was: the command as given, or the
        // platform's default shell if none.
        let command = match agent {
            Some(kind) => sessionmgr_agents::adapter_for(kind).launch_args(
                &command,
                hooks,
                native_session_id.as_deref(),
            ),
            None if command.is_empty() => default_shell(),
            None => command,
        };
        let id = sessionmgr_proc::session_id()
            .map_err(|e| Error::io("generating a session id", None, e))?;

        let workspace = match &parent {
            Some(parent_id) => Some(self.resolve_dependent_workspace(parent_id).await?),
            None => self.prepare_workspace(kind, repo, &id).await?,
        };
        // Installed *before* the session record is written and the
        // worker spawned, so the hook config exists on disk by the time
        // the agent CLI actually starts and reads it. Not offered for a
        // dependent session: it shares its parent's own worktree, so any
        // hook config the parent installed with its own `--hooks`
        // already applies -- installing a second copy on top would be at
        // best redundant and at worst a second, conflicting settings
        // file in the same directory.
        if hooks {
            let Some(agent) = agent else {
                return Err(Error::usage("--hooks needs --agent <claude|codex>"));
            };
            if kind == SessionKind::Dependent {
                return Err(Error::usage(
                    "--hooks is inherited from the parent's own worktree for a dependent \
                     session; pass --hooks on the parent session instead",
                ));
            }
            let Some(workspace) = workspace.as_ref() else {
                return Err(Error::usage(
                    "--hooks needs an isolated worktree session (--kind worktree)",
                ));
            };
            let workspace_cwd = workspace.cwd.clone();
            let install_id = id.clone();
            rusty_tokio::spawn_blocking(move || {
                crate::hooks::install::install(kind, &workspace_cwd, agent, &install_id)
            })
            .await
            .map_err(|e| {
                Error::conflict(format!("the hook-install task did not complete: {e}"))
            })??;
        }
        let mut session = Session::new(
            id.clone(),
            kind,
            command,
            workspace,
            pty,
            sessionmgr_proc::now_millis(),
            agent,
            parent.clone(),
            wait_for_parent,
            native_session_id,
            None,
        );

        // A dependent session asked to wait, whose parent is not ready
        // yet, is published `Waiting` and gets **no worker at all** until
        // the parent is -- see `try_advance_waiting_session`. Every other
        // case (no parent, `--start-now`, or a parent that already
        // finished by the time this request arrived) starts immediately,
        // exactly as every session did before Phase 5.
        let parent_readiness = match &parent {
            Some(parent_id) if wait_for_parent => {
                let parent_session = catalog::read_session(&self.root, parent_id)?;
                Some(sessionmgr_core::parent_readiness(parent_session.status))
            }
            _ => None,
        };

        match parent_readiness {
            Some(ParentReadiness::NotYet) => {
                session.transition_to(SessionStatus::Waiting)?;
                catalog::write_session(&self.root, &session)?;
                rusty_tokio::spawn(poll_parent_then_start(Arc::clone(self), id.clone()));
                return Ok(Response::SessionCreated { id });
            }
            Some(ParentReadiness::Unavailable) => {
                return Err(Error::usage(
                    "the parent session's worktree no longer exists (it was merged or discarded)",
                ));
            }
            // `Ready`, or no parent at all (or `--start-now`): fall
            // through to the ordinary immediate-start path below.
            Some(ParentReadiness::Ready) | None => {}
        }

        // Written before the spawn, never after: if this process dies in
        // the window between the two, a record with no worker is
        // recoverable (it reconciles to `Crashed`), whereas a running
        // worker with no record on disk is unreachable garbage.
        catalog::write_session(&self.root, &session)?;
        self.spawn_and_await_running(&id).await?;
        Ok(Response::SessionCreated { id })
    }

    /// Resolves a dependent session's workspace from its parent, and
    /// rejects everything that would leave it with nowhere to run.
    ///
    /// Runs **before** the child session's own record is ever written, so
    /// a rejected parent leaves nothing behind at all -- the same
    /// principle [`Self::prepare_workspace`] already follows for a
    /// worktree that fails to create.
    async fn resolve_dependent_workspace(&self, parent_id: &SessionId) -> Result<Workspace> {
        let parent = catalog::read_session(&self.root, parent_id)
            .map_err(|_| Error::usage(format!("parent session {parent_id} does not exist")))?;
        if !matches!(parent.kind, SessionKind::Worktree | SessionKind::Dependent) {
            return Err(Error::usage(format!(
                "session {parent_id} is a {:?} session and has no worktree to depend on; \
                 a dependent session's parent must be --kind worktree (or itself dependent \
                 on one)",
                parent.kind
            )));
        }
        let parent_workspace = parent.workspace.ok_or_else(|| {
            Error::usage(format!("session {parent_id} has no workspace to depend on"))
        })?;
        // Caught early here as a clear creation-time rejection; the same
        // condition is also what `parent_readiness` calls `Unavailable`
        // for a session that is already `Waiting` when its parent's
        // worktree disappears later.
        if matches!(
            parent.status,
            SessionStatus::Merged | SessionStatus::Discarded
        ) {
            return Err(Error::usage(format!(
                "session {parent_id}'s worktree no longer exists (it was {:?})",
                parent.status
            )));
        }
        Ok(Workspace::dependent(&parent_workspace))
    }

    /// Spawns a worker for `id` -- whose record is currently `Created` or
    /// `Waiting` -- and waits until it reports something (`Running`, or
    /// an immediate exit).
    ///
    /// Factored out of `session_new`'s own original tail so
    /// [`try_advance_waiting_session`] and [`Self::session_start_now`]
    /// can promote a `Waiting` session through the exact same start path
    /// an ordinary session already goes through -- from the worker's own
    /// point of view there is no difference between the two; it just
    /// transitions whatever status the record currently holds to
    /// `Running`.
    async fn spawn_and_await_running(&self, id: &SessionId) -> Result<()> {
        let worker_pid = worker::spawn_detached(&self.exe, &self.root, id)?;

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
        // `Created`/`Waiting` whether it starts serving or exits first.
        // And the worker binds its socket *before* publishing `Running`,
        // so a record past either also guarantees the socket exists for
        // an attach that follows immediately.
        let deadline = std::time::Instant::now() + WORKER_READY_TIMEOUT;
        loop {
            let status = catalog::read_session(&self.root, id)?.status;
            if status != SessionStatus::Created && status != SessionStatus::Waiting {
                return Ok(());
            }
            // The worker died without recording anything -- fail with a
            // pointer to the only place its reason was written.
            if !sessionmgr_proc::is_alive(worker_pid).unwrap_or(false) {
                return Err(Error::conflict(format!(
                    "the worker for session {id} exited before starting it; see {}",
                    paths::worker_log(&self.root, id).display()
                )));
            }
            if std::time::Instant::now() >= deadline {
                return Err(Error::conflict(format!(
                    "the worker for session {id} did not start it in time; see {}",
                    paths::worker_log(&self.root, id).display()
                )));
            }
            rusty_tokio::time::sleep(Duration::from_millis(25)).await;
        }
    }

    /// The CAPABILITIES.md "start now" override, applied to a session
    /// that is already `Waiting` rather than at creation time.
    async fn session_start_now(&self, id: SessionId) -> Result<Response> {
        if !self.try_advance_waiting_session(&id, true).await? {
            return Err(Error::conflict(format!(
                "session {id} is not currently waiting on a parent"
            )));
        }
        Ok(Response::Ok)
    }

    /// CAPABILITIES.md's "Fork session": clones `source_id`'s own
    /// conversation into a brand-new, independent session.
    ///
    /// See `docs/decisions/0003-resume-fork-spike.md` for how any of this
    /// is possible at all, and `docs/phase-6-report.md` for the full
    /// design -- in short, every check below exists because Fork
    /// genuinely needs all of these things to be true, not because this
    /// method is being defensive for its own sake:
    ///
    /// - `source` must be `Worktree`-kind: only a session that owns a
    ///   branch has code state for the fork to start from.
    /// - `source.agent` must be set, and that agent's own adapter must
    ///   answer `true` to `supports_fork()` -- as of this phase, only
    ///   Claude Code.
    /// - `source.native_session_id` must be recorded -- absent for a
    ///   session created before Fork existed, or whose adapter did not
    ///   support pinning one at the time.
    /// - `source`'s own branch must still exist -- reuses
    ///   `sessionmgr_core::parent_readiness` (Phase 5), the exact same
    ///   "does this session's git state still exist" question a
    ///   dependent session's wait-for-parent already asks, just applied
    ///   to a different relationship.
    async fn session_fork(self: &Arc<Self>, source_id: SessionId, pty: bool) -> Result<Response> {
        let source = catalog::read_session(&self.root, &source_id)?;
        if source.kind != SessionKind::Worktree {
            return Err(Error::usage(format!(
                "session {source_id} is a {:?} session; only a worktree session's branch \
                 can be forked",
                source.kind
            )));
        }
        let Some(agent) = source.agent else {
            return Err(Error::usage(format!(
                "session {source_id} has no agent CLI conversation to fork"
            )));
        };
        let adapter = sessionmgr_agents::adapter_for(agent);
        if !adapter.supports_fork() {
            return Err(Error::usage(format!(
                "{agent:?} does not support Fork yet -- see docs/phase-6-report.md for \
                 which agents do and why"
            )));
        }
        let Some(source_native_id) = source.native_session_id.clone() else {
            return Err(Error::conflict(format!(
                "session {source_id} has no recorded native session id to fork from \
                 (sessions created before Fork support existed cannot be forked)"
            )));
        };
        let Some(source_workspace) = source.workspace.clone() else {
            return Err(Error::conflict(format!(
                "session {source_id} has no workspace to fork from"
            )));
        };
        let Some(source_branch) = source_workspace.branch.clone() else {
            return Err(Error::conflict(format!(
                "session {source_id} owns no branch to fork from"
            )));
        };
        if matches!(
            sessionmgr_core::parent_readiness(source.status),
            ParentReadiness::Unavailable
        ) {
            return Err(Error::usage(format!(
                "session {source_id}'s branch no longer exists (it was merged or discarded)"
            )));
        }

        let id = sessionmgr_proc::session_id()
            .map_err(|e| Error::io("generating a session id", None, e))?;
        let new_native_id = sessionmgr_proc::native_session_uuid()
            .map_err(|e| Error::io("generating a native session id", None, e))?;
        // Checked again here, not just trusted from the `supports_fork`
        // check above: that check is a static fact about the adapter,
        // this call is the actual, adapter-specific translation of
        // `(source_native_id, new_native_id)` into a real command line,
        // and `AgentAdapterPort`'s own drift-guard test
        // (`sessionmgr-agents`) is what keeps the two from disagreeing in
        // practice rather than this call site having to.
        let Some(command) = adapter.fork_args(&source_native_id, &new_native_id, &[]) else {
            return Err(Error::conflict(format!("{agent:?} does not support Fork")));
        };

        // A new, independent worktree, branched from the **source**
        // session's own branch tip rather than the repository's default
        // branch -- see `GitPort::worktree_add`'s own docs for why this
        // is load-bearing, not cosmetic.
        let workspace = self
            .fork_workspace(source_workspace.repo.clone(), id.clone(), source_branch)
            .await?;

        let session = Session::new(
            id.clone(),
            SessionKind::Worktree,
            command,
            Some(workspace),
            pty,
            sessionmgr_proc::now_millis(),
            Some(agent),
            None,
            false,
            Some(new_native_id),
            Some(source_id),
        );
        catalog::write_session(&self.root, &session)?;
        self.spawn_and_await_running(&id).await?;
        Ok(Response::SessionCreated { id })
    }

    /// Creates a forked session's new worktree, branched from
    /// `start_point` (the source session's own branch) rather than
    /// git's own default. Same `spawn_blocking` reasoning as
    /// [`Self::prepare_workspace`]: `git worktree add` is a real,
    /// synchronous subprocess call.
    async fn fork_workspace(
        &self,
        repo: PathBuf,
        id: SessionId,
        start_point: String,
    ) -> Result<Workspace> {
        rusty_tokio::spawn_blocking(move || -> Result<Workspace> {
            let git = SystemGit;
            let workspace = Workspace::worktree(repo.clone(), &id);
            let branch = workspace.branch.clone().unwrap_or_default();
            git.worktree_add(&repo, &workspace.cwd, &branch, Some(&start_point))
                .map_err(|e| Error::conflict(e.to_string()))?;
            Ok(workspace)
        })
        .await
        .map_err(|e| Error::conflict(format!("the fork-workspace task did not complete: {e}")))?
    }

    /// The core of the wait-for-parent mechanism: if `id` is currently
    /// `Waiting`, decides whether it can start yet and, if so, starts (or
    /// fails) it. Returns whether `id` was `Waiting` at all -- the
    /// poller uses `false` to mean "keep polling" and `true` to mean
    /// "done, stop"; [`Self::session_start_now`] uses it to report
    /// "there was nothing to start now".
    ///
    /// `force`, when true, skips the parent-readiness check for the
    /// `NotYet` case and starts immediately regardless -- the "start now"
    /// override. It never skips the `Unavailable` check: forcing a start
    /// into a worktree that no longer exists would just move today's
    /// clear error to whatever `git`/the shell reports for a missing
    /// directory instead.
    ///
    /// Takes [`Supervisor::dependent_lock`] for its entire body: see that
    /// field's own docs for the race this closes.
    async fn try_advance_waiting_session(&self, id: &SessionId, force: bool) -> Result<bool> {
        let _guard = self.dependent_lock.lock().await;
        let session = catalog::read_session(&self.root, id)?;
        if session.status != SessionStatus::Waiting {
            return Ok(false);
        }
        let Some(parent_id) = session.parent_id.clone() else {
            // Should be unreachable -- nothing else ever writes
            // `Waiting` -- but a `Waiting` session with no parent to wait
            // for cannot resolve itself, so fail it rather than spin
            // forever.
            self.fail_waiting_session(id, "has no recorded parent to wait for")
                .await?;
            return Ok(true);
        };
        let readiness = match catalog::read_session(&self.root, &parent_id) {
            Ok(parent) => sessionmgr_core::parent_readiness(parent.status),
            // The parent record itself is unreadable (should not
            // normally happen -- sessions are never deleted, only torn
            // down in place). Handled rather than assumed: propagating a
            // read error here would leave the session stuck `Waiting`
            // forever with nothing retrying, since the poller's own
            // caller treats any `Err` as "try again later" (see
            // `poll_parent_then_start`).
            Err(_) => {
                self.fail_waiting_session(id, "its parent session record could not be read")
                    .await?;
                return Ok(true);
            }
        };
        match readiness {
            ParentReadiness::NotYet if !force => Ok(false),
            ParentReadiness::NotYet | ParentReadiness::Ready => {
                self.spawn_and_await_running(id).await?;
                Ok(true)
            }
            ParentReadiness::Unavailable => {
                self.fail_waiting_session(
                    id,
                    "its parent session's worktree was merged or discarded before \
                     this session could start",
                )
                .await?;
                Ok(true)
            }
        }
    }

    /// Fails a `Waiting` session directly to `Errored`, with no worker
    /// ever having been spawned for it.
    async fn fail_waiting_session(&self, id: &SessionId, reason: &str) -> Result<()> {
        let mut session = catalog::read_session(&self.root, id)?;
        if session.transition_to(SessionStatus::Errored).is_ok() {
            eprintln!("sessionmgr daemon: session {id} could not start: {reason}");
            catalog::write_session(&self.root, &session)?;
            // Matches `Worker::record_child_exit`'s own notification for
            // an ordinary `Errored` exit -- from a webhook consumer's
            // point of view, a dependent session that never got to start
            // is the same kind of bad news.
            crate::hooks::dispatch::notify(&session, "errored");
        }
        Ok(())
    }

    /// Resolves the repository and creates the worktree, if the session's
    /// kind calls for one.
    ///
    /// Done **before** the session record is written, so a failure here
    /// leaves nothing behind at all. Writing the record first would leave
    /// a session pointing at a worktree that does not exist -- visible in
    /// `list`, impossible to attach to, and needing its own cleanup path.
    ///
    /// `git worktree add` is a synchronous subprocess call -- real disk
    /// I/O checking out a full working copy, with an antivirus scanner
    /// free to sit in the path (PLAN.md risk 7). Run inline on an async
    /// task, it is the same "real OS work on the executor" bug as
    /// `session_list` (see its own comment), except slower and more
    /// variable, which makes it the more likely trigger for issue #2's
    /// hang under any concurrent `new`/`close` load. `spawn_blocking`
    /// moves it to the blocking pool.
    async fn prepare_workspace(
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
        let id = id.clone();
        rusty_tokio::spawn_blocking(move || -> Result<Option<Workspace>> {
            let git = SystemGit;
            // Resolved from the client's directory to a repository root,
            // so a session created deep inside a repo lands in the same
            // place as one created at the top.
            let root = git
                .repo_root(&from)
                .map_err(|e| Error::usage(e.to_string()))?;

            match kind {
                SessionKind::SameDirectory => Ok(Some(Workspace::same_directory(root))),
                SessionKind::Worktree => {
                    let workspace = Workspace::worktree(root.clone(), &id);
                    let branch = workspace.branch.clone().unwrap_or_default();
                    git.worktree_add(&root, &workspace.cwd, &branch, None)
                        .map_err(|e| Error::conflict(e.to_string()))?;
                    Ok(Some(workspace))
                }
                // Both unreachable in practice: `needs_repo()` is `false`
                // for both, so the early return above already sends
                // every `PlainTerminal`/`Dependent` call here home before
                // this match ever runs. Kept exhaustive (rather than a
                // wildcard) so a future kind cannot silently fall through
                // this match with the wrong behaviour.
                SessionKind::PlainTerminal | SessionKind::Dependent => Ok(None),
            }
        })
        .await
        .map_err(|e| Error::conflict(format!("the workspace-setup task did not complete: {e}")))?
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
    ///
    /// Same `spawn_blocking` reasoning as [`Self::prepare_workspace`]:
    /// `git worktree remove`/`branch -d`/a fast-forward merge are
    /// synchronous subprocess calls doing real file I/O, not fit for the
    /// async executor.
    async fn dispose_workspace(
        &self,
        session: &sessionmgr_core::Session,
        disposition: Option<Disposition>,
    ) -> Result<()> {
        let Some(workspace) = session.workspace.clone() else {
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
        rusty_tokio::spawn_blocking(move || -> Result<()> {
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
        })
        .await
        .map_err(|e| {
            Error::conflict(format!("the workspace-teardown task did not complete: {e}"))
        })?
    }

    /// Filesystem reads and pid-liveness probes here are synchronous --
    /// on Linux, `reconcile` reads `/proc/<pid>/stat`; on macOS/BSD it
    /// shells out to `ps` and waits for it to exit. Doing that inline on
    /// an async task ties up one of `rusty_tokio`'s fixed worker threads
    /// for as long as the syscall (or subprocess) takes, and on a
    /// small CI runner (2-4 workers, per `#[rusty_tokio::main]`'s
    /// default multi-threaded flavor) enough concurrent `list` calls can
    /// starve every worker at once -- indistinguishable from the daemon
    /// hanging to anything else trying to connect. Issue #2's daemon
    /// hang on Linux CI is this class of bug, same as the bind-before-
    /// recover race already fixed: real OS work belongs on the blocking
    /// pool, not on the executor. `spawn_blocking` moves it there.
    async fn session_list(&self) -> Result<Response> {
        let root = self.root.clone();
        let sessions = rusty_tokio::spawn_blocking(move || -> Result<Vec<SessionSummary>> {
            let mut sessions = Vec::new();
            for session in catalog::list_sessions(&root)? {
                sessions.push(catalog::summarize(&catalog::reconcile(&root, session)?));
            }
            Ok(sessions)
        })
        .await
        .map_err(|e| Error::conflict(format!("the session-list task did not complete: {e}")))??;
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
        conn.request(&Request::SessionResize { id, rows, cols })
            .await
    }

    /// Forwards a hook event to the named session's worker.
    ///
    /// **Always answers `Response::Ok`, never an error** -- this is
    /// PLAN.md's own requirement made concrete: a hook this tool
    /// installs only ever fires for a session it created, but nothing
    /// upstream of this method has fully proven that (the public socket
    /// accepts `HookFire` from anything that can connect to it, not
    /// only `__hook-fire`), so every failure mode here -- unparseable
    /// id, unknown session, no live worker, the worker itself refusing
    /// to answer -- collapses to the same silent no-op rather than
    /// surfacing an error into the invoking CLI's own transcript.
    async fn hook_fire(&self, session_id: String, event: String) -> Result<Response> {
        let Ok(id) = session_id.parse::<SessionId>() else {
            return Ok(Response::Ok);
        };
        let Ok(session) = catalog::read_session(&self.root, &id) else {
            return Ok(Response::Ok);
        };
        if !session.status.expects_live_worker() {
            return Ok(Response::Ok);
        }
        let Ok(mut conn) = transport::Connection::connect(
            "connecting to a worker",
            &paths::worker_socket(&self.root, &id),
        )
        .await
        else {
            return Ok(Response::Ok);
        };
        let _ = conn
            .request::<_, Response>(&Request::HookFire {
                session_id: id.to_string(),
                event,
            })
            .await;
        Ok(Response::Ok)
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

        // A `Waiting` dependent session has no worker at all yet -- see
        // `SessionStatus::Waiting`'s own docs -- so closing it is a pure
        // record update, not the graceful-then-forced teardown below.
        // Guarded by `dependent_lock` and re-read under it, because the
        // background poller (`poll_parent_then_start`) is racing to
        // promote this exact session concurrently: see
        // `Supervisor::dependent_lock`'s own docs for what goes wrong
        // without this.
        if session.status == SessionStatus::Waiting {
            let _guard = self.dependent_lock.lock().await;
            session = catalog::read_session(&self.root, &id)?;
            if session.status == SessionStatus::Waiting {
                // Nothing was ever spawned: nothing to terminate, and
                // (per `Workspace::dependent`'s `branch: None`) nothing
                // for `dispose_workspace` to remove either -- it is a
                // no-op below regardless, but skipping the graceful
                // WorkerShutdown attempt against a socket that was never
                // bound avoids paying its timeout for no reason.
                session.transition_to(session.teardown_status(disposition))?;
                catalog::write_session(&self.root, &session)?;
                return Ok(Response::Ok);
            }
            // The poller won the race: `session` is now a fresh, real
            // record with a worker to close. Fall through to the
            // ordinary path below using it.
        }

        // 1. Ask nicely. A worker that acks shuts its own child down and
        //    exits, which is cleaner than anything done from outside.
        let socket = paths::worker_socket(&self.root, &id);
        let graceful = rusty_tokio::time::timeout(GRACEFUL_CLOSE_TIMEOUT, async {
            let mut conn =
                transport::Connection::connect("connecting to a worker", &socket).await?;
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
        self.dispose_workspace(&session, disposition).await?;

        // 4. Now, with no other possible writer, record the outcome.
        session.transition_to(session.teardown_status(disposition))?;
        catalog::write_session(&self.root, &session)?;
        Ok(Response::Ok)
    }

    /// Sets or clears a session's purely cosmetic display label. No
    /// state-machine transition and no live worker involved -- unlike
    /// every method above this touches only the on-disk record, so a
    /// finished/crashed/closed session can still be renamed.
    async fn session_rename(&self, id: SessionId, name: Option<String>) -> Result<Response> {
        let mut session = catalog::read_session(&self.root, &id)?;
        session.rename(name);
        catalog::write_session(&self.root, &session)?;
        Ok(Response::Ok)
    }

    /// The files changed in a session's workspace, for the TUI's diff
    /// pane.
    ///
    /// `git status` is a synchronous subprocess call, so it goes to
    /// `spawn_blocking` -- the same reasoning as `prepare_workspace`/
    /// `dispose_workspace` (see their own comments): real OS work does
    /// not belong inline on the async executor.
    async fn session_git_status(&self, id: SessionId) -> Result<Response> {
        let session = catalog::read_session(&self.root, &id)?;
        let Some(workspace) = session.workspace else {
            return Err(Error::conflict(format!(
                "session {id} has no workspace to read git status from"
            )));
        };
        let files = rusty_tokio::spawn_blocking(move || {
            SystemGit
                .changed_files(&workspace.cwd)
                .map_err(|e| Error::conflict(e.to_string()))
        })
        .await
        .map_err(|e| Error::conflict(format!("the git-status task did not complete: {e}")))??;
        Ok(Response::GitStatus { files })
    }

    /// A unified diff of a session's workspace, optionally narrowed to
    /// one file. Same `spawn_blocking` reasoning as
    /// [`Self::session_git_status`].
    async fn session_git_diff(&self, id: SessionId, path: Option<String>) -> Result<Response> {
        let session = catalog::read_session(&self.root, &id)?;
        let Some(workspace) = session.workspace else {
            return Err(Error::conflict(format!(
                "session {id} has no workspace to diff"
            )));
        };
        let diff = rusty_tokio::spawn_blocking(move || {
            SystemGit
                .diff(&workspace.cwd, path.as_deref())
                .map_err(|e| Error::conflict(e.to_string()))
        })
        .await
        .map_err(|e| Error::conflict(format!("the git-diff task did not complete: {e}")))??;
        Ok(Response::GitDiff { diff })
    }
}

/// Background task, one per `Waiting` dependent session: re-checks its
/// parent every [`DEPENDENT_POLL_INTERVAL`] until
/// [`Supervisor::try_advance_waiting_session`] reports it is done (either
/// started or failed).
///
/// Deliberately **daemon-owned, not worker-owned**: a session with
/// nothing spawned yet has no worker to survive the daemon's own
/// restart, so this is exactly the kind of readiness-gating work the
/// daemon already does elsewhere (`prepare_workspace`, hook install)
/// before a worker ever exists. If the daemon itself is killed while a
/// session is `Waiting`, this task simply stops -- and `run`'s own
/// `reconcile_all` restarts an equivalent one for it the next time any
/// `sessionmgr` command brings a daemon back up (see `run`'s own
/// comments). Nothing is lost either way: there is no process running
/// yet for a `Waiting` session, so there is nothing an unclean daemon
/// exit could have orphaned.
async fn poll_parent_then_start(supervisor: Arc<Supervisor>, id: SessionId) {
    loop {
        match supervisor.try_advance_waiting_session(&id, false).await {
            Ok(true) => return,
            Ok(false) => {}
            Err(e) => {
                eprintln!("sessionmgr daemon: checking session {id}'s parent failed: {e}");
            }
        }
        rusty_tokio::time::sleep(DEPENDENT_POLL_INTERVAL).await;
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
