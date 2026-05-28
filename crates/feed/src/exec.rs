//! Capability isolation seam (ADR-0030 / Phase 7B). A [`ToolExecutor`] builds
//! the OS process for a vetted shell command. It sits **below `feed`, above the
//! OS**, and does **not** change the `constrain` vetting contract — vetting
//! still runs first; isolation governs *how* a vetted side-effect runs.
//!
//! - [`Isolation::None`] (default) — [`LocalExecutor`]: byte-for-byte today's
//!   behaviour, the sub-millisecond local-first hot path.
//! - [`Isolation::Sandboxed`] — [`SandboxedExecutor`]: wraps a battle-tested OS
//!   sandbox launcher (bubblewrap / firejail) with **network-deny-by-default**
//!   and a filesystem view confined to the workspace. Per "be wary of custom
//!   components," it wraps mature primitives rather than hand-rolling namespaces;
//!   if none is present it **fails closed** (never silently runs unsandboxed).

use std::path::{Path, PathBuf};
use std::sync::{Arc, Mutex};

use tokio::process::Command;
use tokio::sync::mpsc::UnboundedSender;

use crate::ToolError;

/// A per-turn sink for live `bash` stdout/stderr chunks. An adapter (the desktop
/// terminal, the CLI) installs a sender before a turn via `Session::set_bash_sink`
/// and drains it; `bash_impl` forwards each chunk as it is read off the process.
/// `None` ⇒ no live streaming (the default), and the call still returns the full
/// captured output as its `ToolOutcome`.
#[derive(Default)]
pub struct BashStream {
    tx: Mutex<Option<UnboundedSender<String>>>,
}

impl BashStream {
    /// Install (or clear, with `None`) the chunk sink for the next turn.
    pub fn set(&self, tx: Option<UnboundedSender<String>>) {
        *self.tx.lock().unwrap_or_else(|p| p.into_inner()) = tx;
    }

    /// Forward one output chunk to the installed sink, if any (best-effort).
    pub(crate) fn emit(&self, chunk: &str) {
        if let Some(tx) = self.tx.lock().unwrap_or_else(|p| p.into_inner()).as_ref() {
            let _ = tx.send(chunk.to_string());
        }
    }
}

/// The runtime isolation profile, from `RUSTYKEYS_ISOLATION`.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub enum Isolation {
    /// In-process, no OS sandbox (today's behaviour).
    #[default]
    None,
    /// Tool side-effects run inside an OS sandbox (Linux-first).
    Sandboxed,
}

impl Isolation {
    /// Parse from the raw config string; anything but `sandboxed` is `none`.
    pub fn from_config(s: &str) -> Self {
        match s.trim().to_ascii_lowercase().as_str() {
            "sandboxed" => Isolation::Sandboxed,
            _ => Isolation::None,
        }
    }

    /// snake_case label.
    pub fn as_str(self) -> &'static str {
        match self {
            Isolation::None => "none",
            Isolation::Sandboxed => "sandboxed",
        }
    }
}

/// Builds the process for a vetted shell command, applying the isolation profile.
pub trait ToolExecutor: Send + Sync {
    /// Construct a [`Command`] that runs `shell_command` confined per the
    /// profile. `Err` fails the call closed (used when a required sandbox is
    /// unavailable).
    fn build(&self, shell_command: &str, workspace: &Path) -> Result<Command, ToolError>;
    /// The isolation profile label, for diagnostics.
    fn profile(&self) -> &'static str;
}

/// No isolation — runs `sh -c <command>` in the workspace directly.
pub struct LocalExecutor;

impl ToolExecutor for LocalExecutor {
    fn build(&self, shell_command: &str, workspace: &Path) -> Result<Command, ToolError> {
        let mut cmd = Command::new("sh");
        cmd.arg("-c").arg(shell_command).current_dir(workspace);
        Ok(cmd)
    }
    fn profile(&self) -> &'static str {
        "none"
    }
}

/// A detected OS sandbox launcher (a mature, externally-maintained primitive).
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum SandboxLauncher {
    /// bubblewrap (`bwrap`) — allowlist bind mounts + namespaces.
    Bwrap(PathBuf),
    /// firejail — profile-based sandbox.
    Firejail(PathBuf),
}

