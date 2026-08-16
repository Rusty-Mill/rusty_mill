//! The per-session worker: one detached OS process that owns one
//! session's process, its transcript, and its private socket.
//!
//! **This is the process that makes the product's central promise true.**
//! Closing the manager must not stop the work, so the thing actually
//! running the work cannot be a child of the manager's lifetime. The
//! daemon re-execs its own binary here with detach flags and then forgets
//! about it except as a pid on disk.
//!
//! # Two backends, and why both exist
//!
//! A session runs its process either on a real terminal
//! ([`Backend::Pty`], the default) or on plain pipes
//! ([`Backend::Piped`]).
//!
//! The PTY is the default because it is **required**: interactive agent
//! CLIs refuse to run without a terminal, which the Phase 1 spike
//! established by measurement (ADR-0002). Piped stdio cannot host the
//! product's actual workload.
//!
//! The piped path is nonetheless kept, reachable with `--no-pty`, for one
//! specific reason rather than as general hedging. The persistence
//! guarantee -- a session surviving the manager being killed -- is
//! **proven** on Windows for the piped path, by a test suite that runs
//! green. It is *unproven* for ConPTY: whether a ConPTY-attached child
//! survives an unclean worker crash is an open question that needs a
//! Windows machine to answer (ADR-0002, "Still open"). Deleting the
//! proven path to make room for the unproven one would be trading a
//! demonstrated guarantee for an assumed one.

use std::path::{Path, PathBuf};
use std::sync::Arc;

use rusty_tokio::io::AsyncReadExt;
use rusty_tokio::io::AsyncWriteExt;
use rusty_tokio::sync::{broadcast, Mutex, Notify};
use sessionmgr_core::ports::worker_ref;
use sessionmgr_core::{SessionId, SessionStatus};
use sessionmgr_proc::SystemProcessPort;
use sessionmgr_pty::{PtyOptions, PtySession, TerminalSize};
use sessionmgr_protocol::{ErrorKind, Request, Response, SessionEvent};

use crate::error::{Error, Result};
use crate::{catalog, paths, transport};

/// How many events a slow client may fall behind before it starts losing
/// them. Output is also on disk in the transcript, so a lagging client
/// loses nothing permanently -- it just sees a gap in the live stream.
const BROADCAST_CAPACITY: usize = 1024;

/// Read buffer size for session output.
const READ_BUFFER: usize = 8192;

/// Supervisor-side: launch a detached worker process for `id`.
///
/// The binary re-execs **itself** (`std::env::current_exe()`), which is
/// correct by construction and is the main reason this project ships one
/// binary with three roles rather than several binaries: there is no
/// version skew possible between a running daemon and the worker it just
/// spawned, because they are the same file.
///
/// No Job Object, no process group placement. See `sessionmgr-proc`'s
/// module docs -- kill-on-close would defeat the entire point of this
/// function.
pub fn spawn_detached(exe: &Path, root: &Path, id: &SessionId) -> Result<u32> {
    use rusty_tokio::process::{Command, Stdio};

    let session_dir = paths::session_dir(root, id);
    paths::ensure_dir("creating a session directory", &session_dir)?;

    let log_path = paths::worker_log(root, id);
    let log = std::fs::File::create(&log_path)
        .map_err(|e| Error::io("creating a worker log", log_path, e))?;

    let mut cmd = Command::new(exe);
    cmd.arg("__worker-main")
        .arg("--session-id")
        .arg(id.as_str())
        .arg("--state-root")
        .arg(root)
        // stdin/stdout to null and stderr to a log file: a worker that
        // panics before binding its socket would otherwise fail
        // completely silently, and the only symptom the user ever sees
        // would be the daemon timing out waiting for a socket.
        .stdin(Stdio::null())
        .stdout(Stdio::null())
        .stderr(Stdio::from(log));
    sessionmgr_proc::prepare_detached(&mut cmd);

    let mut child = cmd
        .spawn()
        .map_err(|e| Error::io("spawning a worker", exe.to_path_buf(), e))?;
    let pid = child.id();

    // Hand the `Child` to a fire-and-forget reaper rather than dropping
    // it. On Unix, `setsid` detaches the worker from this process's
    // *session* but does not reparent it: the kernel still considers it
    // this process's child until something calls `wait`. A worker that
    // dies while the daemon is still running would otherwise become a
    // zombie under the daemon -- and a zombie answers `kill(pid, 0)`
    // successfully, so crash detection would never fire for exactly the
    // sessions it matters most for. This has no bearing on detachment
    // (that is `setsid`'s doing, not `wait`'s), and if the daemon dies
    // first the worker is simply reparented to init, which reaps it the
    // ordinary way.
    rusty_tokio::spawn(async move {
        let _ = child.wait().await;
    });
    Ok(pid)
}

