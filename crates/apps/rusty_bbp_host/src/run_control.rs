//! Cancellation shared by the moderator and its current runner invocation.
//! The moderator joins the runner while holding its task lock, so a replacement
//! cannot execute the same pending run before workload cleanup has completed.
use crate::runner::{Confinement, Sandboxed};
use rusty_sandbox::{ExecOutcome, Executor, JobHandle, SandboxSpec, Sockets, Termination};
use std::io::Read;
use std::process::{Command, Stdio};
use std::sync::{Arc, Mutex};
use std::time::{Duration, Instant};

#[derive(Default)]
struct State {
    cancelled: bool,
    active: Option<Active>,
}
enum Active {
    Sandboxed(JobHandle),
    Unconfined(i32),
}
impl Active {
    fn kill(&self) -> Result<(), String> {
        match self {
            Self::Sandboxed(job) => job.kill().map_err(|e| e.to_string()),
            Self::Unconfined(group) => kill_group(*group),
        }
    }
}
/// Cancellation is latched before a kill, including when startup races with it.
#[derive(Clone, Default)]
pub(crate) struct RunControl(Arc<Mutex<State>>);
impl RunControl {
    pub(crate) fn cancel(&self) -> Result<(), String> {
        let mut state = self.0.lock().map_err(|_| "runner control lock poisoned")?;
        state.cancelled = true;
        match &state.active {
            Some(active) => active.kill(),
            None => Ok(()),
        }
    }
    fn cancelled(&self) -> Result<bool, String> {
        Ok(self
            .0
            .lock()
            .map_err(|_| "runner control lock poisoned")?
            .cancelled)
    }
    fn register(&self, active: Active) -> Result<(), String> {
        let mut state = self.0.lock().map_err(|_| "runner control lock poisoned")?;
        // Register even a startup concurrent with cancellation: the waiting
        // runner still owns and reaps the child after this kill.
        state.active = Some(active);
        if state.cancelled {
            if let Some(job) = &state.active {
                job.kill()?;
            }
        }
        Ok(())
    }
    fn clear(&self) -> Result<(), String> {
        self.0
            .lock()
            .map_err(|_| "runner control lock poisoned")?
            .active = None;
        Ok(())
    }
}
/// Reuses BBP's sandbox executor's start/handle/wait contract. The unconfined
/// branch exists only for the existing explicitly trusted CLI/test mode.
pub(crate) struct Controlled {
    pub(crate) control: RunControl,
    pub(crate) sandbox: Sandboxed,
    pub(crate) confinement: Confinement,
}
impl Executor for Controlled {
    type Error = String;
    fn exec(
        &self,
        spec: &SandboxSpec,
        program: &str,
        args: &[String],
    ) -> Result<ExecOutcome, String> {
        if self.control.cancelled()? {
            return Err("runner cancelled".into());
        }
        let result = match self.confinement {
            Confinement::Sandboxed => self
                .sandbox
                .0
                .start_registered(
                    spec,
                    program,
                    args,
                    Stdio::null(),
                    Sockets::NoEndpoints,
                    |handle| {
                        self.control
                            .register(Active::Sandboxed(handle))
                            .map_err(rusty_sandbox::Error::Sandbox)
                    },
                )
                .and_then(|job| job.wait(spec.limits().wall()))
                .map_err(|e| e.to_string()),
            Confinement::Unconfined => self.unconfined(spec, program, args),
        };
        self.control.clear()?;
        if self.control.cancelled()? {
            return Err("runner cancelled".into());
        }
        result
    }
}
impl Controlled {
    fn unconfined(
        &self,
        spec: &SandboxSpec,
        program: &str,
        args: &[String],
    ) -> Result<ExecOutcome, String> {
        if !cfg!(unix) {
            return Err("controlled unconfined execution requires Unix".into());
        }
        let mut command = Command::new(program);
        command
            .args(args)
            .current_dir(spec.cwd())
            .env_clear()
            .envs(spec.env().iter().cloned())
            .stdin(Stdio::null())
            .stdout(Stdio::piped())
            .stderr(Stdio::piped());
        #[cfg(unix)]
        {
            use std::os::unix::process::CommandExt;
            command.process_group(0);
        }
        let started = Instant::now();
        let mut child = command.spawn().map_err(|e| e.to_string())?;
        let group = i32::try_from(child.id()).map_err(|_| "runner pid out of range")?;
        let registered = self.control.register(Active::Unconfined(group));
        if registered.is_err() {
            let _ = kill_group(group);
        }
        let stdout = child.stdout.take().map(capture);
        let stderr = child.stderr.take().map(capture);
        let mut timed_out = false;
        let status = loop {
            match child.try_wait() {
                Ok(Some(status)) => break Ok(status),
                Ok(None) => {}
                Err(error) => break Err(error.to_string()),
            }
            if started.elapsed() >= spec.limits().wall() {
                timed_out = true;
                break kill_group(group).and_then(|_| child.wait().map_err(|e| e.to_string()));
            }
            std::thread::sleep(Duration::from_millis(5));
        };
        let killed = kill_group(group);
        let reaped = child.wait().map_err(|e| e.to_string());
        let stdout = collected(stdout);
        let stderr = collected(stderr);
        registered?;
        killed?;
        reaped?;
        let status = status?;
        #[cfg(unix)]
        use std::os::unix::process::ExitStatusExt;
        #[cfg(unix)]
        let signal = status.signal().unwrap_or(0);
        #[cfg(not(unix))]
        let signal = 0;
        Ok(ExecOutcome {
            termination: if timed_out {
                Termination::TimedOut
            } else {
                status
                    .code()
                    .map(Termination::Exited)
                    .unwrap_or(Termination::Signaled(signal))
            },
            stdout: stdout?,
            stderr: stderr?,
            wall: started.elapsed(),
        })
    }
}
fn capture(
    mut pipe: impl Read + Send + 'static,
) -> std::thread::JoinHandle<Result<Vec<u8>, String>> {
    std::thread::spawn(move || {
        let mut bytes = Vec::new();
        pipe.by_ref()
            .take(rusty_sandbox::DEFAULT_CAPTURE_BYTES as u64)
            .read_to_end(&mut bytes)
            .map_err(|e| e.to_string())?;
        std::io::copy(&mut pipe, &mut std::io::sink()).map_err(|e| e.to_string())?;
        Ok(bytes)
    })
}
fn collected(
    thread: Option<std::thread::JoinHandle<Result<Vec<u8>, String>>>,
) -> Result<Vec<u8>, String> {
    match thread {
        Some(thread) => thread.join().map_err(|_| "runner capture panicked")?,
        None => Ok(vec![]),
    }
}
fn kill_group(group: i32) -> Result<(), String> {
    #[cfg(target_os = "linux")]
    match rusty_libc::process::killpg(group, rusty_libc::signal::SIGKILL) {
        Ok(()) | Err(rusty_libc::Errno::ESRCH) => Ok(()),
        Err(e) => Err(format!("runner killpg({group}): {e:?}")),
    }
    #[cfg(not(target_os = "linux"))]
    {
        // Off Linux this is the explicitly unconfined Unix test path. ESRCH
        // after the leader exited is success, as for the Linux backend.
        let exists = Command::new("kill")
            .args(["-0", "--", &format!("-{group}")])
            .stderr(Stdio::null())
            .status()
            .map_err(|e| e.to_string())?;
        if !exists.success() {
            return Ok(());
        }
        let status = Command::new("kill")
            .args(["-KILL", "--", &format!("-{group}")])
            .status()
            .map_err(|e| e.to_string())?;
        if status.success() {
            Ok(())
        } else {
            Err("runner group kill failed".into())
        }
    }
}