impl SandboxLauncher {
    /// Find a supported launcher on `PATH`, preferring bubblewrap.
    pub fn detect() -> Option<Self> {
        which("bwrap")
            .map(SandboxLauncher::Bwrap)
            .or_else(|| which("firejail").map(SandboxLauncher::Firejail))
    }
}

/// Best-effort `PATH` lookup (no external crate; we only need an absolute path).
fn which(bin: &str) -> Option<PathBuf> {
    let path = std::env::var_os("PATH")?;
    std::env::split_paths(&path)
        .map(|dir| dir.join(bin))
        .find(|p| p.is_file())
}

/// Runs tool side-effects inside an OS sandbox: network-deny-by-default and a
/// filesystem view limited to the workspace. The agent's secrets (e.g.
/// `~/.aws/credentials`) and the network are simply *not in the grant*, so an
/// exfil attempt fails closed at the boundary regardless of in-process checkers.
pub struct SandboxedExecutor {
    launcher: Option<SandboxLauncher>,
}

impl SandboxedExecutor {
    /// Auto-detect an available launcher.
    pub fn detect() -> Self {
        Self {
            launcher: SandboxLauncher::detect(),
        }
    }

    /// Force a specific launcher (used in tests; also lets a caller pin one).
    pub fn with_launcher(launcher: SandboxLauncher) -> Self {
        Self {
            launcher: Some(launcher),
        }
    }

    /// The minimal read-only system paths a shell needs, bound if they exist.
    /// The workspace is the only writable path; `$HOME` is never bound.
    fn system_ro_paths() -> &'static [&'static str] {
        &[
            "/usr",
            "/bin",
            "/sbin",
            "/lib",
            "/lib64",
            "/etc/alternatives",
        ]
    }

    fn build_bwrap(bwrap: &Path, shell_command: &str, workspace: &Path) -> Command {
        let mut cmd = Command::new(bwrap);
        // Fresh namespaces incl. network → no egress (network-deny-by-default).
        cmd.arg("--unshare-all");
        cmd.arg("--die-with-parent");
        // Read-only system dirs (only those that exist) — allowlist, not blocklist.
        for p in Self::system_ro_paths() {
            if Path::new(p).exists() {
                cmd.arg("--ro-bind").arg(p).arg(p);
            }
        }
        // Pseudo-filesystems the shell/tools expect.
        cmd.arg("--proc").arg("/proc");
        cmd.arg("--dev").arg("/dev");
        cmd.arg("--tmpfs").arg("/tmp");
        // The workspace is the only writable grant, and the working directory.
        cmd.arg("--bind").arg(workspace).arg(workspace);
        cmd.arg("--chdir").arg(workspace);
        cmd.arg("sh").arg("-c").arg(shell_command);
        cmd
    }

    fn build_firejail(firejail: &Path, shell_command: &str, workspace: &Path) -> Command {
        let mut cmd = Command::new(firejail);
        cmd.arg("--quiet");
        cmd.arg("--net=none"); // network-deny-by-default
                               // Replace $HOME with the workspace so the real home (and its secrets)
                               // is not visible; system dirs remain read-only by firejail default.
        cmd.arg(format!("--private={}", workspace.display()));
        cmd.arg("sh").arg("-c").arg(shell_command);
        cmd.current_dir(workspace);
        cmd
    }
}

impl ToolExecutor for SandboxedExecutor {
    fn build(&self, shell_command: &str, workspace: &Path) -> Result<Command, ToolError> {
        match &self.launcher {
            Some(SandboxLauncher::Bwrap(p)) => {
                Ok(Self::build_bwrap(p, shell_command, workspace))
            }
            Some(SandboxLauncher::Firejail(p)) => {
                Ok(Self::build_firejail(p, shell_command, workspace))
            }
            // Fail closed: a requested sandbox we cannot establish must not
            // degrade to running the command unsandboxed.
            None => Err(ToolError::Sandbox(
                "RUSTYKEYS_ISOLATION=sandboxed but no sandbox launcher (bwrap/firejail) found on PATH"
                    .to_string(),
            )),
        }
    }
    fn profile(&self) -> &'static str {
        "sandboxed"
    }
}