pub struct WorkerArgs {
    pub session_id: SessionId,
    pub state_root: PathBuf,
}

/// A just-started session process, before it has been wired to a
/// [`Worker`]. Exists only to carry the handles across that gap.
enum Started {
    Pty(Arc<PtySession>),
    Piped(rusty_tokio::process::Child),
}

/// How a session's process is attached to the world.
enum Backend {
    /// A real terminal. See the module docs.
    Pty(Arc<PtySession>),
    /// Plain pipes, holding the child's stdin so input can still be sent.
    Piped(Mutex<Option<rusty_tokio::process::ChildStdin>>),
}

/// Shared worker state, `Arc`ed across the accept loop, the output
/// reader, and the exit watcher.
struct Worker {
    root: PathBuf,
    id: SessionId,
    events: broadcast::Sender<SessionEvent>,
    backend: Backend,
    /// Set when the worker should exit: either the session's process
    /// finished or a shutdown was requested.
    shutdown: Notify,
    child_pid: u32,
}

/// The `__worker-main` entrypoint.
pub async fn run(args: WorkerArgs) -> Result<()> {
    let root = args.state_root;
    let id = args.session_id;
    let mut session = catalog::read_session(&root, &id)?;

    // Cloned rather than borrowed from `session`: the failure paths below
    // need to mutate the record while still holding the program name for
    // the error message.
    let command = session.command.clone();
    let Some((program, program_args)) = command.split_first() else {
        return Err(Error::conflict(format!(
            "session {id} has an empty command and cannot be started"
        )));
    };
    // A session's own working directory, which for a worktree session is
    // the entire point: without it an isolated session would quietly
    // operate on the user's main working copy.
    let cwd = session
        .workspace
        .as_ref()
        .map(|w| w.cwd.clone())
        .unwrap_or_else(std::env::temp_dir);

    let port = SystemProcessPort;
    let me = std::process::id();

    // Started, but not yet wired: the output reader and exit watcher both
    // need the `Worker`, which cannot exist until the pid is known. So the
    // process is started here and handed on below.
    let (backend, child_pid, started) = if session.pty {
        match start_pty(program, program_args, &cwd) {
            Ok(pty) => {
                let pty = Arc::new(pty);
                let pid = pty.pid();
                (Backend::Pty(Arc::clone(&pty)), pid, Started::Pty(pty))
            }
            Err(e) => return record_start_failure(&root, &mut session, program, e),
        }
    } else {
        match start_piped(program, program_args, &cwd) {
            Ok(mut child) => {
                let pid = child.id();
                // Taken now, because the `Backend` owns it from here on.
                let stdin = child.stdin.take();
                (Backend::Piped(Mutex::new(stdin)), pid, Started::Piped(child))
            }
            Err(e) => return record_start_failure(&root, &mut session, program, e),
        }
    };

    // Bound **before** the record below leaves `Created`, and that
    // ordering is load-bearing. The daemon treats a record past `Created`
    // as "this session is ready to attach to", so publishing `Running`
    // first would open a window where a client that attached immediately
    // found no socket and silently fell back to replaying a transcript
    // instead of streaming.
    let listener = transport::Listener::bind(
        "binding the worker socket",
        &paths::worker_socket(&root, &id),
    )?;

    // The worker records **itself**, rather than the daemon recording it.
    // That is what keeps `state.json` single-writer while a worker is
    // alive: the daemon writes the record at creation and then never
    // touches it again until it has established that this process is
    // dead. See `catalog`'s ownership table.
    session.worker = Some(worker_ref(&port, me));
    session.child = Some(worker_ref(&port, child_pid));
    session.transition_to(SessionStatus::Running)?;
    catalog::write_session(&root, &session)?;

    let (events, _) = broadcast::channel(BROADCAST_CAPACITY);
    let worker = Arc::new(Worker {
        root: root.clone(),
        id: id.clone(),
        events,
        backend,
        shutdown: Notify::new(),
        child_pid,
    });

    worker.emit(SessionEvent::Status {
        status: SessionStatus::Running,
    });

    // Wire output and exit detection, which differ by backend: a PTY is
    // one merged stream read on its own thread, while pipes are two
    // async streams plus a separate `wait`.
    match started {
        Started::Pty(pty) => spawn_pty_reader(Arc::clone(&worker), pty),
        Started::Piped(mut child) => {
            if let Some(stdout) = child.stdout.take() {
                rusty_tokio::spawn(pump(Arc::clone(&worker), stdout));
            }
            if let Some(stderr) = child.stderr.take() {
                rusty_tokio::spawn(pump(Arc::clone(&worker), stderr));
            }
            rusty_tokio::spawn({
                let worker = Arc::clone(&worker);
                async move {
                    // The exit status is the tier-2 signal: free, always
                    // available, and the one status source that cannot be
                    // wrong.
                    let code = match child.wait().await {
                        Ok(status) => status.code(),
                        Err(e) => {
                            eprintln!("sessionmgr worker: waiting on the session failed: {e}");
                            None
                        }
                    };
                    worker.record_child_exit(code);
                    worker.shutdown.notify_one();
                }
            });
        }
    }

    rusty_tokio::spawn({
        let worker = Arc::clone(&worker);
        async move {
            loop {
                match listener.accept().await {
                    Ok(conn) => {
                        rusty_tokio::spawn(serve(Arc::clone(&worker), conn));
                    }
                    Err(e) => {
                        eprintln!("sessionmgr worker: accept failed: {e}");
                        return;
                    }
                }
            }
        }
    });

    worker.shutdown.notified().await;
    Ok(())
}

