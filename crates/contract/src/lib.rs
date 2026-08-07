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
    /// The conservative compile-time baseline: what is safe to assume on a
    /// host of this family *without asking it anything*.
    ///
    /// This is **not** detection, and is named so it cannot be mistaken for
    /// it. Deciding capabilities from `cfg!` alone produces answers a real
    /// host can contradict — conformance measured exactly that, creating and
    /// resolving a symlink on a `windows-latest` runner while this function
    /// reports `symlinks: false`. A capability model that CI cannot falsify
    /// will drift from reality silently.
    ///
    /// Real detection needs I/O, and this crate is deliberately I/O-free, so
    /// it lives in the adapter: use `compat::NativeCapabilities::detect()`
    /// whenever you can afford the probe. Reach for this only when you
    /// cannot, and treat a `false` as "not proven safe to assume," never as
    /// "impossible on this host."
    pub fn conservative_baseline() -> Self {
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
/// alone drops only the `MasterPty` handle it holds; `reader` and
/// `writer` are independently-owned clones of the master's read/write
/// ends (`portable-pty`'s `try_clone_reader`/`take_writer`) and are
/// **not** closed by dropping `control` by itself. Whether dropping
/// `control` alone is sufficient to hang up the child is host- and
/// `portable-pty`-handle-ownership-dependent and is **not verified** by
/// this spike's tests — do not depend on it. The only guaranteed way to
/// end a session is to drop `reader`, `writer`, and `control` together,
/// or let the child exit on its own and call `wait` to reap it. `wait`
/// blocks and has a single owner — there is no way for more than one
/// caller to await it. There is no `kill`/`terminate` method in this
/// spike (see CONTRACT.md).
pub trait PtyControl: Send {
    fn resize(&mut self, cols: u16, rows: u16) -> Result<()>;
    fn wait(&mut self) -> Result<i32>;
}

/// Opens interactive PTY sessions.
///
/// The primary operation is [`PtySession::spawn`], which runs an explicit
/// command. That is deliberate: when the only way to open a PTY was "run the
/// host's default shell," every observable property of a session — including
/// whether it ever exits — was a function of the user's rc files rather than
/// of this contract, and so could not be stated as a guarantee or tested as
/// one. Command selection is what makes PTY behavior a contract property.
///
/// Callers MUST check `pty_win32_input_mode` before relying on
/// Win32-input-mode escape sequences.
pub trait PtySession {
    /// Runs `command` under a new PTY of the given size. `ProcessSpec` is
    /// reused verbatim so that argv/cwd/env semantics — including
    /// `inherit_env` — are identical to `ProcessRunner::run`; a PTY should
    /// not be a second, subtly different way to describe a process.
    fn spawn(&self, command: &ProcessSpec, cols: u16, rows: u16) -> Result<PtySpawn>;

    /// The host's default interactive shell, as a spawnable command.
    ///
    /// Adapters MUST document how they choose it. It is exposed separately
    /// so callers can inspect or override the choice rather than having it
    /// baked into the spawn path.
    fn host_default_shell(&self) -> Result<ProcessSpec>;

    /// Convenience wrapper over [`PtySession::spawn`].
    ///
    /// Behavior of the resulting session depends on the user's shell and
    /// their rc files, which this contract does not govern: a customized
    /// login chain can hand off to another shell that never exits on `exit`.
    /// Nothing about this method is a guarantee beyond "a PTY was opened."
    /// Use [`PtySession::spawn`] for anything that must be deterministic.
    fn spawn_shell(&self, cols: u16, rows: u16) -> Result<PtySpawn> {
        let command = self.host_default_shell()?;
        self.spawn(&command, cols, rows)
    }
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
        let caps = Capabilities::conservative_baseline();
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
