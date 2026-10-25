//! Detached moderator supervision. BBP's own lock is the authority.
use crate::{Config, SCRUB};
use std::fs::{File, OpenOptions};
use std::path::Path;
use std::process::{Command, Stdio};

/// Starting is harmless to race: only the BBP moderator holding its lock runs.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Supervised {
    Running,
    Started,
}
/// Long-lived process seam, separate from finite CommandRunner invocations.
pub trait Supervisor: std::fmt::Debug {
    /// Start the task's moderator unless one runs. With a `deadline`, in
    /// unix seconds, BBP has every harness of the task gone by then.
    fn ensure(
        &mut self,
        config: &Config,
        task: &str,
        agents: &Path,
        deadline: Option<u64>,
    ) -> Result<Supervised, String>;
}
/// Supervisor for supported Linux sandboxed workloads. Never falls back to none.
#[derive(Debug, Default)]
pub struct StdSupervisor;
impl Supervisor for StdSupervisor {
    fn ensure(
        &mut self,
        config: &Config,
        task: &str,
        agents: &Path,
        deadline: Option<u64>,
    ) -> Result<Supervised, String> {
        if !cfg!(target_os = "linux") {
            return Err("BBP sandboxed hosting requires Linux".into());
        }
        // Host health replays BBP's existing log. A paused launch loop must not
        // restart on a new Orch process; finite CLI diagnostics preserve the card.
        let argv = vec![
            config.binary.to_string_lossy().into_owned(),
            "mod-health".into(),
            "--dir".into(),
            config.dir.to_string_lossy().into_owned(),
            "--task".into(),
            task.into(),
            "--check".into(),
            "yes".into(),
        ];
        let health = orch_cli::CommandRunner::run_in(
            &orch_cli::StdCommand,
            None,
            &argv,
            b"",
            std::time::Duration::from_secs(5),
            SCRUB,
        )
        .map_err(|e| e.to_string())?;
        if health.status != 0 {
            return Err(format!(
                "BBP moderator health: {}",
                String::from_utf8_lossy(&health.stderr)
            ));
        }
        let key = rusty_bbp::Sha256::of(task.as_bytes()).hex();
        let dir = config.dir.join("mod");
        if locked(&dir, &key)? {
            return Ok(Supervised::Running);
        }
        let log = OpenOptions::new()
            .create(true)
            .append(true)
            .open(dir.join(format!("{key}.log")))
            .map_err(|e| e.to_string())?;
        let err: File = log.try_clone().map_err(|e| e.to_string())?;
        let mut command = Command::new(&config.binary);
        command
            .args(["mod", "--dir"])
            .arg(&config.dir)
            .args(["--task", task, "--repo-path"])
            .arg(&config.repo)
            .arg("--work")
            .arg(task_work(config, task))
            .arg("--agents")
            .arg(agents)
            .args([
                "--confine",
                "sandbox",
                "--max-wall-secs",
                &config.max_wall_secs.to_string(),
            ]);
        if let Some(slots) = &config.slots {
            command
                .arg("--slots")
                .arg(&slots.dir)
                .args(["--agent-slots", &slots.agents.to_string()])
                .args(["--validator-slots", &slots.validators.to_string()])
                .arg("--claim")
                .arg(agents.with_file_name(crate::CLAIM));
        }
        if let Some(secs) = config.end_grace_secs {
            command.args(["--end-grace-secs", &secs.to_string()]);
        }
        if let Some(unix) = deadline {
            command.args(["--deadline-unix", &unix.to_string()]);
        }
        command.stdin(Stdio::null()).stdout(log).stderr(err);
        orch_cli::scrub_env(&mut command, SCRUB);
        #[cfg(unix)]
        {
            use std::os::unix::process::CommandExt;
            command.process_group(0);
        }
        let mut child = command.spawn().map_err(|e| e.to_string())?;
        let pid = child.id();
        // Observe BBP's lock, not merely a pid that could already have exited.
        let deadline = std::time::Instant::now() + std::time::Duration::from_secs(2);
        loop {
            if locked(&dir, &key)? {
                break;
            }
            if let Some(status) = child.try_wait().map_err(|e| e.to_string())? {
                return Err(format!("BBP moderator exited {status}; see its task log"));
            }
            if std::time::Instant::now() >= deadline {
                let _ = child.kill();
                let _ = child.wait();
                return Err("BBP moderator did not acquire its task lock".into());
            }
            std::thread::sleep(std::time::Duration::from_millis(10));
        }
        // Reap if this process survives. Exiting Orch deliberately leaves BBP alive.
        std::thread::spawn(move || {
            let _ = child.wait();
        });
        rusty_atomic_file::write(&dir.join(format!("{key}.pid")), pid.to_string().as_bytes())
            .map_err(|e| e.to_string())?;
        Ok(Supervised::Started)
    }
}

/// Whether some process holds `<dir>/<key>.lock`, creating the file (and
/// `dir`) when missing, as BBP's moderator does.
fn locked(dir: &Path, key: &str) -> Result<bool, String> {
    std::fs::create_dir_all(dir).map_err(|e| e.to_string())?;
    let probe = OpenOptions::new()
        .create(true)
        .truncate(false)
        .write(true)
        .open(dir.join(format!("{key}.lock")))
        .map_err(|e| e.to_string())?;
    match probe.try_lock() {
        Err(std::fs::TryLockError::WouldBlock) => Ok(true),
        Err(std::fs::TryLockError::Error(e)) => Err(e.to_string()),
        Ok(()) => Ok(false),
    }
}

/// Stable per-task root: BBP run ids are local to a task, not the pool.
pub(crate) fn task_work(config: &Config, task: &str) -> std::path::PathBuf {
    config
        .work
        .join(rusty_bbp::Sha256::of(task.as_bytes()).hex())
}