/// Starts the session's process on a real terminal.
fn start_pty(program: &str, args: &[String], cwd: &Path) -> std::io::Result<PtySession> {
    PtySession::spawn(PtyOptions {
        program: program.into(),
        args: args.iter().map(Into::into).collect(),
        cwd: cwd.as_os_str().to_owned(),
        // The creating client usually has no terminal of its own, so this
        // is a placeholder an attaching UI corrects with a resize.
        size: TerminalSize::default(),
    })
}

/// Starts the session's process on plain pipes.
fn start_piped(
    program: &str,
    args: &[String],
    cwd: &Path,
) -> std::io::Result<rusty_tokio::process::Child> {
    use rusty_tokio::process::{Command, Stdio};
    let mut cmd = Command::new(program);
    cmd.args(args)
        .current_dir(cwd)
        .stdin(Stdio::piped())
        .stdout(Stdio::piped())
        .stderr(Stdio::piped());
    // Deliberately *not* `prepare_detached`: this worker owns its child.
    // Detachment is about surviving the *daemon*, and this worker is
    // already detached from it. Detaching the child too would mean
    // nothing tracked its exit.
    cmd.spawn()
}

/// Records a failure to start the session's process as a real exit.
///
/// A bad command is the user's ordinary mistake, not a crash. Recording
/// it as an exit makes the session show `Errored` rather than sitting in
/// `Created` until something else reconciles it as a phantom crash.
fn record_start_failure(
    root: &Path,
    session: &mut sessionmgr_core::Session,
    program: &str,
    error: std::io::Error,
) -> Result<()> {
    let _ = session.record_exit(None);
    catalog::write_session(root, session)?;
    Err(Error::io(
        "starting the session's command",
        PathBuf::from(program),
        error,
    ))
}

