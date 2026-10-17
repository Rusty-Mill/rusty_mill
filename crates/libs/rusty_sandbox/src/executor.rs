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
        let outcome = spawn(self, &request, stdin).and_then(|job| job.wait(spec.limits().wall()));
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

impl ProcessExecutor {
    /// Starts `program` like [`ProcessExecutor::exec_with`] but returns the
    /// running [`Job`] instead of waiting for it, for a long-lived process
    /// (a bot's agent) that something else decides to stop. The sandbox
    /// setup is checked the same way: a helper that could not confine
    /// itself is an error here, before the job is handed back.
    ///
    /// # Errors
    /// As [`Executor::exec`].
    pub fn start(
        &self,
        spec: &SandboxSpec,
        program: &str,
        args: &[String],
        stdin: Stdio,
        sockets: Sockets,
    ) -> Result<Job, Error> {
        self.start_registered(spec, program, args, stdin, sockets, |_| Ok(()))
    }

    /// Like [`Self::start`], registering the job's cancellation handle immediately
    /// after spawn, before any fallible setup check or polling. A registration or
    /// setup error kills, contains and reaps the job before returning. Dropping a
    /// returned job without waiting also cleans it up.
    ///
    /// # Errors
    /// As [`Self::start`], or the error returned by `register`.
    #[allow(clippy::too_many_arguments)]
    pub fn start_registered(
        &self,
        spec: &SandboxSpec,
        program: &str,
        args: &[String],
        stdin: Stdio,
        sockets: Sockets,
        register: impl FnOnce(JobHandle) -> Result<(), Error>,
    ) -> Result<Job, Error> {
        self.start_setup(
            spec,
            program,
            args,
            stdin,
            sockets,
            register,
            SETUP_DEADLINE,
        )
    }

    #[allow(clippy::too_many_arguments)]
    fn start_setup(
        &self,
        spec: &SandboxSpec,
        program: &str,
        args: &[String],
        stdin: Stdio,
        sockets: Sockets,
        register: impl FnOnce(JobHandle) -> Result<(), Error>,
        setup_deadline: Duration,
    ) -> Result<Job, Error> {
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
        let mut job = spawn(self, &request, stdin)?;
        let checked = (|| {
            register(job.handle())?;
            // The helper either failed setup (and exited with the reason in the
            // status file) or exec'd the program (and the file stays empty).
            // Either way the file is closed on exec, so once the helper is past
            // setup there is nothing more to read: poll until it is gone or the
            // process has ended.
            let helper = self
                .helper
                .canonicalize()
                .map_err(|e| Error::io("resolving the helper", e))?;
            let setup_error = job.setup_error(&status, &helper, setup_deadline)?;
            let removed = std::fs::remove_file(&status);
            if !setup_error.is_empty() {
                return Err(Error::Sandbox(setup_error));
            }
            removed.map_err(|e| Error::io("removing the status file", e))?;
            Ok(())
        })();
        if let Err(error) = checked {
            return match job.cleanup() {
                Ok(()) => Err(error),
                Err(cleanup) => Err(Error::Sandbox(format!(
                    "{error}; cleanup failed: {cleanup}"
                ))),
            };
        }
        Ok(job)
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

/// A sandboxed process group that has been started and not yet reaped.
///
/// Owned by whoever started it; [`Job::wait`] reaps it, and a
/// [`JobHandle`] can kill it from anywhere. Dropping an unwaited job kills,
/// contains and reaps it before returning.
#[derive(Debug)]
pub struct Job {
    #[cfg(target_os = "linux")]
    inner: linux::Running,
}

/// A way to stop a [`Job`] from another thread: the group id, nothing
/// else, so it is `Copy` and outlives nothing.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct JobHandle {
    group: i32,
}

impl JobHandle {
    /// Sends `SIGKILL` to the whole group. A group that is already gone is
    /// fine; the job's [`Job::wait`] then reports the signal.
    ///
    /// # Errors
    /// [`Error::Sandbox`] if the kill itself fails.
    pub fn kill(&self) -> Result<(), Error> {
        kill_group(self.group)
    }

