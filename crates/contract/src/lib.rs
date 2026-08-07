//! Portable Process & Filesystem Contract (Phase 0).
//!
//! This crate is the trait boundary only — no OS-specific behavior, no
//! third-party OS adapters. Implementations live in the `compat` crate.
//! See `/CONTRACT.md` at the repo root for the guarantees each trait makes.

use std::collections::HashMap;
use std::io::{Read, Write};
use std::path::{Path, PathBuf};
use std::time::SystemTime;

/// Stable error surface. Adapters MUST map OS errno/HRESULT into one of
/// the specific variants below where possible. `Io` is the explicit
/// fallback category for OS errors with no better match — its `source`
/// is retained for diagnostics (logging, `Display`) only; callers MUST
/// match on the variant, never on message text, to stay portable.
#[derive(Debug, thiserror::Error)]
pub enum ContractError {
    #[error("path escapes scoped root: {0}")]
    PathEscape(String),
    #[error("not found: {0}")]
    NotFound(String),
    #[error("permission denied: {0}")]
    PermissionDenied(String),
    #[error("unsupported on this host: {0}")]
    Unsupported(String),
    #[error("io error: {source}")]
    Io {
        #[source]
        source: std::io::Error,
    },
}

impl From<std::io::Error> for ContractError {
    /// Categorizes by `ErrorKind` into a stable variant; only errors with
    /// no better category fall through to `Io`.
    fn from(err: std::io::Error) -> Self {
        match err.kind() {
            std::io::ErrorKind::NotFound => ContractError::NotFound(err.to_string()),
            std::io::ErrorKind::PermissionDenied => {
                ContractError::PermissionDenied(err.to_string())
            }
            std::io::ErrorKind::Unsupported => ContractError::Unsupported(err.to_string()),
            _ => ContractError::Io { source: err },
        }
    }
}

pub type Result<T, E = ContractError> = std::result::Result<T, E>;

/// Per-host capability flags. Tools MUST check the relevant flag before
/// depending on non-baseline behavior instead of branching on `cfg!(windows)`
/// themselves — that keeps the divergence list in one place (CONTRACT.md).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Capabilities {
    /// Conservative baseline, not a hard platform fact: `true` on Unix,
    /// `false` on Windows. Windows *can* create symlinks under Developer
    /// Mode or elevated privilege, but this crate does not yet probe for
    /// that — treat `false` here as "not proven safe to assume," not as
    /// "impossible on this host."
    pub symlinks: bool,
    pub unix_permissions: bool,
    /// Tracks the known `portable-pty` gap: ConPTY's
    /// `PSEUDOCONSOLE_WIN32_INPUT_MODE` / `PASSTHROUGH_MODE` are not passed
    /// through on the stock crate as of this writing.
    pub pty_win32_input_mode: bool,
    pub advisory_locking: bool,
}

impl Capabilities {
    /// Static per-OS matrix. Deliberately data-only — no I/O, no external
    /// crates — so it belongs in the contract, not an adapter.
    pub fn detect() -> Self {
        Capabilities {
            symlinks: cfg!(unix),
            unix_permissions: cfg!(unix),
            pty_win32_input_mode: false,
            advisory_locking: true,
        }
    }
}

/// Metadata for a single filesystem entry, normalized across hosts.
#[derive(Debug, Clone)]
pub struct Metadata {
    pub len: u64,
    pub is_dir: bool,
    pub is_symlink: bool,
    pub readonly: bool,
    pub modified: Option<SystemTime>,
}

/// A directory entry returned by `FsRoot::read_dir`.
#[derive(Debug, Clone)]
pub struct DirEntryInfo {
    pub name: String,
    pub metadata: Metadata,
}

/// Filesystem operations scoped to a single root directory. No path passed
/// to these methods may escape the root — implementations MUST return
/// `ContractError::PathEscape` rather than silently resolving `..`.
pub trait FsRoot {
    fn stat(&self, path: &Path) -> Result<Metadata>;
    fn read_dir(&self, path: &Path) -> Result<Vec<DirEntryInfo>>;
    fn read_to_string(&self, path: &Path) -> Result<String>;
    fn write(&self, path: &Path, contents: &[u8]) -> Result<()>;
    fn create_dir(&self, path: &Path) -> Result<()>;
    fn remove_file(&self, path: &Path) -> Result<()>;
}

/// A process to spawn. `inherit_env` selects between "start from the
/// current process environment" and "start from an empty environment plus
/// `env`" — the contract has no implicit environment merging behavior.
#[derive(Debug, Clone)]
pub struct ProcessSpec {
    pub program: String,
    pub args: Vec<String>,
    pub cwd: Option<PathBuf>,
    pub env: HashMap<String, String>,
    pub inherit_env: bool,
}