/// Reads a PTY-hosted session's output on a dedicated OS thread.
///
/// A plain `std::thread`, not `spawn_blocking`: the read blocks for the
/// entire life of the session, which would occupy a runtime blocking-pool
/// slot from creation to teardown. Nothing in the loop is async --
/// `broadcast::Sender::send` and the transcript append are both
/// synchronous -- so the thread needs no runtime at all.
fn spawn_pty_reader(worker: Arc<Worker>, pty: Arc<PtySession>) {
    std::thread::spawn(move || {
        let mut buf = [0u8; READ_BUFFER];
        loop {
            match pty.read(&mut buf) {
                // The documented end-of-stream signal: the process exited.
                Ok(0) => break,
                Ok(n) => worker.emit(SessionEvent::Output {
                    data: buf[..n].to_vec(),
                }),
                Err(e) => {
                    // A master whose child has gone reports an I/O error
                    // rather than a clean zero-length read on some
                    // platforms. Either way the session is over, and
                    // treating it as a read failure would report a normal
                    // exit as a fault.
                    let _ = e;
                    break;
                }
            }
        }
        let code = pty.wait().unwrap_or(None);
        worker.record_child_exit(code);
        worker.shutdown.notify_one();
    });
}

impl Worker {
    /// Records an event to the transcript and fans it out to attached
    /// clients.
    ///
    /// Transcript first: a client that receives an event the transcript
    /// does not have would see it vanish on reattach.
    fn emit(&self, event: SessionEvent) {
        if let Err(e) = catalog::append_transcript(&self.root, &self.id, &event) {
            eprintln!("sessionmgr worker: could not append to the transcript: {e}");
        }
        // An error here means nobody is attached, which is the normal
        // case for a background session and not a problem.
        let _ = self.events.send(event);
    }

    fn record_child_exit(&self, code: Option<i32>) {
        match catalog::read_session(&self.root, &self.id) {
            Ok(mut session) => {
                if let Err(e) = session.record_exit(code) {
                    eprintln!("sessionmgr worker: {e}");
                }
                if let Err(e) = catalog::write_session(&self.root, &session) {
                    eprintln!("sessionmgr worker: could not record the exit: {e}");
                }
            }
            Err(e) => eprintln!("sessionmgr worker: could not read the session record: {e}"),
        }
        self.emit(SessionEvent::Exited { code });
    }

    /// Sends input to the session's process.
    async fn send_input(&self, data: Vec<u8>) -> Result<()> {
        match &self.backend {
            Backend::Pty(pty) => {
                let pty = Arc::clone(pty);
                // The write blocks, so it goes to the blocking pool
                // rather than stalling a runtime worker. Unlike the read,
                // this is short-lived, so a pool slot is the right home
                // for it.
                rusty_tokio::spawn_blocking(move || pty.write_all(&data))
                    .await
                    // The blocking task itself failed to run to
                    // completion -- it panicked, or the runtime is
                    // shutting down. Neither is an I/O failure of the
                    // terminal, so it is reported as its own thing.
                    .map_err(|e| {
                        Error::conflict(format!("the terminal write task did not complete: {e}"))
                    })?
                    .map_err(|e| Error::io("writing to the session's terminal", None, e))
            }
            Backend::Piped(stdin) => {
                let mut guard = stdin.lock().await;
                let Some(stdin) = guard.as_mut() else {
                    return Err(Error::conflict("this session's input stream is closed"));
                };
                stdin
                    .write_all(&data)
                    .await
                    .map_err(|e| Error::io("writing to the session's stdin", None, e))?;
                stdin
                    .flush()
                    .await
                    .map_err(|e| Error::io("flushing the session's stdin", None, e))
            }
        }
    }

    /// Tells the session's terminal it has been resized.
    fn resize(&self, rows: u16, cols: u16) -> Result<()> {
        match &self.backend {
            Backend::Pty(pty) => pty
                .resize(TerminalSize { rows, cols })
                .map_err(|e| Error::io("resizing the session's terminal", None, e)),
            // Not an error worth failing on: a piped session has no
            // terminal to resize, and a UI that resizes every session it
            // shows should not have to special-case which ones have one.
            Backend::Piped(_) => Ok(()),
        }
    }
}