/// Existing BBP runner as a separately owned process. The input pipe is held
/// only by its moderator; EOF latches cancellation even after moderator death.
pub fn supervised(
    dir: &std::path::Path,
    task: &rusty_bbp::TaskId,
    repo: &std::path::Path,
    work: &std::path::Path,
    confinement: Confinement,
) -> Result<rusty_bbp::Response, String> {
    let control = RunControl::default();
    let on_eof = control.clone();
    std::thread::Builder::new()
        .name("bbp-runner-owner".into())
        .spawn(move || {
            watch_owner(std::io::stdin().lock(), on_eof);
        })
        .map_err(|e| format!("watching runner owner: {e}"))?;
    let exec = Controlled {
        control,
        sandbox: crate::runner::sandboxed(&dir.join("sandbox-state"))?,
        confinement: confinement.clone(),
    };
    crate::runner::run_once(dir, task, repo, work, &exec, confinement)
}

fn watch_owner(mut owner: impl Read, control: RunControl) {
    let mut sink = std::io::sink();
    if let Err(error) = std::io::copy(&mut owner, &mut sink) {
        eprintln!("bbp runner: owner pipe failed: {error}");
    }
    if let Err(error) = control.cancel() {
        eprintln!("bbp runner: cancellation failed: {error}");
    }
}

#[cfg(all(test, target_os = "linux"))]
#[allow(clippy::expect_used, clippy::unwrap_used)]
mod tests {
    use super::*;
    use crate::runner::lock_task;
    use rusty_bbp::TaskId;
    use rusty_sandbox::{Limits, ProcessExecutor};
    use std::os::unix::net::UnixStream;

