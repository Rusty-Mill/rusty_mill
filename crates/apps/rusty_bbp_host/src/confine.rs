//! Confined harness launch.
//!
//! A launcher with a `sandbox` runs its harness under `rusty_sandbox`, through
//! this binary's own `__sandbox` helper: Landlock limits the filesystem to the
//! system directories, the `bbp` binary, the launcher's declared roots and a
//! fresh turn workspace under `--work`; seccomp allows internet sockets only,
//! so no local daemon can act for it; the environment is exactly what is
//! declared here. The task directory is never reachable, so the harness can
//! read neither `master` nor the log and cannot run `bbp human`. Its MCP
//! server is a relay to a broker the moderator starts outside the sandbox
//! (see [`crate::broker`]). A sandbox the kernel cannot enforce means the
//! helper exits before the program runs: confinement fails closed.

use crate::broker;
use crate::moderator::{task_dir, Config};
use rusty_bbp::{PrincipalId, Sha256, TurnId};
use rusty_sandbox::{HelperRequest, Limits, SandboxSpec, Sockets};
use rusty_serde::{Deserialize, Serialize};
use rusty_worktree::Repo;
use std::path::{Path, PathBuf};
use std::process::{Command, Stdio};
use std::time::Duration;

const MEMORY_BYTES: u64 = 8 << 30;
const FILE_BYTES: u64 = 1 << 30;
const OPEN_FILES: u64 = 4_096;
/// Per user on Linux, so a brake rather than a quota.
const PROCESSES: u64 = 4_096;
const SYSTEM_ROOTS: [&str; 6] = ["/usr", "/bin", "/sbin", "/lib", "/lib64", "/etc"];

/// What a launcher's harness may reach beyond its turn workspace.
#[derive(Clone, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct HarnessSandbox {
    /// Absolute paths it may read and execute: the provider's installation
    /// and its login, for example.
    #[rusty_serde(default)]
    pub read: Vec<String>,
    /// Absolute paths it may also write.
    #[rusty_serde(default)]
    pub write: Vec<String>,
    /// Names of the moderator's environment variables it keeps. `PATH`
    /// defaults to `/usr/bin:/bin`, and `HOME` and `TMPDIR` to a scratch
    /// directory beside the workspace.
    #[rusty_serde(default)]
    pub env: Vec<String>,
    /// Make the workspace a git worktree of `--repo-path`, detached at its
    /// `HEAD`. Worktrees of every turn and task share the repository's
    /// object store, which the harness may read but not write; each harness
    /// may write only its own worktree.
    #[rusty_serde(default)]
    pub checkout: bool,
}

/// One confined turn's places: the workspace it may write, the pipes of its
/// MCP relay, and the helper's setup-status file, which it cannot reach.
pub(crate) struct Turn {
    pub workspace: PathBuf,
    /// `HOME` and `TMPDIR`, kept out of the workspace so a checkout's diff
    /// shows only the harness's edits.
    scratch: PathBuf,
    /// The repository the workspace is a worktree of, with `checkout`.
    repo: Option<Repo>,
    pub pipes: PathBuf,
    pub mcp_config: PathBuf,
    status: PathBuf,
}

impl Turn {
    /// Make the workspace and the pipes afresh and write the turn's MCP
    /// configuration into the workspace.
    pub fn prepare(cfg: &Config, sandbox: &HarnessSandbox, turn: TurnId) -> Result<Turn, String> {
        // The sandbox takes absolute roots only; a relative `--work` is
        // resolved against the moderator's working directory.
        let work = std::path::absolute(&cfg.work)
            .map_err(|e| format!("work {}: {e}", cfg.work.display()))?;
        let base = work
            .join("turns")
            .join(Sha256::of(cfg.task.0.as_bytes()).hex());
        let workspace = base.join(format!("turn-{}", turn.0));
        let pipes = base.join(format!("turn-{}.mcp", turn.0));
        let scratch = base.join(format!("turn-{}.home", turn.0));
        let status =
            task_dir(&cfg.dir, "agents", &cfg.task).join(format!("turn-{}.sandbox", turn.0));
        for dir in [&base, &scratch] {
            std::fs::create_dir_all(dir).map_err(|e| format!("{}: {e}", dir.display()))?;
        }
        let repo = if sandbox.checkout {
            let repo = Repo::open(&cfg.repo)?;
            repo.add_detached(&workspace, "HEAD")?;
            Some(repo)
        } else {
            std::fs::create_dir_all(&workspace)
                .map_err(|e| format!("{}: {e}", workspace.display()))?;
            None
        };
        broker::make_pipes(&pipes)?;
        let mcp_config = scratch.join("mcp.json");
        let text = format!(
            "{{\"mcpServers\":{{\"bbp\":{{\"command\":{},\"args\":[\"mcp\"],\"env\":{{{}:{}}}}}}}}}\n",
            json(&cfg.bbp.to_string_lossy())?,
            json(broker::RELAY_ENV)?,
            json(&pipes.to_string_lossy())?,
        );
        rusty_atomic_file::write(&mcp_config, text.as_bytes())
            .map_err(|e| format!("{}: {e}", mcp_config.display()))?;
        if let Some(parent) = status.parent() {
            std::fs::create_dir_all(parent).map_err(|e| e.to_string())?;
        }
        std::fs::write(&status, b"").map_err(|e| format!("{}: {e}", status.display()))?;
        Ok(Turn {
            workspace,
            scratch,
            repo,
            pipes,
            mcp_config,
            status,
        })
    }