/// Serves one connection on the worker's private socket.
async fn serve(worker: Arc<Worker>, mut conn: transport::Connection) {
    let request: Option<Request> = match conn.read().await {
        Ok(request) => request,
        Err(e) => {
            let _ = conn
                .write(&Response::Error {
                    kind: ErrorKind::Protocol,
                    message: e.to_string(),
                })
                .await;
            return;
        }
    };
    let Some(request) = request else { return };

    match request {
        Request::Ping => {
            let _ = conn
                .write(&Response::Pong {
                    pid: std::process::id(),
                })
                .await;
        }
        Request::SessionAttach { .. } => attach(worker, conn).await,
        Request::SessionInput { data, .. } => {
            let response = match worker.send_input(data).await {
                Ok(()) => Response::Ok,
                Err(e) => Response::Error {
                    kind: e.kind(),
                    message: e.to_string(),
                },
            };
            let _ = conn.write(&response).await;
        }
        Request::SessionResize { rows, cols, .. } => {
            let response = match worker.resize(rows, cols) {
                Ok(()) => Response::Ok,
                Err(e) => Response::Error {
                    kind: e.kind(),
                    message: e.to_string(),
                },
            };
            let _ = conn.write(&response).await;
        }
        Request::WorkerShutdown => {
            let _ = conn.write(&Response::Ok).await;
            // Terminate the session's process before exiting. Leaving it
            // running would orphan it with nothing recording it as live
            // and nothing able to reach it -- the exact failure the
            // recorded pid pair exists to prevent, and it would be
            // perverse to create it here on the *graceful* path.
            if let Err(e) = sessionmgr_proc::terminate(worker.child_pid) {
                eprintln!("sessionmgr worker: could not terminate the session's process: {e}");
            }
            worker.shutdown.notify_one();
        }
        other => {
            let _ = conn
                .write(&Response::Error {
                    kind: ErrorKind::Protocol,
                    message: format!("unexpected request on the private worker socket: {other:?}"),
                })
                .await;
        }
    }
}

/// Replays the transcript, then streams live events.
///
/// Replay-then-subscribe, and **subscribe before replaying**: the
/// subscription is taken first so that an event arriving during the
/// replay is queued rather than lost in the gap between the two. The cost
/// is that such an event may be delivered twice, which is the right
/// trade -- a client seeing a line twice is a cosmetic flaw, a client
/// missing the line telling it the agent is waiting for input is a
/// functional one.
async fn attach(worker: Arc<Worker>, conn: transport::Connection) {
    let mut live = worker.events.subscribe();
    let (mut reader, mut writer) = conn.into_parts();

    match catalog::read_transcript(&worker.root, &worker.id) {
        Ok(history) => {
            for event in history {
                if transport::write_framed(&mut writer, &event).await.is_err() {
                    return;
                }
            }
        }
        Err(e) => eprintln!("sessionmgr worker: could not replay the transcript: {e}"),
    }

    // Client -> session: input and resizes, read concurrently with the
    // outbound stream below.
    rusty_tokio::spawn({
        let worker = Arc::clone(&worker);
        async move {
            while let Ok(Some(request)) = transport::read_framed::<Request>(&mut reader).await {
                let outcome = match request {
                    Request::SessionInput { data, .. } => worker.send_input(data).await,
                    Request::SessionResize { rows, cols, .. } => worker.resize(rows, cols),
                    _ => Ok(()),
                };
                if let Err(e) = outcome {
                    eprintln!("sessionmgr worker: {e}");
                }
            }
        }
    });

    // Session -> client.
    loop {
        match live.recv().await {
            Ok(event) => {
                if transport::write_framed(&mut writer, &event).await.is_err() {
                    return;
                }
            }
            // The client fell far enough behind to lose events. The
            // transcript still has them, so this is a gap in the live
            // view rather than data loss; keep streaming rather than
            // dropping the client.
            Err(broadcast::RecvError::Lagged(_)) => continue,
            Err(broadcast::RecvError::Closed) => return,
        }
    }
}

/// Streams one of a piped child's output handles into the transcript and
/// the broadcast channel.
///
/// Chunked reads, not line reads: a prompt waiting for an answer has no
/// trailing newline, and a line-buffered pump would hold it until the
/// user answered a question they had not been shown.
async fn pump<R>(worker: Arc<Worker>, mut source: R)
where
    R: AsyncReadExt + Unpin + Send + 'static,
{
    let mut buf = [0u8; READ_BUFFER];
    loop {
        match source.read(&mut buf).await {
            Ok(0) => return,
            Ok(n) => worker.emit(SessionEvent::Output {
                data: buf[..n].to_vec(),
            }),
            Err(e) => {
                eprintln!("sessionmgr worker: reading session output failed: {e}");
                return;
            }
        }
    }
}