    #[test]
    fn owner_eof_during_setup_keeps_task_locked_through_cleanup() {
        for eof_before_spawn in [false, true] {
            let base = std::env::temp_dir().join(format!(
                "bbp-owner-setup-{}-{eof_before_spawn}",
                std::process::id()
            ));
            std::fs::create_dir_all(base.join("work")).expect("work");
            let root = base.join("work").canonicalize().expect("root");
            let control = RunControl::default();
            let (owner, reader) = UnixStream::pair().expect("owner pipe");
            let on_eof = control.clone();
            let watcher = std::thread::spawn(move || watch_owner(reader, on_eof));
            let task = TaskId("setup-owner-test".into());
            let lock = lock_task(&base, &task).expect("task lock");
            if eof_before_spawn {
                // Cancellation can latch between exec's initial check and spawn.
                // Exercise start_registered directly for that race below.
                drop(owner);
                watcher.join().expect("watcher");
                let executor = ProcessExecutor::new(
                    "/bin/sh".into(),
                    vec!["-c".into(), "exec sleep 30".into()],
                    base.join("state"),
                );
                let limits = Limits::new(
                    Duration::from_secs(1),
                    Duration::from_secs(5),
                    1 << 28,
                    1 << 20,
                    32,
                    16,
                )
                .expect("limits");
                let spec =
                    SandboxSpec::new(vec![], vec![root.clone()], root.clone(), vec![], limits)
                        .expect("spec");
                let mut group = 0;
                let job = executor
                    .start_registered(
                        &spec,
                        "unused",
                        &[],
                        Stdio::null(),
                        Sockets::NoEndpoints,
                        |handle| {
                            group = handle.group();
                            control
                                .register(Active::Sandboxed(handle))
                                .map_err(rusty_sandbox::Error::Sandbox)
                        },
                    )
                    .expect("killed startup can finish checking");
                assert!(lock_task(&base, &task).is_err());
                let outcome = job.wait(Duration::from_secs(5)).expect("cleanup");
                assert_ne!(outcome.termination, Termination::Exited(0));
                assert!(!std::path::Path::new(&format!("/proc/{group}")).exists());
                drop(lock);
            } else {
                let executor = Controlled {
                    control: control.clone(),
                    sandbox: Sandboxed(ProcessExecutor::new("/bin/sh".into(), vec!["-c".into(),
                        "echo $$ > \"$1/helper.pid\"; sleep 30 & echo $! > \"$1/child.pid\"; while [ ! -f \"$1/launch\" ]; do sleep 0.005; done; echo late > \"$1/launched\"; exec sleep 30".into(),
                        "helper".into(), root.clone().into_os_string()], base.join("state"))),
                    confinement: Confinement::Sandboxed,
                };
                let limits = Limits::new(
                    Duration::from_secs(1),
                    Duration::from_secs(5),
                    1 << 28,
                    1 << 20,
                    32,
                    16,
                )
                .expect("limits");
                let spec =
                    SandboxSpec::new(vec![], vec![root.clone()], root.clone(), vec![], limits)
                        .expect("spec");
                let worker = std::thread::spawn(move || {
                    let _lock = lock;
                    executor.exec(&spec, "unused", &[])
                });
                let until = Instant::now() + Duration::from_secs(3);
                let group = loop {
                    let active = control.0.lock().expect("control");
                    if let Some(Active::Sandboxed(handle)) = &active.active {
                        if std::fs::read_to_string(root.join("child.pid"))
                            .ok()
                            .is_some_and(|s| s.trim().parse::<i32>().is_ok())
                        {
                            break handle.group();
                        }
                    }
                    assert!(Instant::now() < until, "startup handle not registered");
                    drop(active);
                    std::thread::sleep(Duration::from_millis(2));
                };
                assert!(
                    lock_task(&base, &task).is_err(),
                    "startup released ownership"
                );
                drop(owner);
                watcher.join().expect("watcher");
                assert!(worker
                    .join()
                    .expect("worker")
                    .unwrap_err()
                    .contains("cancelled"));
                assert!(
                    !std::path::Path::new(&format!("/proc/{group}")).exists(),
                    "helper not reaped"
                );
                assert!(control.0.lock().expect("control").active.is_none());
                let child: i32 = std::fs::read_to_string(root.join("child.pid"))
                    .expect("child")
                    .trim()
                    .parse()
                    .expect("pid");
                if let Ok(stat) = std::fs::read_to_string(format!("/proc/{child}/stat")) {
                    let state = stat
                        .rsplit_once(')')
                        .expect("stat")
                        .1
                        .trim()
                        .chars()
                        .next()
                        .expect("state");
                    assert!(matches!(state, 'Z' | 'X'), "descendant survived: {stat}");
                }
            }
            let _replacement = lock_task(&base, &task).expect("ownership available after cleanup");
            std::fs::write(root.join("launch"), "").expect("allow delayed launch");
            std::thread::sleep(Duration::from_millis(100));
            assert!(
                !root.join("launched").exists(),
                "workload launched after owner EOF"
            );
            std::fs::remove_dir_all(base).expect("cleanup");
        }
    }
}