    /// `program args` under the helper, with exactly `env` plus what
    /// `sandbox` keeps. Refused when any granted path reaches the task
    /// directory.
    pub fn command(
        &self,
        cfg: &Config,
        sandbox: &HarnessSandbox,
        program: String,
        args: Vec<String>,
        mut env: Vec<(String, String)>,
    ) -> Result<Command, String> {
        for name in &sandbox.env {
            if let Ok(value) = std::env::var(name) {
                env.push((name.clone(), value));
            }
        }
        let scratch = self.scratch.to_string_lossy().into_owned();
        for (name, default) in [
            ("PATH", "/usr/bin:/bin"),
            ("HOME", scratch.as_str()),
            ("TMPDIR", scratch.as_str()),
        ] {
            if !env.iter().any(|(k, _)| k == name) {
                env.push((name.into(), default.into()));
            }
        }
        let (requests, replies) = broker::pipes(&self.pipes);
        let bbp = canonical(&cfg.bbp)?;
        let mut read: Vec<PathBuf> = SYSTEM_ROOTS
            .iter()
            .map(PathBuf::from)
            .filter(|p| p.exists())
            .collect();
        read.push(bbp.parent().ok_or("bbp binary has no directory")?.into());
        read.push(replies);
        read.extend(self.repo.as_ref().map(Repo::git_dir));
        read.extend(sandbox.read.iter().map(PathBuf::from));
        let mut write = vec![
            self.workspace.clone(),
            self.scratch.clone(),
            requests,
            PathBuf::from("/dev/null"),
        ];
        write.extend(sandbox.write.iter().map(PathBuf::from));
        let cpu = cfg
            .max_wall
            .clamp(Duration::from_secs(1), Duration::from_secs(7 * 24 * 3600));
        let limits = Limits::new(cpu, cpu, MEMORY_BYTES, FILE_BYTES, OPEN_FILES, PROCESSES)
            .map_err(|e| e.to_string())?;
        let spec = SandboxSpec::new(read, write, self.workspace.clone(), env, limits)
            .map_err(|e| format!("harness sandbox: {e}"))?;
        refuse_reaching(&spec, &cfg.dir)?;
        let request = HelperRequest {
            status: self.status.clone(),
            cwd: spec.cwd().clone(),
            cpu_secs: limits.cpu().as_secs(),
            memory_bytes: limits.memory_bytes(),
            file_bytes: limits.file_bytes(),
            open_files: limits.open_files(),
            processes: limits.processes(),
            read: spec.read_roots().to_vec(),
            write: spec.write_roots().to_vec(),
            env: spec.env().to_vec(),
            sockets: Sockets::InternetOnly,
            program: program.into(),
            args: args.into_iter().map(Into::into).collect(),
        };
        let mut command = Command::new(&cfg.bbp);
        command.arg("__sandbox").args(request.encode()).env_clear();
        Ok(command)
    }

    /// Start the broker for this turn, in `harness`'s process group so that
    /// killing the harness's group kills it too.
    pub fn broker(
        &self,
        cfg: &Config,
        principal: &PrincipalId,
        turn: TurnId,
        harness: u32,
        log: Stdio,
    ) -> Result<std::process::Child, String> {
        let mut command = Command::new(&cfg.bbp);
        command
            .arg("mcp-broker")
            .arg("--pipes")
            .arg(&self.pipes)
            .args(["--harness", &harness.to_string()])
            .env("BBP_DIR", &cfg.dir)
            .env("BBP_TASK", &cfg.task.0)
            .env("BBP_PRINCIPAL", &principal.0)
            .env("BBP_TURN", turn.0.to_string())
            .stdin(Stdio::null())
            .stdout(Stdio::null())
            .stderr(log);
        #[cfg(unix)]
        std::os::unix::process::CommandExt::process_group(&mut command, harness as i32);
        command
            .spawn()
            .map_err(|e| format!("starting the MCP broker: {e}"))
    }

    /// Remove the turn's worktree, if it has one: its edits reached the task
    /// through MCP, and the object store keeps nothing of them.
    pub fn finish(&self) -> Result<(), String> {
        self.repo
            .as_ref()
            .map_or(Ok(()), |repo| repo.remove(&self.workspace))
    }

    /// Why the helper refused to run the harness, if it did.
    pub fn setup_error(&self) -> Option<String> {
        std::fs::read_to_string(&self.status)
            .ok()
            .filter(|s| !s.trim().is_empty())
    }
}

/// Refuse a sandbox from which `dir` (or anything in it) can be reached.
fn refuse_reaching(spec: &SandboxSpec, dir: &Path) -> Result<(), String> {
    let dir = canonical(dir)?;
    let roots = spec.read_roots().iter().chain(spec.write_roots());
    for root in roots {
        let root = canonical(root).unwrap_or_else(|_| root.clone());
        if dir.starts_with(&root) || root.starts_with(&dir) {
            return Err(format!(
                "harness sandbox: {} would expose the task directory {}",
                root.display(),
                dir.display()
            ));
        }
    }
    Ok(())
}

fn canonical(path: &Path) -> Result<PathBuf, String> {
    path.canonicalize()
        .map_err(|e| format!("{}: {e}", path.display()))
}

fn json(s: &str) -> Result<String, String> {
    rusty_serde::json::to_string(&s).map_err(|e| e.to_string())
}
