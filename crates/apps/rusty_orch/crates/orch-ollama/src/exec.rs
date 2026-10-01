//! The process seam: argv in, captured output out, with a deadline.
//!
//! `contract::ProcessRunner` has no stdin and no timeout, so this crate
//! keeps its own one-method trait (ADR-0004). It collapses into `contract`
//! the day that trait grows both.

use std::fmt;
use std::io::{self, Read, Write};
use std::process::{Child, Command, Stdio};
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::Arc;
use std::thread;
use std::time::{Duration, Instant};

/// Captured stdout is cut off here; a reply this long is never valid.
pub const MAX_STDOUT_BYTES: usize = 1 << 20;

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
    /// The deadline passed; the process was killed and reaped.
    Timeout(Duration),
    /// stdout exceeded [`MAX_STDOUT_BYTES`]; the process was killed.
    StdoutOverflow,
    /// Reading or writing a pipe failed.
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
    /// `argv[0]` is the program. Never a shell.
    fn run(&self, argv: &[String], stdin: &[u8], timeout: Duration) -> Result<Exit, ExecError>;
}

/// The real thing, over `std::process`.
#[derive(Debug, Clone, Copy, Default)]
pub struct StdCommand;

impl CommandRunner for StdCommand {
    fn run(&self, argv: &[String], stdin: &[u8], timeout: Duration) -> Result<Exit, ExecError> {
        let (program, args) = argv
            .split_first()
            .ok_or_else(|| ExecError::Spawn("empty argv".to_owned()))?;
        let mut child = Command::new(program)
            .args(args)
            .stdin(Stdio::piped())
            .stdout(Stdio::piped())
            .stderr(Stdio::piped())
            .spawn()
            .map_err(|e| ExecError::Spawn(e.to_string()))?;
        let pipes = Pipes::take(&mut child, stdin);
        let status = wait_with_deadline(&mut child, &pipes.overflow, timeout);
        let (stdout, stderr) = pipes.join()?;
        Ok(Exit {
            status: status?,
            stdout,
            stderr,
        })
    }
}

/// The three pipe threads. stdin is written and closed on its own thread so
/// a child that writes before it reads cannot deadlock us; stdout and stderr
/// are drained concurrently for the same reason.
struct Pipes {
    overflow: Arc<AtomicBool>,
    stdin: Option<thread::JoinHandle<io::Result<()>>>,
    stdout: Option<thread::JoinHandle<io::Result<Vec<u8>>>>,
    stderr: Option<thread::JoinHandle<io::Result<Vec<u8>>>>,
}

impl Pipes {
    fn take(child: &mut Child, input: &[u8]) -> Self {
        let overflow = Arc::new(AtomicBool::new(false));
        let input = input.to_vec();
        let stdin = child.stdin.take().map(|mut pipe| {
            thread::spawn(move || {
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
            .map(|pipe| thread::spawn(move || read_capped(pipe, &flag)));
        let stderr = child
            .stderr
            .take()
            .map(|pipe| thread::spawn(move || read_capped(pipe, &AtomicBool::new(false))));
        Self {
            overflow,
            stdin,
            stdout,
            stderr,
        }
    }

    fn join(self) -> Result<(Vec<u8>, Vec<u8>), ExecError> {
        join(self.stdin).map(|_| ())?;
        let stdout = join(self.stdout)?.unwrap_or_default();
        let stderr = join(self.stderr)?.unwrap_or_default();
        if self.overflow.load(Ordering::SeqCst) {
            return Err(ExecError::StdoutOverflow);
        }
        Ok((stdout, stderr))
    }
}

fn join<T>(handle: Option<thread::JoinHandle<io::Result<T>>>) -> Result<Option<T>, ExecError> {
    let Some(handle) = handle else {
        return Ok(None);
    };
    let result = handle
        .join()
        .map_err(|_| ExecError::Io("pipe thread panicked".to_owned()))?;
    result.map(Some).map_err(|e| ExecError::Io(e.to_string()))
}

/// Read until EOF or one byte past [`MAX_STDOUT_BYTES`]. On overflow the
/// flag is raised and the pipe dropped, so the child sees EPIPE and the
/// waiter kills it.
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

/// Poll until exit, deadline, or overflow. On deadline or overflow the child
/// is killed and reaped so it never lingers as a zombie.
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
            kill_and_reap(child)?;
            return Err(ExecError::StdoutOverflow);
        }
        if Instant::now() >= deadline {
            kill_and_reap(child)?;
            return Err(ExecError::Timeout(timeout));
        }
        thread::sleep(POLL);
    }
}

fn kill_and_reap(child: &mut Child) -> Result<(), ExecError> {
    child.kill().map_err(|e| ExecError::Io(e.to_string()))?;
    child.wait().map_err(|e| ExecError::Io(e.to_string()))?;
    Ok(())
}
