//! # coreutils — the reference consumer
//!
//! Utilities written *only* against the `platform` trait surface: they
//! compile with zero knowledge of which backend runs them, and their unit
//! tests run against `platform-mock` with no OS interaction. This crate is
//! the consumer that gates the fs/process domains (RFC v2 §3) and the
//! exercise ground for the understanding mandate (M1).
//!
//! Explicitly not a uutils competitor; rush bundles uutils for daily-driver
//! use (rush ADR-005).

#![forbid(unsafe_code)]

pub mod cat;
pub mod ls;
pub mod term_report;

/// Ambient entry point to this OS's native backend — the one place the
/// CLI binaries touch a concrete backend type; everything else in this
/// crate is written against the `platform` traits alone.
#[cfg(any(target_os = "linux", windows))]
pub mod native {
    use std::path::Path;

    use platform::error::{ErrorKind, OsCode, PlatformError, Result};
    use platform::fs::{Dir, File, OpenOptions};

    #[cfg(target_os = "linux")]
    pub fn open_dir(path: &Path) -> Result<Box<dyn Dir>> {
        Ok(Box::new(platform_linux::LinuxDir::open_ambient(path)?))
    }

    #[cfg(windows)]
    pub fn open_dir(path: &Path) -> Result<Box<dyn Dir>> {
        Ok(Box::new(platform_windows::WindowsDir::open_ambient(path)?))
    }

    /// Opens `path` — an arbitrary ambient *file* path, not necessarily
    /// under a directory this process already has a capability for —
    /// with `opts`. Splits into parent directory + file name and goes
    /// through [`open_dir`] + [`Dir::open`], the only portable way to
    /// reach an arbitrary path through the `platform::fs` capability
    /// model (rustils#62): every binary here that takes a bare CLI path
    /// argument for a file should use this rather than re-deriving the
    /// split itself, or — as `rtee` originally did — reaching past
    /// `platform::fs` for `std::fs::File` directly.
    pub fn open_ambient_file(path: &Path, opts: &OpenOptions) -> Result<Box<dyn File>> {
        let parent = path
            .parent()
            .filter(|d| !d.as_os_str().is_empty())
            .unwrap_or_else(|| Path::new("."));
        let name = path.file_name().ok_or_else(|| {
            PlatformError::new(
                ErrorKind::InvalidInput,
                OsCode::None,
                "open_ambient_file: not a file path",
            )
            .with_path(path)
        })?;
        open_dir(parent)?.open(name, opts)
    }

    #[cfg(target_os = "linux")]
    pub fn spawner() -> Box<dyn platform::process::Spawner> {
        Box::new(platform_linux::LinuxSpawner)
    }

    #[cfg(windows)]
    pub fn spawner() -> Box<dyn platform::process::Spawner> {
        Box::new(platform_windows::WindowsSpawner)
    }

    #[cfg(target_os = "linux")]
    pub fn signal_source() -> Box<dyn platform::events::SignalSource> {
        Box::new(platform_linux::LinuxSignalSource)
    }

    #[cfg(windows)]
    pub fn signal_source() -> Box<dyn platform::events::SignalSource> {
        Box::new(platform_windows::WindowsSignalSource)
    }

    #[cfg(target_os = "linux")]
    pub fn terminal() -> Box<dyn platform::term::Terminal> {
        Box::new(platform_linux::LinuxTerminal::new())
    }

    #[cfg(windows)]
    pub fn terminal() -> Box<dyn platform::term::Terminal> {
        Box::new(platform_windows::WindowsTerminal::new())
    }

    /// `uid` → account name, numeric-string fallback when there's no
    /// such account (or, on Windows, no `uid` concept at all —
    /// `Dir::unix_mode` is always `None` there, so this never actually
    /// gets called with a real value on that backend, but still needs
    /// to exist for `rls -l`'s call site to compile on every target).
    /// `ls -l`'s donor material, coreutils gap backlog #65.
    #[cfg(target_os = "linux")]
    pub fn user_name(uid: u32) -> String {
        platform_linux::user_name(uid).unwrap_or_else(|| uid.to_string())
    }

    #[cfg(windows)]
    pub fn user_name(uid: u32) -> String {
        uid.to_string()
    }

    /// `gid` → group name, on the same terms as [`user_name`].
    #[cfg(target_os = "linux")]
    pub fn group_name(gid: u32) -> String {
        platform_linux::group_name(gid).unwrap_or_else(|| gid.to_string())
    }

    #[cfg(windows)]
    pub fn group_name(gid: u32) -> String {
        gid.to_string()
    }
}