    /// The process group id, for logs.
    #[must_use]
    pub const fn group(&self) -> i32 {
        self.group
    }
}

#[cfg(not(target_os = "linux"))]
fn spawn(
    _executor: &ProcessExecutor,
    _request: &HelperRequest,
    _stdin: Stdio,
) -> Result<Job, Error> {
    Err(Error::Sandbox(
        "sandboxed execution is only implemented on Linux".into(),
    ))
}

#[cfg(not(target_os = "linux"))]
impl Job {
    /// Never constructed off Linux.
    pub fn handle(&self) -> JobHandle {
        JobHandle { group: 0 }
    }

    fn cleanup(&mut self) -> Result<(), Error> {
        Ok(())
    }

    /// Never constructed off Linux.
    pub fn wait(self, _wall: Duration) -> Result<ExecOutcome, Error> {
        Err(Error::Sandbox(
            "sandboxed execution is only implemented on Linux".into(),
        ))
    }

    fn setup_error(
        &mut self,
        _status: &Path,
        _helper: &Path,
        _deadline: Duration,
    ) -> Result<String, Error> {
        Ok(String::new())
    }
}

#[cfg(not(target_os = "linux"))]
fn kill_group(_group: i32) -> Result<(), Error> {
    Ok(())
}

#[cfg(target_os = "linux")]
fn spawn(executor: &ProcessExecutor, request: &HelperRequest, stdin: Stdio) -> Result<Job, Error> {
    use std::os::unix::process::CommandExt;
    use std::process::Command;

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
    let group = match i32::try_from(child.id()) {
        Ok(group) => group,
        Err(error) => {
            // No group id is representable, but we still own and must reap
            // the spawned child. Linux pids normally fit in i32.
            child
                .kill()
                .map_err(|e| Error::io("killing the helper", e))?;
            child
                .wait()
                .map_err(|e| Error::io("reaping the helper", e))?;
            return Err(Error::Sandbox(format!(
                "child process id out of range: {error}"
            )));
        }
    };
    // Arm ownership before starting capture threads too: a thread-spawn panic
    // must not abandon a helper already running.
    let mut job = Job {
        inner: linux::Running {
            child,
            group,
            stdout: Capture(None),
            stderr: Capture(None),
            start,
            cleaned: false,
        },
    };
    job.inner.stdout = capture(job.inner.child.stdout.take(), executor.capture_bytes);
    job.inner.stderr = capture(job.inner.child.stderr.take(), executor.capture_bytes);
    Ok(job)
}

#[cfg(target_os = "linux")]
impl Job {
    fn cleanup(&mut self) -> Result<(), Error> {
        self.inner.cleanup()
    }

    /// A handle that can kill this job from another thread.
    #[must_use]
    pub fn handle(&self) -> JobHandle {
        JobHandle {
            group: self.inner.group,
        }
    }

