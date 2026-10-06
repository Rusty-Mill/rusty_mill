//! [`ProcessExecutor`]: the [`Executor`] adapter that runs programs through
//! the sandbox helper.
//!
//! The parent side: spawn the helper in its own process group with a clean
//! environment, capture bounded output, enforce the wall-clock limit by
//! killing the whole group, and turn a non-empty status file into a
//! fail-closed [`Error::Sandbox`].

use std::ffi::OsString;
use std::path::{Path, PathBuf};
use std::process::Stdio;
use std::sync::atomic::{AtomicU64, Ordering};
use std::time::Duration;

use crate::error::Error;
use crate::helper::{HelperRequest, Sockets};
use crate::spec::{ExecOutcome, Executor, SandboxSpec};

/// Bytes of each output stream kept by default.
pub const DEFAULT_CAPTURE_BYTES: usize = 64 * 1024;

/// How long to wait for output pipes after the process group is gone.
#[cfg(target_os = "linux")]
const DRAIN_GRACE: Duration = Duration::from_secs(1);

static NEXT_RUN: AtomicU64 = AtomicU64::new(0);

/// Runs programs under the sandbox helper.
#[derive(Debug, Clone)]
// Off Linux nothing is ever spawned, so the spawn settings go unread.
#[cfg_attr(not(target_os = "linux"), allow(dead_code))]
pub struct ProcessExecutor {
    helper: PathBuf,
    helper_args: Vec<OsString>,
    state_dir: PathBuf,
    capture_bytes: usize,
}

impl ProcessExecutor {
    /// An executor that starts `helper` with `helper_args` (a binary that
    /// hands those arguments to [`crate::run_helper`]; for `rsi`, its own
    /// path and `["__sandbox"]`), keeping status files in `state_dir`.
    ///
    /// `state_dir` must be outside every sandbox's reach; [`Executor::exec`]
    /// checks this on each run.
    #[must_use]
    pub fn new(helper: PathBuf, helper_args: Vec<OsString>, state_dir: PathBuf) -> Self {
        Self {
            helper,
            helper_args,
            state_dir,
            capture_bytes: DEFAULT_CAPTURE_BYTES,
        }
    }

    /// The same executor, keeping up to `bytes` of each output stream
    /// instead of [`DEFAULT_CAPTURE_BYTES`].
    #[must_use]
    pub fn with_capture_bytes(mut self, bytes: usize) -> Self {
        self.capture_bytes = bytes;
        self
    }

    fn status_file(&self) -> Result<PathBuf, Error> {
        std::fs::create_dir_all(&self.state_dir)
            .map_err(|e| Error::io("creating the executor state directory", e))?;
        let run = NEXT_RUN.fetch_add(1, Ordering::Relaxed);
        let path = self
            .state_dir
            .join(format!("sandbox-{}-{run}.status", std::process::id()));
        std::fs::File::create(&path).map_err(|e| Error::io("creating a status file", e))?;
        Ok(path)
    }

    /// Runs `program` like [`Executor::exec`], but with `stdin` as its
    /// standard input and the stricter socket rule `sockets`.
    ///
    /// This is how the harness build ([`Sockets::NoEndpoints`]) and the
    /// inner agent ([`Sockets::None`]) run; for the agent, `stdin` is its
    /// end of the broker socket.
    ///
    /// # Errors
    /// As [`Executor::exec`].
    pub fn exec_with(
        &self,
        spec: &SandboxSpec,
        program: &str,
        args: &[String],
        stdin: Stdio,
        sockets: Sockets,
    ) -> Result<ExecOutcome, Error> {
        let state_dir = canonical(&self.state_dir)?;
        if spec.can_reach(&state_dir) {
            return Err(Error::Sandbox(format!(
                "executor state directory {} is reachable from the sandbox",
                state_dir.display()
            )));
        }
        let status = self.status_file()?;
        let request = HelperRequest {
            status: status.clone(),
            cwd: spec.cwd().clone(),
            cpu_secs: spec.limits().cpu().as_secs(),
            memory_bytes: spec.limits().memory_bytes(),
            file_bytes: spec.limits().file_bytes(),
            open_files: spec.limits().open_files(),
            processes: spec.limits().processes(),
            read: spec.read_roots().to_vec(),
            write: spec.write_roots().to_vec(),
            env: spec.env().to_vec(),
            sockets,
            program: program.into(),
            args: args.iter().map(OsString::from).collect(),
        };
        let outcome = run(self, &request, spec.limits().wall(), stdin);
        let setup_error = std::fs::read_to_string(&status);
        let removed = std::fs::remove_file(&status);
        let setup_error = setup_error.map_err(|e| Error::io("reading the status file", e))?;
        if !setup_error.is_empty() {
            return Err(Error::Sandbox(setup_error));
        }
        removed.map_err(|e| Error::io("removing the status file", e))?;
        outcome
    }
}

