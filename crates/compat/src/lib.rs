//! Adapters wiring the `contract` trait boundary to existing, mature crates:
//! cap-std (fs), portable-pty (process/pty), dirs (standard dirs), and
//! std's own file-locking methods (stable since 1.89). This crate is
//! deliberately thin — the primitives it wraps are already cross-platform;
//! the only genuinely OS-specific logic lives in `Capabilities::detect()`
//! (in `contract`) and the PTY shell selection below.

use std::path::{Path, PathBuf};
use std::process::Command;

use contract::{
    ContractError, DirEntryInfo, FileLock, FsRoot, LockGuard, Metadata, ProcessOutput,
    ProcessRunner, ProcessSpec, PtyControl, PtySession, PtySpawn, Result, StandardDirs,
};

fn to_metadata(m: cap_std::fs::Metadata) -> Metadata {
    Metadata {
        len: m.len(),
        is_dir: m.is_dir(),
        is_symlink: m.is_symlink(),
        readonly: m.permissions().readonly(),
        modified: m.modified().ok().map(cap_std::time::SystemTime::into_std),
    }
}

/// A filesystem root scoped by `cap-std`. No path passed through its
/// methods can escape the directory it was opened on.
pub struct Workspace {
    dir: cap_std::fs::Dir,
}

impl Workspace {
    pub fn open_ambient(root: &Path) -> Result<Self> {
        let dir = cap_std::fs::Dir::open_ambient_dir(root, cap_std::ambient_authority())?;
        Ok(Workspace { dir })
    }
}

impl FsRoot for Workspace {
    fn stat(&self, path: &Path) -> Result<Metadata> {
        Ok(to_metadata(self.dir.metadata(path)?))
    }

    fn read_dir(&self, path: &Path) -> Result<Vec<DirEntryInfo>> {
        let mut out = Vec::new();
        for entry in self.dir.read_dir(path)? {
            let entry = entry?;
            out.push(DirEntryInfo {
                name: entry.file_name().to_string_lossy().into_owned(),
                metadata: to_metadata(entry.metadata()?),
            });
        }
        Ok(out)
    }

    fn read_to_string(&self, path: &Path) -> Result<String> {
        Ok(self.dir.read_to_string(path)?)
    }

    fn write(&self, path: &Path, contents: &[u8]) -> Result<()> {
        Ok(self.dir.write(path, contents)?)
    }

    fn create_dir(&self, path: &Path) -> Result<()> {
        Ok(self.dir.create_dir(path)?)
    }

    fn remove_file(&self, path: &Path) -> Result<()> {
        Ok(self.dir.remove_file(path)?)
    }
}

struct StdFileLockGuard(std::fs::File);

impl LockGuard for StdFileLockGuard {
    fn unlock(self: Box<Self>) -> Result<()> {
        self.0.unlock()?;
        Ok(())
    }
}

impl FileLock for Workspace {
    fn lock_exclusive(&self, path: &Path) -> Result<Box<dyn LockGuard>> {
        let file = self.dir.open_with(
            path,
            cap_std::fs::OpenOptions::new()
                .read(true)
                .write(true)
                .create(true),
        )?;
        let file = file.into_std();
        file.lock()?;
        Ok(Box::new(StdFileLockGuard(file)))
    }

    fn lock_shared(&self, path: &Path) -> Result<Box<dyn LockGuard>> {
        let file = self.dir.open_with(
            path,
            cap_std::fs::OpenOptions::new()
                .read(true)
                .write(true)
                .create(true),
        )?;
        let file = file.into_std();
        file.lock_shared()?;
        Ok(Box::new(StdFileLockGuard(file)))
    }
}

/// Non-interactive process execution via `std::process`, already portable.
pub struct NativeProcessRunner;

impl ProcessRunner for NativeProcessRunner {
    fn run(&self, spec: &ProcessSpec) -> Result<ProcessOutput> {
        let mut cmd = Command::new(&spec.program);
        cmd.args(&spec.args);
        if let Some(cwd) = &spec.cwd {
            cmd.current_dir(cwd);
        }
        if !spec.inherit_env {
            cmd.env_clear();
        }
        cmd.envs(&spec.env);
        let output = cmd.output()?;
        Ok(ProcessOutput {
            status: output.status.code().unwrap_or(-1),
            stdout: output.stdout,
            stderr: output.stderr,
        })
    }
}

/// Interactive PTY sessions via `portable-pty`. Known gap: `pixel_width`/
/// `pixel_height` are left at 0, and the stock crate does not pass through
/// ConPTY's `PSEUDOCONSOLE_WIN32_INPUT_MODE` — see
/// `Capabilities::pty_win32_input_mode` in the `contract` crate.
pub struct NativePtySession;

fn default_shell() -> String {
    if cfg!(windows) {
        std::env::var("COMSPEC").unwrap_or_else(|_| "cmd.exe".to_string())
    } else {
        std::env::var("SHELL").unwrap_or_else(|_| "/bin/sh".to_string())
    }
}