    /// Waits for the job to end, killing the whole group at `wall`, and
    /// contains whatever it left running.
    ///
    /// # Errors
    /// [`Error::Io`] when the process cannot be waited on; [`Error::Sandbox`]
    /// when a member of the group survives being killed.
    pub fn wait(mut self, wall: Duration) -> Result<ExecOutcome, Error> {
        use std::os::unix::process::ExitStatusExt;

        use crate::spec::Termination;

        let running = &mut self.inner;
        let group = running.group;
        let start = running.start;
        let mut timed_out = false;
        let status = loop {
            if let Some(status) = running
                .child
                .try_wait()
                .map_err(|e| Error::io("waiting for the sandboxed process", e))?
            {
                break status;
            }
            if start.elapsed() >= wall {
                timed_out = true;
                kill_group(group)?;
                break running
                    .child
                    .wait()
                    .map_err(|e| Error::io("reaping the timed-out process", e))?;
            }
            std::thread::sleep(Duration::from_millis(5));
        };
        let elapsed = start.elapsed();
        // Anything the program left running dies with it. The sandbox forbids
        // leaving the group (helper::group_lock), so the group is the job.
        contain(group)?;
        running.cleaned = true;

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
            stdout: running.stdout.finish(),
            stderr: running.stderr.finish(),
            wall: elapsed,
        })
    }

    /// What the helper wrote to `status` before `exec`: empty once it has
    /// exec'd the program (its image is no longer `helper`) or exited.
    /// Polls until one of those; a helper that has not opened the file yet
    /// must not find it gone.
    fn setup_error(
        &mut self,
        status: &Path,
        helper: &Path,
        timeout: Duration,
    ) -> Result<String, Error> {
        let deadline = std::time::Instant::now() + timeout;
        loop {
            let ended = self
                .inner
                .child
                .try_wait()
                .map_err(|e| Error::io("waiting for the sandbox helper", e))?
                .is_some();
            if ended || linux::has_execed(self.inner.group, helper) {
                return std::fs::read_to_string(status)
                    .map_err(|e| Error::io("reading the status file", e));
            }
            if std::time::Instant::now() >= deadline {
                return Err(Error::Sandbox(
                    "the sandbox helper did not finish setting up".into(),
                ));
            }
            std::thread::sleep(Duration::from_millis(5));
        }
    }
}

/// How long a helper gets to confine itself and `exec` before the job is
/// refused.
const SETUP_DEADLINE: Duration = Duration::from_secs(10);

#[cfg(target_os = "linux")]
mod linux {
    use std::path::Path;

    use super::Capture;

    #[derive(Debug)]
    pub(super) struct Running {
        pub(super) child: std::process::Child,
        pub(super) group: i32,
        pub(super) stdout: Capture,
        pub(super) stderr: Capture,
        pub(super) start: std::time::Instant,
        pub(super) cleaned: bool,
    }

    impl Running {
        pub(super) fn cleanup(&mut self) -> Result<(), super::Error> {
            if self.cleaned {
                return Ok(());
            }
            // Attempt all cleanup operations even if an earlier one fails.
            let killed = super::kill_group(self.group);
            let contained = super::contain(self.group);
            let reaped = self
                .child
                .wait()
                .map_err(|e| super::Error::io("reaping the helper", e));
            let _stdout = self.stdout.finish();
            let _stderr = self.stderr.finish();
            killed.and(contained).and(reaped)?;
            self.cleaned = true;
            Ok(())
        }
    }

    impl Drop for Running {
        fn drop(&mut self) {
            // Armed before startup checks and on errors from wait(). Explicit
            // startup cleanup propagates failures; Drop also covers unwinding
            // and callers that abandon a successfully started job.
            if let Err(error) = self.cleanup() {
                eprintln!("sandbox cleanup: {error}");
            }
        }
    }

