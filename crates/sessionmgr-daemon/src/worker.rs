//! The per-session worker: one detached OS process that owns one
//! session's child process, its transcript, and its private socket.
//!
//! **This is the process that makes the product's central promise true.**
//! Closing the manager must not stop the work, so the thing actually
//! running the work cannot be a child of the manager's lifetime. The
//! daemon re-execs its own binary here with detach flags and then forgets
//! about it except as a pid on disk.
//!
//! Modelled directly on `rusty_prime_agent::worker`, which solved the
//! same problem for the same reasons.

use std::path::{Path, PathBuf};
use std::sync::Arc;

use rusty_tokio::io::AsyncReadExt;
use rusty_tokio::io::AsyncWriteExt;
use rusty_tokio::sync::{broadcast, Mutex, Notify};
use sessionmgr_core::ports::worker_ref;
use sessionmgr_core::{SessionId, SessionStatus};
use sessionmgr_proc::SystemProcessPort;
use sessionmgr_protocol::{ErrorKind, Request, Response, SessionEvent};

use crate::error::{Error, Result};
use crate::{catalog, paths, transport};

/// How many events a slow client may fall behind before it starts losing
/// them. Output is also on disk in the transcript, so a lagging client
/// loses nothing permanently -- it just sees a gap in the live stream.
const BROADCAST_CAPACITY: usize = 1024;

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

/// Shared worker state. `Arc`ed across the accept loop, the output pumps,
/// and the child-exit watcher.
struct Worker {
    root: PathBuf,
    id: SessionId,
    events: broadcast::Sender<SessionEvent>,
    /// The child's stdin, held so attached clients can send input.
    stdin: Mutex<Option<rusty_tokio::process::ChildStdin>>,
    /// Set when the worker should exit: either the child finished or a
    /// shutdown was requested.
    shutdown: Notify,
    child_pid: u32,
}

/// The `__worker-main` entrypoint.
pub async fn run(args: WorkerArgs) -> Result<()> {
    use rusty_tokio::process::{Command, Stdio};

    let root = args.state_root;
    let id = args.session_id;
    let mut session = catalog::read_session(&root, &id)?;

    // Cloned rather than borrowed from `session`: the failure path below
    // needs to mutate the record while still holding the program name for
    // the error message.
    let command = session.command.clone();
    let Some((program, program_args)) = command.split_first() else {
        return Err(Error::conflict(format!(
            "session {id} has an empty command and cannot be started"
        )));
    };

    let mut cmd = Command::new(program);
    cmd.args(program_args)
        .stdin(Stdio::piped())
        .stdout(Stdio::piped())
        .stderr(Stdio::piped());
    // Deliberately *not* `prepare_detached`: this worker owns its child.
    // Detachment is about surviving the *daemon*, and this worker is
    // already detached from it. Detaching the child too would mean
    // nothing tracked its exit.
    let mut child = match cmd.spawn() {
        Ok(child) => child,
        Err(e) => {
            // A bad command is the user's ordinary mistake, not a crash.
            // Record it as a real exit so the session shows `Errored`
            // rather than sitting in `Created` until something else
            // reconciles it as a phantom crash.
            let _ = session.record_exit(None);
            catalog::write_session(&root, &session)?;
            return Err(Error::io(
                "spawning the session's command",
                PathBuf::from(program),
                e,
            ));
        }
    };

    let port = SystemProcessPort;
    let child_pid = child.id();
    let me = std::process::id();

    // Bound **before** the record below leaves `Created`, and that
    // ordering is load-bearing. The daemon treats a record past `Created`
    // as "this session is ready to attach to", so publishing `Running`
    // first would open a window where a client that attached immediately
    // found no socket and silently fell back to replaying a transcript
    // instead of streaming. Connections that arrive before the accept
    // loop starts queue in the listen backlog, which is exactly what a
    // backlog is for.
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
        stdin: Mutex::new(child.stdin.take()),
        shutdown: Notify::new(),
        child_pid,
    });

    worker.emit(SessionEvent::Status {
        status: SessionStatus::Running,
    });

    if let Some(stdout) = child.stdout.take() {
        rusty_tokio::spawn(pump(Arc::clone(&worker), stdout));
    }
    if let Some(stderr) = child.stderr.take() {
        rusty_tokio::spawn(pump(Arc::clone(&worker), stderr));
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

    // Watch the child. Its exit status is PLAN.md's tier-2 signal: free,
    // always available, and the one status source that cannot be wrong.
    rusty_tokio::spawn({
        let worker = Arc::clone(&worker);
        async move {
            let status = child.wait().await;
            let code = match status {
                Ok(status) => status.code(),
                Err(e) => {
                    eprintln!("sessionmgr worker: waiting on the child failed: {e}");
                    None
                }
            };
            worker.record_child_exit(code);
            worker.shutdown.notify_one();
        }
    });

    worker.shutdown.notified().await;
    Ok(())
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
}

/// Streams one of the child's output handles into the transcript and the
/// broadcast channel.
///
/// Chunked reads, not line reads: an agent CLI's prompt ("Continue?
/// [y/N] ") has no trailing newline, and a line-buffered pump would hold
/// it until the user answered a question they had not been shown.
///
/// **Known limitation, and a real input to the Phase 1 PTY spike**:
/// `from_utf8_lossy` is applied per chunk, so a multi-byte character
/// split across a read boundary is mangled into replacement characters.
/// Fixing that properly means carrying a decoder across chunks, or
/// switching the wire type to bytes. Which of those is right depends on
/// whether sessions end up PTY-backed and therefore carrying control
/// sequences -- which is precisely what the spike decides, so this stays
/// as-is until it has an answer.
async fn pump<R>(worker: Arc<Worker>, mut source: R)
where
    R: AsyncReadExt + Unpin + Send + 'static,
{
    let mut buf = [0u8; 8192];
    loop {
        match source.read(&mut buf).await {
            Ok(0) => return,
            Ok(n) => worker.emit(SessionEvent::Output {
                data: String::from_utf8_lossy(&buf[..n]).into_owned(),
            }),
            Err(e) => {
                eprintln!("sessionmgr worker: reading child output failed: {e}");
                return;
            }
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
            let response = match worker.send_input(&data).await {
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
            // Terminate the child before exiting. Leaving it running
            // would orphan it with nothing recording it as live and
            // nothing able to reach it -- the exact failure the recorded
            // pid pair exists to prevent, and it would be perverse to
            // create it here on the *graceful* path.
            if let Err(e) = sessionmgr_proc::terminate(worker.child_pid) {
                eprintln!("sessionmgr worker: could not terminate the child: {e}");
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

impl Worker {
    async fn send_input(&self, data: &str) -> Result<()> {
        let mut guard = self.stdin.lock().await;
        let Some(stdin) = guard.as_mut() else {
            return Err(Error::conflict(
                "this session's input stream is closed".to_owned(),
            ));
        };
        stdin
            .write_all(data.as_bytes())
            .await
            .map_err(|e| Error::io("writing to the session's stdin", None, e))?;
        stdin
            .flush()
            .await
            .map_err(|e| Error::io("flushing the session's stdin", None, e))
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

    // Client -> child: input lines, read concurrently with the outbound
    // stream below.
    rusty_tokio::spawn({
        let worker = Arc::clone(&worker);
        async move {
            while let Ok(Some(request)) = transport::read_framed::<Request>(&mut reader).await {
                if let Request::SessionInput { data, .. } = request {
                    if let Err(e) = worker.send_input(&data).await {
                        eprintln!("sessionmgr worker: {e}");
                    }
                }
            }
        }
    });

    // Child -> client.
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
