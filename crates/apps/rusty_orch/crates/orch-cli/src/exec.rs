//! The process seam: argv in, captured output out, with a hard deadline.
//!
//! `contract::ProcessRunner` has no stdin and no timeout, so this crate
//! keeps its own one-method trait (ADR-0004). It collapses into `contract`
//! the day that trait grows both.
//!
//! The deadline is a hard bound. The child is spawned as the leader of its
//! own process group (unix) and the whole group, or process tree on
//! Windows, is killed on timeout or overflow, then the leader is reaped.
//! Pipe reader threads are then joined for at most [`JOIN_GRACE`]; if a
//! grandchild that escaped the group (`setsid`) still holds a pipe, the
//! reader thread is detached and leaks, and the deadline still holds.

use std::fmt;
use std::io::{self, Read, Write};
use std::path::Path;
use std::process::{Child, Command, Stdio};
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::mpsc::{self, Receiver, RecvTimeoutError};
use std::sync::Arc;
use std::thread;
use std::time::{Duration, Instant};

/// Longest slice of model output or stderr an error message carries.
pub const EXCERPT_CHARS: usize = 200;

/// A bounded, single-line excerpt for error messages; never the whole output.
pub fn excerpt(bytes: &[u8]) -> String {
    let text = String::from_utf8_lossy(bytes);
    let mut out: String = text.chars().take(EXCERPT_CHARS).collect();
    if text.chars().count() > EXCERPT_CHARS {
        out.push('…');
    }
    out.replace('\n', " ").trim().to_owned()
}

/// Captured stdout is cut off here; a reply this long is never valid.
pub const MAX_STDOUT_BYTES: usize = 1 << 20;

/// How long to wait for the pipe threads after the child is gone.
pub const JOIN_GRACE: Duration = Duration::from_secs(2);

const POLL: Duration = Duration::from_millis(10);

/// What a finished process left behind.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Exit {
    /// Exit status; `-1` when the process died to a signal.
    pub status: i32,
    pub stdout: Vec<u8>,
    pub stderr: Vec<u8>,
}

/// Why a process could not be run to completion.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum ExecError {
    /// The program could not be started.
    Spawn(String),
    /// The deadline passed; the process group was killed and reaped.
    Timeout(Duration),
    /// stdout exceeded [`MAX_STDOUT_BYTES`]; the process group was killed.
    StdoutOverflow,
    /// Reading or writing a pipe failed, or a pipe stayed open past
    /// [`JOIN_GRACE`] after the child exited.
    Io(String),
}

impl fmt::Display for ExecError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::Spawn(e) => write!(f, "could not start process: {e}"),
            Self::Timeout(d) => write!(f, "process exceeded {}s", d.as_secs()),
            Self::StdoutOverflow => write!(f, "process wrote more than {MAX_STDOUT_BYTES} bytes"),
            Self::Io(e) => write!(f, "pipe error: {e}"),
        }
    }
}

impl std::error::Error for ExecError {}

/// Runs one non-interactive command: fixed argv, bytes on stdin, deadline.
pub trait CommandRunner {
    /// `argv[0]` is the program. Never a shell. `cwd` is the child's working
    /// directory, or the parent's when `None`; an adapter whose CLI reads
    /// the repository from its working directory passes the repository root
    /// here rather than inheriting wherever the orchestrator was launched.
    /// `remove_env` names variables the child must not see, e.g. a vendor
    /// API key that would otherwise route a subscription CLI onto a paid API
    /// path.
    fn run_in(
        &self,
        cwd: Option<&Path>,
        argv: &[String],
        stdin: &[u8],
        timeout: Duration,
        remove_env: &[&str],
    ) -> Result<Exit, ExecError>;

    /// [`CommandRunner::run_in`] in the parent's working directory.
    fn run_scrubbed(
        &self,
        argv: &[String],
        stdin: &[u8],
        timeout: Duration,
        remove_env: &[&str],
    ) -> Result<Exit, ExecError> {
        self.run_in(None, argv, stdin, timeout, remove_env)
    }

    /// [`CommandRunner::run_in`] in the parent's working directory with the
    /// child's environment untouched.
    fn run(&self, argv: &[String], stdin: &[u8], timeout: Duration) -> Result<Exit, ExecError> {
        self.run_in(None, argv, stdin, timeout, &[])
    }
}