    /// Whether the group leader has replaced the helper's image: its
    /// `/proc/<pid>/exe` no longer points at `helper`. A process that is
    /// gone reads as not exec'd; the caller checks for exit separately.
    pub(super) fn has_execed(group: i32, helper: &Path) -> bool {
        std::fs::read_link(format!("/proc/{group}/exe")).is_ok_and(|exe| exe != helper)
    }
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
#[derive(Debug)]
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
    fn finish(&mut self) -> Vec<u8> {
        let Some(receiver) = self.0.take() else {
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

    #[cfg(target_os = "linux")]
    #[test]
    fn a_started_job_can_be_killed_from_a_handle() {
        // `sh -c "exec sleep 30"` as the "helper": it ignores the request
        // arguments, never touches the status file, and replaces its image
        // the way a real helper does, then runs until killed. Enough to
        // exercise start, the exec check, the handle and wait.
        let base = std::env::temp_dir().join(format!("sandbox-job-test-{}", std::process::id()));
        let root = canonical(&base.join("work")).expect("temp dir");
        let executor = ProcessExecutor::new(
            "/bin/sh".into(),
            vec!["-c".into(), "exec sleep 30".into()],
            base.join("state"),
        );
        let job = executor
            .start(
                &spec(&root),
                "unused",
                &[],
                Stdio::null(),
                Sockets::NoInternet,
            )
            .expect("started");
        let handle = job.handle();
        let waiter = std::thread::spawn(move || job.wait(Duration::from_secs(20)));
        std::thread::sleep(Duration::from_millis(100));
        handle.kill().expect("killed");
        let outcome = waiter.join().expect("thread").expect("waited");
        assert_eq!(outcome.termination, crate::spec::Termination::Signaled(9));
        assert!(
            outcome.wall < Duration::from_secs(5),
            "ended by the kill, not the wall"
        );
        std::fs::remove_dir_all(&base).expect("cleanup");
    }

    // Synthetic helpers exercise the real spawn/setup/contain/reap path even
    // when the host kernel cannot enforce Landlock. They deliberately postpone
    // exec, then would launch a workload if the executor abandoned them.
    #[cfg(target_os = "linux")]
    fn delayed_helper(name: &str) -> (PathBuf, ProcessExecutor, SandboxSpec) {
        let base = std::env::temp_dir().join(format!(
            "sandbox-start-{name}-{}-{}",
            std::process::id(),
            NEXT_RUN.fetch_add(1, Ordering::Relaxed)
        ));
        let root = canonical(&base.join("work")).expect("work");
        let helper = base.join("helper");
        std::os::unix::fs::symlink("/bin/sh", &helper).expect("helper link");
        let executor = ProcessExecutor::new(helper, vec![
            "-c".into(),
            "echo $$ > \"$1/helper.pid\"; sleep 30 & echo $! > \"$1/child.pid\"; while [ ! -f \"$1/launch\" ]; do sleep 0.005; done; echo late > \"$1/launched\"; exec sleep 30".into(),
            "helper".into(), root.clone().into_os_string(),
        ], base.join("state"));
        (base, executor, spec(&root))
    }

    #[cfg(target_os = "linux")]
    fn helper_ready(root: &Path) -> Vec<i32> {
        let until = std::time::Instant::now() + Duration::from_secs(3);
        loop {
            let pids: Option<Vec<i32>> = ["helper.pid", "child.pid"]
                .iter()
                .map(|file| {
                    std::fs::read_to_string(root.join(file))
                        .ok()?
                        .trim()
                        .parse()
                        .ok()
                })
                .collect();
            if let Some(pids) = pids {
                return pids;
            }
            assert!(std::time::Instant::now() < until, "helper did not start");
            std::thread::sleep(Duration::from_millis(2));
        }
    }

    #[cfg(target_os = "linux")]
    fn assert_start_cleaned(pids: &[i32], root: &Path) {
        assert!(live_members(pids[0]).expect("members").is_empty());
        // The direct helper must have been reaped, not merely killed.
        assert!(
            !Path::new(&format!("/proc/{}", pids[0])).exists(),
            "helper was not reaped"
        );
        std::fs::write(root.join("launch"), "").expect("allow delayed launch");
        std::thread::sleep(Duration::from_millis(100));
        assert!(
            !root.join("launched").exists(),
            "workload launched after failure"
        );
        assert!(live_members(pids[0]).expect("members").is_empty());
    }

    #[cfg(target_os = "linux")]
    #[test]
    fn startup_timeout_contains_and_reaps_before_returning() {
        let (base, executor, spec) = delayed_helper("timeout");
        let mut pids = vec![];
        let result = executor.start_setup(
            &spec,
            "unused",
            &[],
            Stdio::null(),
            Sockets::NoEndpoints,
            |_| {
                pids = helper_ready(spec.cwd());
                Ok(())
            },
            Duration::from_millis(50),
        );
        assert!(result
            .unwrap_err()
            .to_string()
            .contains("did not finish setting up"));
        assert_start_cleaned(&pids, spec.cwd());
        std::fs::remove_dir_all(base).expect("cleanup");
    }

    #[cfg(target_os = "linux")]
    #[test]
    fn post_spawn_registration_and_resolution_errors_clean_up() {
        for resolution in [false, true] {
            let (base, executor, spec) = delayed_helper(if resolution {
                "resolution"
            } else {
                "registration"
            });
            let mut pids = vec![];
            let result = executor.start_registered(
                &spec,
                "unused",
                &[],
                Stdio::null(),
                Sockets::NoEndpoints,
                |_| {
                    pids = helper_ready(spec.cwd());
                    if resolution {
                        std::fs::remove_file(&executor.helper).expect("remove helper link");
                        Ok(())
                    } else {
                        Err(Error::Sandbox("forced registration error".into()))
                    }
                },
            );
            let error = result.unwrap_err().to_string();
            assert!(
                error.contains(if resolution {
                    "resolving the helper"
                } else {
                    "forced registration error"
                }),
                "{error}"
            );
            assert_start_cleaned(&pids, spec.cwd());
            std::fs::remove_dir_all(base).expect("cleanup");
        }
    }

    #[cfg(target_os = "linux")]
    #[test]
    fn status_read_and_setup_refusal_errors_contain_descendants() {
        for missing_status in [false, true] {
            let (base, mut executor, spec) = delayed_helper(if missing_status {
                "status-read"
            } else {
                "setup-refused"
            });
            executor.helper_args[1] = r#"echo $$ > "$1/helper.pid"; (while [ ! -f "$1/launch" ]; do sleep 0.005; done; echo late > "$1/launched"; sleep 30) & echo $! > "$1/child.pid"; while [ ! -f "$1/release" ]; do sleep 0.005; done; exit 1"#.into();
            let mut pids = vec![];
            let result = executor.start_registered(
                &spec,
                "unused",
                &[],
                Stdio::null(),
                Sockets::NoEndpoints,
                |_| {
                    pids = helper_ready(spec.cwd());
                    let status = std::fs::read_dir(&executor.state_dir)
                        .expect("state")
                        .next()
                        .expect("status entry")
                        .expect("entry")
                        .path();
                    if missing_status {
                        std::fs::remove_file(status).expect("remove status");
                    } else {
                        std::fs::write(status, "forced setup refusal").expect("setup reason");
                    }
                    std::fs::write(spec.cwd().join("release"), "").expect("release helper");
                    Ok(())
                },
            );
            let error = result.unwrap_err().to_string();
            assert!(
                error.contains(if missing_status {
                    "reading the status file"
                } else {
                    "forced setup refusal"
                }),
                "{error}"
            );
            assert_start_cleaned(&pids, spec.cwd());
            std::fs::remove_dir_all(base).expect("cleanup");
        }
    }

    #[cfg(target_os = "linux")]
    #[test]
    fn dropping_a_started_job_contains_and_reaps_it() {
        let (base, executor, spec) = delayed_helper("drop");
        let status = executor.status_file().expect("status");
        let request = HelperRequest {
            status,
            cwd: spec.cwd().clone(),
            cpu_secs: 1,
            memory_bytes: 1 << 28,
            file_bytes: 1 << 20,
            open_files: 32,
            processes: 16,
            read: vec![],
            write: vec![],
            env: vec![],
            sockets: Sockets::NoEndpoints,
            program: "unused".into(),
            args: vec![],
        };
        let job = spawn(&executor, &request, Stdio::null()).expect("spawn");
        let pids = helper_ready(spec.cwd());
        drop(job);
        assert_start_cleaned(&pids, spec.cwd());
        std::fs::remove_dir_all(base).expect("cleanup");
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