impl Executor for ProcessExecutor {
    type Error = Error;

    fn exec(
        &self,
        spec: &SandboxSpec,
        program: &str,
        args: &[String],
    ) -> Result<ExecOutcome, Error> {
        self.exec_with(spec, program, args, Stdio::null(), Sockets::NoInternet)
    }
}

fn canonical(path: &Path) -> Result<PathBuf, Error> {
    std::fs::create_dir_all(path).map_err(|e| Error::io("creating a directory", e))?;
    path.canonicalize()
        .map_err(|e| Error::io(format!("resolving {}", path.display()), e))
}

#[cfg(not(target_os = "linux"))]
fn run(
    _executor: &ProcessExecutor,
    _request: &HelperRequest,
    _wall: Duration,
    _stdin: Stdio,
) -> Result<ExecOutcome, Error> {
    Err(Error::Sandbox(
        "sandboxed execution is only implemented on Linux".into(),
    ))
}

#[cfg(target_os = "linux")]
fn run(
    executor: &ProcessExecutor,
    request: &HelperRequest,
    wall: Duration,
    stdin: Stdio,
) -> Result<ExecOutcome, Error> {
    use std::os::unix::process::{CommandExt, ExitStatusExt};
    use std::process::Command;

    use crate::spec::Termination;

    let start = std::time::Instant::now();
    let mut child = Command::new(&executor.helper)
        .args(&executor.helper_args)
        .args(request.encode())
        .env_clear()
        .stdin(stdin)
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .process_group(0)
        .spawn()
        .map_err(|e| Error::io("starting the sandbox helper", e))?;
    let group = i32::try_from(child.id())
        .map_err(|_| Error::Sandbox("child process id out of range".into()))?;
    let stdout = capture(child.stdout.take(), executor.capture_bytes);
    let stderr = capture(child.stderr.take(), executor.capture_bytes);

    let mut timed_out = false;
    let status = loop {
        if let Some(status) = child
            .try_wait()
            .map_err(|e| Error::io("waiting for the sandboxed process", e))?
        {
            break status;
        }
        if start.elapsed() >= wall {
            timed_out = true;
            kill_group(group)?;
            break child
                .wait()
                .map_err(|e| Error::io("reaping the timed-out process", e))?;
        }
        std::thread::sleep(Duration::from_millis(5));
    };
    let elapsed = start.elapsed();
    // Anything the program left running dies with it. The sandbox forbids
    // leaving the group (sandbox::group_lock), so the group is the job.
    contain(group)?;

    let termination = match (timed_out, status.code(), status.signal()) {
        (true, _, _) => Termination::TimedOut,
        (false, Some(code), _) => Termination::Exited(code),
        (false, None, Some(signal)) => Termination::Signaled(signal),
        (false, None, None) => {
            return Err(Error::Sandbox(format!("unrecognised exit status {status}")));
        }
    };
    Ok(ExecOutcome {
        termination,
        stdout: stdout.finish(),
        stderr: stderr.finish(),
        wall: elapsed,
    })
}

/// Sends `SIGKILL` to the group; a group that is already gone is fine.
#[cfg(target_os = "linux")]
fn kill_group(group: i32) -> Result<(), Error> {
    use rusty_libc::Errno;
    match rusty_libc::process::killpg(group, rusty_libc::signal::SIGKILL) {
        Ok(()) => Ok(()),
        Err(errno) if errno == Errno::ESRCH => Ok(()),
        Err(errno) => Err(Error::Sandbox(format!(
            "killing process group {group}: {errno}"
        ))),
    }
}

/// How long [`contain`] keeps killing before declaring the job uncontained.
#[cfg(target_os = "linux")]
const CONTAIN_DEADLINE: Duration = Duration::from_secs(2);

/// Kills the group until no live member remains (fail closed otherwise).
#[cfg(target_os = "linux")]
fn contain(group: i32) -> Result<(), Error> {
    let start = std::time::Instant::now();
    loop {
        kill_group(group)?;
        let survivors = live_members(group)?;
        if survivors.is_empty() {
            return Ok(());
        }
        if start.elapsed() >= CONTAIN_DEADLINE {
            return Err(Error::Sandbox(format!(
                "processes {survivors:?} of group {group} survived cleanup"
            )));
        }
        std::thread::sleep(Duration::from_millis(10));
    }
}