/// The real thing, over `std::process`.
#[derive(Debug, Clone, Copy, Default)]
pub struct StdCommand;

impl CommandRunner for StdCommand {
    fn run_in(
        &self,
        cwd: Option<&Path>,
        argv: &[String],
        stdin: &[u8],
        timeout: Duration,
        remove_env: &[&str],
    ) -> Result<Exit, ExecError> {
        let (program, args) = argv
            .split_first()
            .ok_or_else(|| ExecError::Spawn("empty argv".to_owned()))?;
        let mut cmd = Command::new(program);
        cmd.args(args)
            .stdin(Stdio::piped())
            .stdout(Stdio::piped())
            .stderr(Stdio::piped());
        if let Some(dir) = cwd {
            cmd.current_dir(dir);
        }
        for name in remove_env {
            cmd.env_remove(name);
        }
        own_process_group(&mut cmd);
        let mut child = cmd.spawn().map_err(|e| ExecError::Spawn(e.to_string()))?;
        let pipes = Pipes::take(&mut child, stdin);
        let status = match wait_with_deadline(&mut child, &pipes.overflow, timeout) {
            Ok(status) => status,
            Err(stop) => {
                // Timeout and overflow win over anything the pipes report.
                let _ = pipes.join(JOIN_GRACE);
                return Err(stop);
            }
        };
        match pipes.join(JOIN_GRACE) {
            Ok((stdout, stderr)) => Ok(Exit {
                status,
                stdout,
                stderr,
            }),
            Err(e) => {
                // A reader is still blocked: something the child spawned holds
                // the pipe. Take the group down so the deadline holds.
                let _ = kill_group(&mut child);
                Err(e)
            }
        }
    }
}

#[cfg(unix)]
fn own_process_group(cmd: &mut Command) {
    use std::os::unix::process::CommandExt;
    // Leader of a new group whose id equals the child's pid, so the group
    // can be addressed as `-<pid>` without a libc dependency.
    cmd.process_group(0);
}

#[cfg(not(unix))]
fn own_process_group(_: &mut Command) {}

/// Kill the child and everything it spawned, then reap the child. The
/// child itself is also killed directly in case the group command fails.
fn kill_group(child: &mut Child) -> Result<(), ExecError> {
    let _ = kill_descendants(child.id());
    match child.kill() {
        Ok(()) => {}
        // Already gone: fine, we only need it reaped below.
        Err(e) if e.kind() == io::ErrorKind::InvalidInput => {}
        Err(e) => return Err(ExecError::Io(e.to_string())),
    }
    child.wait().map_err(|e| ExecError::Io(e.to_string()))?;
    Ok(())
}

/// SIGKILL the whole process group via the system `kill`, fixed argv.
#[cfg(unix)]
fn kill_descendants(pid: u32) -> io::Result<()> {
    Command::new("kill")
        .args(["-KILL", "--", &format!("-{pid}")])
        .stdin(Stdio::null())
        .stdout(Stdio::null())
        .stderr(Stdio::null())
        .status()
        .map(|_| ())
}

/// Kill the process tree via the system `taskkill`, fixed argv. Compiles
/// on Windows; not exercised in CI.
#[cfg(windows)]
fn kill_descendants(pid: u32) -> io::Result<()> {
    Command::new("taskkill")
        .args(["/T", "/F", "/PID", &pid.to_string()])
        .stdin(Stdio::null())
        .stdout(Stdio::null())
        .stderr(Stdio::null())
        .status()
        .map(|_| ())
}

#[cfg(not(any(unix, windows)))]
fn kill_descendants(_: u32) -> io::Result<()> {
    Ok(())
}

/// The three pipe threads, each reporting on its own channel so a join can
/// be bounded: stdin is written and closed on its own thread so a child that
/// writes before it reads cannot deadlock us; stdout and stderr are drained
/// concurrently for the same reason.
struct Pipes {
    overflow: Arc<AtomicBool>,
    stdin: Option<Receiver<io::Result<()>>>,
    stdout: Option<Receiver<io::Result<Vec<u8>>>>,
    stderr: Option<Receiver<io::Result<Vec<u8>>>>,
}