impl ProcessSpec {
    pub fn new(program: impl Into<String>) -> Self {
        ProcessSpec {
            program: program.into(),
            args: Vec::new(),
            cwd: None,
            env: HashMap::new(),
            inherit_env: true,
        }
    }

    pub fn arg(mut self, arg: impl Into<String>) -> Self {
        self.args.push(arg.into());
        self
    }
}

#[derive(Debug, Clone)]
pub struct ProcessOutput {
    pub status: i32,
    pub stdout: Vec<u8>,
    pub stderr: Vec<u8>,
}

/// Non-interactive process execution: spawn, capture stdout/stderr, wait.
pub trait ProcessRunner {
    fn run(&self, spec: &ProcessSpec) -> Result<ProcessOutput>;
}

/// A live interactive PTY session. `Read`/`Write` carry the terminal
/// byte stream; `resize` and `wait` are the only additional primitives the
/// contract promises. Reader and writer are independent streams (as real
/// PTY masters expose) so a caller can pump output on one thread while
/// writing input on another without sharing a lock across a blocking read.
pub struct PtySpawn {
    pub reader: Box<dyn Read + Send>,
    pub writer: Box<dyn Write + Send>,
    pub control: Box<dyn PtyControl>,
}

/// Out-of-band control for a live PTY session: resize and wait-for-exit.
///
/// Lifecycle, since ownership is split across `PtySpawn`'s three fields:
/// `reader`, `writer`, and `control` are each independently droppable —
/// dropping one does not drop or close the others. Dropping `control`
/// closes the pty master, which hangs up the child (SIGHUP-equivalent on
/// Unix, the ConPTY-equivalent on Windows) but does **not** explicitly
/// kill it and does **not** reap it. `wait` is the only way to reap the
/// child and obtain its exit status; it blocks and has a single owner —
/// there is no way for more than one caller to await it. There is no
/// `kill`/`terminate` method in this spike (see CONTRACT.md).
pub trait PtyControl: Send {
    fn resize(&mut self, cols: u16, rows: u16) -> Result<()>;
    fn wait(&mut self) -> Result<i32>;
}

/// Opens interactive PTY sessions running the host's default shell.
/// Callers MUST check `Capabilities::detect().pty_win32_input_mode` before
/// relying on Win32-input-mode escape sequences.
pub trait PtySession {
    fn spawn_shell(&self, cols: u16, rows: u16) -> Result<PtySpawn>;
}

/// A held advisory lock. Dropping without calling `unlock` MUST still
/// release the lock (adapters implement `Drop`), `unlock` exists only to
/// surface release errors explicitly.
pub trait LockGuard {
    fn unlock(self: Box<Self>) -> Result<()>;
}

/// Advisory, best-effort file locking. Never mandatory — two processes
/// that ignore the lock can still race. See CONTRACT.md.
pub trait FileLock {
    fn lock_exclusive(&self, path: &Path) -> Result<Box<dyn LockGuard>>;
    fn lock_shared(&self, path: &Path) -> Result<Box<dyn LockGuard>>;
}

/// Deterministic per-OS config/cache/data directories for a named app.
pub trait StandardDirs {
    fn config_dir(&self, app: &str) -> Result<PathBuf>;
    fn cache_dir(&self, app: &str) -> Result<PathBuf>;
    fn data_dir(&self, app: &str) -> Result<PathBuf>;
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn capabilities_are_internally_consistent() {
        let caps = Capabilities::detect();
        // Win32 input mode is a Windows-only gap; it can never be true
        // while symlinks (a Unix-only baseline) is also true.
        assert!(!(caps.pty_win32_input_mode && caps.symlinks));
    }

    #[test]
    fn process_spec_builder_appends_args() {
        let spec = ProcessSpec::new("echo").arg("a").arg("b");
        assert_eq!(spec.args, vec!["a".to_string(), "b".to_string()]);
        assert!(spec.inherit_env);
    }

    #[test]
    fn io_errors_categorize_into_stable_variants() {
        let not_found = std::io::Error::from(std::io::ErrorKind::NotFound);
        assert!(matches!(
            ContractError::from(not_found),
            ContractError::NotFound(_)
        ));

        let denied = std::io::Error::from(std::io::ErrorKind::PermissionDenied);
        assert!(matches!(
            ContractError::from(denied),
            ContractError::PermissionDenied(_)
        ));

        let unsupported = std::io::Error::from(std::io::ErrorKind::Unsupported);
        assert!(matches!(
            ContractError::from(unsupported),
            ContractError::Unsupported(_)
        ));

        // No specific category: falls through to `Io`, source retained.
        let other = std::io::Error::from(std::io::ErrorKind::Other);
        let mapped = ContractError::from(other);
        assert!(matches!(mapped, ContractError::Io { .. }));
        assert!(std::error::Error::source(&mapped).is_some());
    }
}