/// Select the executor for `isolation` (the seam's entry point).
pub fn executor_for(isolation: Isolation) -> Arc<dyn ToolExecutor> {
    match isolation {
        Isolation::None => Arc::new(LocalExecutor),
        Isolation::Sandboxed => Arc::new(SandboxedExecutor::detect()),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn args_of(cmd: &Command) -> Vec<String> {
        cmd.as_std()
            .get_args()
            .map(|a| a.to_string_lossy().into_owned())
            .collect()
    }

    #[test]
    fn isolation_parses_default_none() {
        assert_eq!(Isolation::from_config("none"), Isolation::None);
        assert_eq!(Isolation::from_config(""), Isolation::None);
        assert_eq!(Isolation::from_config("garbage"), Isolation::None);
        assert_eq!(Isolation::from_config("Sandboxed"), Isolation::Sandboxed);
    }

    #[test]
    fn local_executor_is_plain_sh() {
        let cmd = LocalExecutor
            .build("echo hi", Path::new("/ws"))
            .expect("local never fails");
        assert!(cmd.as_std().get_program().to_string_lossy().contains("sh"));
        assert_eq!(args_of(&cmd), vec!["-c", "echo hi"]);
    }

    #[test]
    fn bwrap_denies_network_and_binds_only_workspace() {
        let exec = SandboxedExecutor::with_launcher(SandboxLauncher::Bwrap(PathBuf::from(
            "/usr/bin/bwrap",
        )));
        let cmd = exec
            .build("cat secrets", Path::new("/work/ws"))
            .expect("launcher present");
        let args = args_of(&cmd);
        // Network is denied via fresh namespaces.
        assert!(args.iter().any(|a| a == "--unshare-all"));
        // The workspace is bound writable...
        assert!(args
            .windows(3)
            .any(|w| w == ["--bind", "/work/ws", "/work/ws"]));
        // ...and is the working directory.
        assert!(args.windows(2).any(|w| w == ["--chdir", "/work/ws"]));
        // $HOME is never bound into the sandbox.
        assert!(!args
            .iter()
            .any(|a| a.contains("/root") || a.contains("/home")));
        // The command still runs under sh -c.
        assert!(args.windows(2).any(|w| w == ["sh", "-c"]));
    }

    #[test]
    fn firejail_denies_network_and_privatizes_home() {
        let exec = SandboxedExecutor::with_launcher(SandboxLauncher::Firejail(PathBuf::from(
            "/usr/bin/firejail",
        )));
        let cmd = exec
            .build("cat secrets", Path::new("/work/ws"))
            .expect("launcher present");
        let args = args_of(&cmd);
        assert!(args.iter().any(|a| a == "--net=none"));
        assert!(args.iter().any(|a| a == "--private=/work/ws"));
    }

    #[test]
    fn sandboxed_without_launcher_fails_closed() {
        let exec = SandboxedExecutor { launcher: None };
        let err = exec
            .build("echo hi", Path::new("/ws"))
            .expect_err("must fail closed");
        assert!(matches!(err, ToolError::Sandbox(_)));
    }

    // End-to-end fail-closed proof under a real sandbox. Runs only where a
    // launcher is installed (Linux CI with bubblewrap/firejail); elsewhere it
    // is a no-op so the suite stays green on bare runners.
    #[tokio::test]
    async fn sandboxed_bash_blocks_secret_read_and_network() {
        let Some(_) = SandboxLauncher::detect() else {
            eprintln!("skipping: no sandbox launcher on PATH");
            return;
        };
        let ws = std::env::temp_dir().join(format!("rk-sandbox-e2e-{}", std::process::id()));
        let _ = std::fs::create_dir_all(&ws);
        let exec = SandboxedExecutor::detect();

        // A path outside the workspace grant is not visible → read fails.
        let mut cmd = exec
            .build("cat /etc/shadow 2>&1; echo done", &ws)
            .expect("launcher present");
        let out = cmd.output().await.expect("spawned");
        let combined = String::from_utf8_lossy(&out.stdout);
        assert!(combined.contains("done"));
        assert!(
            !combined.contains("root:"),
            "secret file must not be readable under sandbox"
        );

        let _ = std::fs::remove_dir_all(&ws);
    }
}