impl Pipes {
    fn take(child: &mut Child, input: &[u8]) -> Self {
        let overflow = Arc::new(AtomicBool::new(false));
        let input = input.to_vec();
        let stdin = child.stdin.take().map(|mut pipe| {
            spawn_reporting(move || {
                let written = pipe.write_all(&input);
                drop(pipe);
                // A child that exits without reading its prompt reports
                // that through its exit status; the broken pipe is noise.
                match written {
                    Err(e) if e.kind() == io::ErrorKind::BrokenPipe => Ok(()),
                    other => other,
                }
            })
        });
        let flag = Arc::clone(&overflow);
        let stdout = child
            .stdout
            .take()
            .map(|pipe| spawn_reporting(move || read_capped(pipe, &flag)));
        let stderr = child
            .stderr
            .take()
            .map(|pipe| spawn_reporting(move || read_capped(pipe, &AtomicBool::new(false))));
        Self {
            overflow,
            stdin,
            stdout,
            stderr,
        }
    }

    /// Wait at most `grace` in total for all three threads. A thread that
    /// does not finish is detached, never blocked on.
    fn join(self, grace: Duration) -> Result<(Vec<u8>, Vec<u8>), ExecError> {
        let deadline = Instant::now() + grace;
        recv_by(self.stdin, deadline, "stdin")?;
        let stdout = recv_by(self.stdout, deadline, "stdout")?.unwrap_or_default();
        let stderr = recv_by(self.stderr, deadline, "stderr")?.unwrap_or_default();
        if self.overflow.load(Ordering::SeqCst) {
            return Err(ExecError::StdoutOverflow);
        }
        Ok((stdout, stderr))
    }
}

fn spawn_reporting<T: Send + 'static>(
    work: impl FnOnce() -> io::Result<T> + Send + 'static,
) -> Receiver<io::Result<T>> {
    let (tx, rx) = mpsc::channel();
    thread::spawn(move || {
        // The receiver is gone only if the caller already gave up on us.
        let _ = tx.send(work());
    });
    rx
}

fn recv_by<T>(
    rx: Option<Receiver<io::Result<T>>>,
    deadline: Instant,
    name: &str,
) -> Result<Option<T>, ExecError> {
    let Some(rx) = rx else {
        return Ok(None);
    };
    let remaining = deadline.saturating_duration_since(Instant::now());
    match rx.recv_timeout(remaining) {
        Ok(Ok(value)) => Ok(Some(value)),
        Ok(Err(e)) => Err(ExecError::Io(format!("{name}: {e}"))),
        Err(RecvTimeoutError::Timeout) => Err(ExecError::Io(format!(
            "{name} still open {}s after the process ended",
            JOIN_GRACE.as_secs()
        ))),
        Err(RecvTimeoutError::Disconnected) => {
            Err(ExecError::Io(format!("{name} pipe thread panicked")))
        }
    }
}

/// Read until EOF or one byte past [`MAX_STDOUT_BYTES`]. On overflow the
/// flag is raised and the pipe dropped, so the waiter kills the group.
fn read_capped(mut pipe: impl Read, overflow: &AtomicBool) -> io::Result<Vec<u8>> {
    let mut buf = Vec::new();
    let mut chunk = [0u8; 8192];
    loop {
        let n = pipe.read(&mut chunk)?;
        if n == 0 {
            return Ok(buf);
        }
        buf.extend_from_slice(&chunk[..n]);
        if buf.len() > MAX_STDOUT_BYTES {
            overflow.store(true, Ordering::SeqCst);
            buf.truncate(MAX_STDOUT_BYTES);
            return Ok(buf);
        }
    }
}

/// Poll until exit, deadline, or overflow. On deadline or overflow the
/// whole group is killed and the child reaped so nothing lingers.
fn wait_with_deadline(
    child: &mut Child,
    overflow: &AtomicBool,
    timeout: Duration,
) -> Result<i32, ExecError> {
    let deadline = Instant::now() + timeout;
    loop {
        if let Some(status) = child.try_wait().map_err(|e| ExecError::Io(e.to_string()))? {
            return Ok(status.code().unwrap_or(-1));
        }
        if overflow.load(Ordering::SeqCst) {
            kill_group(child)?;
            return Err(ExecError::StdoutOverflow);
        }
        if Instant::now() >= deadline {
            kill_group(child)?;
            return Err(ExecError::Timeout(timeout));
        }
        thread::sleep(POLL);
    }
}