struct NativePtyControl {
    master: Box<dyn portable_pty::MasterPty + Send>,
    child: Box<dyn portable_pty::Child + Send + Sync>,
}

impl PtyControl for NativePtyControl {
    fn resize(&mut self, cols: u16, rows: u16) -> Result<()> {
        self.master
            .resize(portable_pty::PtySize {
                rows,
                cols,
                pixel_width: 0,
                pixel_height: 0,
            })
            .map_err(|e| ContractError::Unsupported(e.to_string()))
    }

    fn wait(&mut self) -> Result<i32> {
        let status = self.child.wait()?;
        Ok(status.exit_code() as i32)
    }
}

impl PtySession for NativePtySession {
    fn spawn_shell(&self, cols: u16, rows: u16) -> Result<PtySpawn> {
        let pty_system = portable_pty::native_pty_system();
        let pair = pty_system
            .openpty(portable_pty::PtySize {
                rows,
                cols,
                pixel_width: 0,
                pixel_height: 0,
            })
            .map_err(|e| ContractError::Unsupported(e.to_string()))?;
        let cmd = portable_pty::CommandBuilder::new(default_shell());
        let child = pair
            .slave
            .spawn_command(cmd)
            .map_err(|e| ContractError::Unsupported(e.to_string()))?;
        let reader = pair
            .master
            .try_clone_reader()
            .map_err(|e| ContractError::Unsupported(e.to_string()))?;
        let writer = pair
            .master
            .take_writer()
            .map_err(|e| ContractError::Unsupported(e.to_string()))?;
        Ok(PtySpawn {
            reader,
            writer,
            control: Box::new(NativePtyControl {
                master: pair.master,
                child,
            }),
        })
    }
}

/// Deterministic per-OS config/cache/data directories via the `dirs` crate.
pub struct NativeStandardDirs;

impl StandardDirs for NativeStandardDirs {
    fn config_dir(&self, app: &str) -> Result<PathBuf> {
        dirs::config_dir()
            .map(|p| p.join(app))
            .ok_or_else(|| ContractError::Unsupported("no config dir on this host".into()))
    }

    fn cache_dir(&self, app: &str) -> Result<PathBuf> {
        dirs::cache_dir()
            .map(|p| p.join(app))
            .ok_or_else(|| ContractError::Unsupported("no cache dir on this host".into()))
    }

    fn data_dir(&self, app: &str) -> Result<PathBuf> {
        dirs::data_dir()
            .map(|p| p.join(app))
            .ok_or_else(|| ContractError::Unsupported("no data dir on this host".into()))
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn workspace_scoped_fs_roundtrip() {
        let tmp = std::env::temp_dir().join(format!("compat-test-{}", std::process::id()));
        std::fs::create_dir_all(&tmp).unwrap();
        let ws = Workspace::open_ambient(&tmp).unwrap();

        ws.write(Path::new("hello.txt"), b"hi").unwrap();
        assert_eq!(ws.read_to_string(Path::new("hello.txt")).unwrap(), "hi");

        let meta = ws.stat(Path::new("hello.txt")).unwrap();
        assert_eq!(meta.len, 2);
        assert!(!meta.is_dir);

        let entries = ws.read_dir(Path::new(".")).unwrap();
        assert!(entries.iter().any(|e| e.name == "hello.txt"));

        ws.remove_file(Path::new("hello.txt")).unwrap();
        std::fs::remove_dir_all(&tmp).ok();
    }

    #[test]
    fn workspace_exclusive_lock_blocks_a_second_holder() {
        let tmp = std::env::temp_dir().join(format!("compat-lock-{}", std::process::id()));
        std::fs::create_dir_all(&tmp).unwrap();
        let ws = Workspace::open_ambient(&tmp).unwrap();

        let guard = ws.lock_exclusive(Path::new("lockfile")).unwrap();
        // A second, independently-opened handle must not acquire the same
        // exclusive lock while `guard` is held.
        let second = std::fs::OpenOptions::new()
            .read(true)
            .write(true)
            .open(tmp.join("lockfile"))
            .unwrap();
        assert!(second.try_lock().is_err());

        guard.unlock().unwrap();
        std::fs::remove_dir_all(&tmp).ok();
    }

    #[test]
    fn process_runner_captures_stdout() {
        let runner = NativeProcessRunner;
        let spec = if cfg!(windows) {
            ProcessSpec::new("cmd").arg("/C").arg("echo hello")
        } else {
            ProcessSpec::new("echo").arg("hello")
        };
        let output = runner.run(&spec).unwrap();
        assert_eq!(output.status, 0);
        assert!(String::from_utf8_lossy(&output.stdout).contains("hello"));
    }

    #[test]
    fn standard_dirs_are_non_empty() {
        let dirs = NativeStandardDirs;
        let cfg = dirs.config_dir("compat-test");
        // Every CI host (Windows/Linux/macOS) resolves a config dir; the
        // only intentionally-unsupported case is a minimal/headless host
        // with no home directory, which none of our CI runners are.
        assert!(cfg.is_ok(), "expected a config dir on this host: {cfg:?}");
    }
}