/// Live (non-zombie) processes whose process group is `group`, from `/proc`.
#[cfg(target_os = "linux")]
fn live_members(group: i32) -> Result<Vec<i32>, Error> {
    let entries = std::fs::read_dir("/proc").map_err(|e| Error::io("listing /proc", e))?;
    let mut members = Vec::new();
    for entry in entries {
        let entry = entry.map_err(|e| Error::io("listing /proc", e))?;
        let Some(pid) = entry
            .file_name()
            .to_str()
            .and_then(|n| n.parse::<i32>().ok())
        else {
            continue;
        };
        let stat = match std::fs::read_to_string(entry.path().join("stat")) {
            Ok(stat) => stat,
            // The process exited between listing and reading: not a member.
            Err(e)
                if matches!(e.kind(), std::io::ErrorKind::NotFound)
                    || e.raw_os_error() == Some(3) =>
            {
                continue;
            }
            Err(e) => return Err(Error::io(format!("reading /proc/{pid}/stat"), e)),
        };
        if let Some((state, pgrp)) = parse_stat(&stat) {
            if pgrp == group && !matches!(state, 'Z' | 'X') {
                members.push(pid);
            }
        }
    }
    Ok(members)
}

/// The state and process group from a `/proc/<pid>/stat` line.
///
/// The command name is parenthesised and may itself contain `)` or spaces,
/// so fields are counted from the *last* `)`.
#[cfg(target_os = "linux")]
fn parse_stat(stat: &str) -> Option<(char, i32)> {
    let (_, rest) = stat.rsplit_once(')')?;
    let mut fields = rest.split_whitespace();
    let state = fields.next()?.chars().next()?;
    let _ppid = fields.next()?;
    let pgrp = fields.next()?.parse().ok()?;
    Some((state, pgrp))
}

/// A stream being drained on its own thread, keeping only the first bytes.
#[cfg(target_os = "linux")]
struct Capture(Option<std::sync::mpsc::Receiver<Vec<u8>>>);

#[cfg(target_os = "linux")]
fn capture<R: std::io::Read + Send + 'static>(stream: Option<R>, limit: usize) -> Capture {
    let Some(mut stream) = stream else {
        return Capture(None);
    };
    let (sender, receiver) = std::sync::mpsc::channel();
    std::thread::spawn(move || {
        let mut kept = Vec::new();
        let mut buffer = [0u8; 8192];
        // Keep reading past the limit so the program never blocks on a full pipe.
        while let Ok(n) = std::io::Read::read(&mut stream, &mut buffer) {
            if n == 0 {
                break;
            }
            let room = limit.saturating_sub(kept.len());
            kept.extend_from_slice(&buffer[..n.min(room)]);
        }
        // The receiver may have given up waiting; there is nobody left to tell.
        let _ = sender.send(kept);
    });
    Capture(Some(receiver))
}

#[cfg(target_os = "linux")]
impl Capture {
    /// The captured bytes, or a note when the pipe was held open by a
    /// process that escaped the group.
    fn finish(self) -> Vec<u8> {
        let Some(receiver) = self.0 else {
            return Vec::new();
        };
        receiver.recv_timeout(DRAIN_GRACE).unwrap_or_else(|_| {
            b"[sandbox: output pipe still held open; capture abandoned]".to_vec()
        })
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::spec::Limits;

    fn spec(root: &Path) -> SandboxSpec {
        let limits = Limits::new(
            Duration::from_secs(1),
            Duration::from_secs(1),
            1 << 28,
            1 << 20,
            32,
            16,
        )
        .expect("valid limits");
        SandboxSpec::new(
            vec![],
            vec![root.to_path_buf()],
            root.to_path_buf(),
            vec![],
            limits,
        )
        .expect("valid spec")
    }

    #[cfg(target_os = "linux")]
    #[test]
    fn parses_stat_lines_with_hostile_command_names() {
        assert_eq!(parse_stat("42 (python3) S 1 42 42 0"), Some(('S', 42)));
        assert_eq!(parse_stat("43 (a) b) (c) Z 42 7 7 0"), Some(('Z', 7)));
        assert_eq!(parse_stat("44 (x y) R 1 -1 0"), Some(('R', -1)));
        assert_eq!(parse_stat("garbage"), None);
        assert_eq!(parse_stat("45 (x) S 1"), None);
    }

    #[cfg(target_os = "linux")]
    #[test]
    fn finds_live_members_of_a_group() {
        use std::os::unix::process::CommandExt;
        let mut child = std::process::Command::new("sleep")
            .arg("30")
            .process_group(0)
            .spawn()
            .expect("spawn sleep");
        let group = i32::try_from(child.id()).expect("pid");
        assert_eq!(live_members(group).expect("scan"), vec![group]);
        contain(group).expect("contained");
        child.wait().expect("reap");
        assert!(live_members(group).expect("scan").is_empty());
    }

    #[test]
    fn refuses_a_state_directory_inside_the_sandbox() {
        let base = std::env::temp_dir().join(format!("sandbox-exec-test-{}", std::process::id()));
        let root = canonical(&base).expect("temp dir");
        let executor = ProcessExecutor::new("/nonexistent".into(), vec![], root.join("state"));
        let result = executor.exec(&spec(&root), "true", &[]);
        assert!(matches!(result, Err(Error::Sandbox(_))), "{result:?}");
        std::fs::remove_dir_all(&base).expect("cleanup");
    }
}
