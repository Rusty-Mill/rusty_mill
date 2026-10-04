//! The sandboxed-execution port (ADR-0005 §4, invariant 5).
//!
//! Every untrusted process (a task solution now, the inner harness and its
//! build later) runs through an [`Executor`] under a [`SandboxSpec`]: an
//! explicit filesystem allowlist, no internet sockets, and hard
//! [`Limits`]. The types here only describe that contract; the Linux
//! adapter that enforces it lives in `rsi-runtime`.

use core::time::Duration;
use std::path::PathBuf;

use crate::error::CoreError;

/// Hard resource limits for one sandboxed process tree.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Limits {
    cpu: Duration,
    wall: Duration,
    memory_bytes: u64,
    file_bytes: u64,
    open_files: u64,
    processes: u64,
}

impl Limits {
    /// Validates a set of limits.
    ///
    /// `cpu` and `wall` are rounded by the adapter to whole seconds and
    /// milliseconds respectively.
    ///
    /// # Errors
    /// [`CoreError::InvalidParameter`] if any limit is zero, or `cpu` is
    /// under one second (the kernel's CPU limit has one-second granularity).
    pub fn new(
        cpu: Duration,
        wall: Duration,
        memory_bytes: u64,
        file_bytes: u64,
        open_files: u64,
        processes: u64,
    ) -> Result<Self, CoreError> {
        let invalid = |name: &'static str, value: f64| CoreError::InvalidParameter { name, value };
        if cpu < Duration::from_secs(1) {
            return Err(invalid("cpu limit (seconds)", cpu.as_secs_f64()));
        }
        if wall.is_zero() {
            return Err(invalid("wall limit (seconds)", 0.0));
        }
        for (name, value) in [
            ("memory limit", memory_bytes),
            ("file size limit", file_bytes),
            ("open file limit", open_files),
            ("process limit", processes),
        ] {
            if value == 0 {
                return Err(invalid(name, 0.0));
            }
        }
        Ok(Self {
            cpu,
            wall,
            memory_bytes,
            file_bytes,
            open_files,
            processes,
        })
    }

    /// CPU time before the kernel kills the process.
    #[must_use]
    pub const fn cpu(&self) -> Duration {
        self.cpu
    }

    /// Wall-clock time before the executor kills the process group.
    #[must_use]
    pub const fn wall(&self) -> Duration {
        self.wall
    }

    /// Address-space limit in bytes.
    #[must_use]
    pub const fn memory_bytes(&self) -> u64 {
        self.memory_bytes
    }

    /// Largest file the process may write, in bytes.
    #[must_use]
    pub const fn file_bytes(&self) -> u64 {
        self.file_bytes
    }

    /// Open file descriptor limit.
    #[must_use]
    pub const fn open_files(&self) -> u64 {
        self.open_files
    }

    /// Process limit (per user on Linux, so a brake rather than a quota).
    #[must_use]
    pub const fn processes(&self) -> u64 {
        self.processes
    }

    /// These limits with the wall clock cut to at most `cap`, for a run that
    /// must end by a deadline. CPU time is cut to match, but never below the
    /// one-second minimum; the wall clock is what bounds the run.
    ///
    /// # Errors
    /// [`CoreError::InvalidParameter`] if `cap` is zero.
    pub fn capped(&self, cap: Duration) -> Result<Self, CoreError> {
        Self::new(
            self.cpu.min(cap.max(Duration::from_secs(1))),
            self.wall.min(cap),
            self.memory_bytes,
            self.file_bytes,
            self.open_files,
            self.processes,
        )
    }
}

/// Where a sandboxed process may read and write, and how it runs.
///
/// Everything not under `read_roots` or `write_roots` is unreadable;
/// internet sockets are always refused. Paths must be absolute.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct SandboxSpec {
    read_roots: Vec<PathBuf>,
    write_roots: Vec<PathBuf>,
    cwd: PathBuf,
    env: Vec<(String, String)>,
    limits: Limits,
}

impl SandboxSpec {
    /// Validates a sandbox specification.
    ///
    /// # Errors
    /// [`CoreError::InvalidId`] if a path is relative, `cwd` is not under a
    /// write root, or an environment variable name is empty or contains `=`.
    pub fn new(
        read_roots: Vec<PathBuf>,
        write_roots: Vec<PathBuf>,
        cwd: PathBuf,
        env: Vec<(String, String)>,
        limits: Limits,
    ) -> Result<Self, CoreError> {
        let all = read_roots.iter().chain(&write_roots).chain([&cwd]);
        if let Some(relative) = all.into_iter().find(|path| !path.is_absolute()) {
            return Err(invalid_path(relative));
        }
        if !write_roots.iter().any(|root| cwd.starts_with(root)) {
            return Err(invalid_path(&cwd));
        }
        if let Some((name, _)) = env
            .iter()
            .find(|(name, _)| name.is_empty() || name.contains('='))
        {
            return Err(CoreError::InvalidId {
                kind: "environment variable name",
                value: name.clone(),
            });
        }
        Ok(Self {
            read_roots,
            write_roots,
            cwd,
            env,
            limits,
        })
    }

    /// Directories the process may read and execute from.
    #[must_use]
    pub fn read_roots(&self) -> &[PathBuf] {
        &self.read_roots
    }

    /// Directories the process may read, write and create files in.
    #[must_use]
    pub fn write_roots(&self) -> &[PathBuf] {
        &self.write_roots
    }

    /// The working directory, always under a write root.
    #[must_use]
    pub fn cwd(&self) -> &PathBuf {
        &self.cwd
    }

    /// The complete environment; nothing is inherited.
    #[must_use]
    pub fn env(&self) -> &[(String, String)] {
        &self.env
    }

    /// The resource limits.
    #[must_use]
    pub const fn limits(&self) -> &Limits {
        &self.limits
    }

    /// Whether `path` lies under any root the process can read or write.
    ///
    /// Callers use this to prove private data stays out of reach before
    /// running anything. Both sides must already be canonical paths.
    #[must_use]
    pub fn can_reach(&self, path: &std::path::Path) -> bool {
        self.read_roots
            .iter()
            .chain(&self.write_roots)
            .any(|root| path.starts_with(root) || root.starts_with(path))
    }
}

fn invalid_path(path: &std::path::Path) -> CoreError {
    CoreError::InvalidId {
        kind: "sandbox path",
        value: path.display().to_string(),
    }
}

/// How a sandboxed process ended.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Termination {
    /// Exited with this status code.
    Exited(i32),
    /// Killed by this signal (for example `SIGXCPU` at the CPU limit).
    Signaled(i32),
    /// Killed by the executor at the wall-clock limit.
    TimedOut,
}

impl Termination {
    /// Whether the process exited cleanly with status 0.
    #[must_use]
    pub const fn succeeded(&self) -> bool {
        matches!(self, Self::Exited(0))
    }
}

/// The result of one sandboxed run. Output is truncated by the adapter.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ExecOutcome {
    /// How the process ended.
    pub termination: Termination,
    /// The start of standard output.
    pub stdout: Vec<u8>,
    /// The start of standard error.
    pub stderr: Vec<u8>,
    /// Wall-clock time from spawn to exit.
    pub wall: Duration,
}

/// Runs one program under a [`SandboxSpec`].
///
/// An implementation must fail closed: when it cannot enforce every part
/// of the spec it returns an error and runs nothing.
pub trait Executor {
    /// The adapter's error type.
    type Error;

    /// Runs `program` with `args` and waits for it to end.
    ///
    /// # Errors
    /// When the sandbox cannot be set up or the process cannot be
    /// started. A program that fails, crashes or times out is an
    /// [`ExecOutcome`], not an error.
    fn exec(
        &self,
        spec: &SandboxSpec,
        program: &str,
        args: &[String],
    ) -> Result<ExecOutcome, Self::Error>;
}

#[cfg(test)]
mod tests {
    use super::*;

    fn limits() -> Limits {
        Limits::new(
            Duration::from_secs(2),
            Duration::from_secs(5),
            1 << 29,
            1 << 20,
            64,
            32,
        )
        .expect("valid limits")
    }

    /// A fixture path: a leading `/` maps onto an absolute root that is
    /// valid on every platform (`/usr` is not absolute on Windows).
    fn p(fixture: &str) -> PathBuf {
        match fixture.strip_prefix('/') {
            Some(rest) => std::env::temp_dir().join("rsi-spec").join(rest),
            None => PathBuf::from(fixture),
        }
    }

    fn spec(read: &[&str], write: &[&str], cwd: &str) -> Result<SandboxSpec, CoreError> {
        let paths = |items: &[&str]| items.iter().map(|item| p(item)).collect();
        SandboxSpec::new(paths(read), paths(write), p(cwd), Vec::new(), limits())
    }

    #[test]
    fn limits_reject_zero_and_sub_second_cpu() {
        let s = Duration::from_secs(1);
        assert!(Limits::new(Duration::from_millis(999), s, 1, 1, 1, 1).is_err());
        assert!(Limits::new(s, Duration::ZERO, 1, 1, 1, 1).is_err());
        for i in 0..4 {
            let mut v = [1u64; 4];
            v[i] = 0;
            assert!(Limits::new(s, s, v[0], v[1], v[2], v[3]).is_err(), "{i}");
        }
        assert_eq!(limits().memory_bytes(), 1 << 29);
    }

    #[test]
    fn capped_limits_end_by_the_deadline() {
        let secs = Duration::from_secs;
        let base = Limits::new(secs(10), secs(20), 1, 1, 1, 1).expect("valid");
        let short = base.capped(secs(3)).expect("valid");
        assert_eq!((short.cpu(), short.wall()), (secs(3), secs(3)));
        let tiny = base.capped(Duration::from_millis(200)).expect("valid");
        assert_eq!(tiny.wall(), Duration::from_millis(200));
        assert_eq!(tiny.cpu(), secs(1), "CPU never drops below one second");
        assert_eq!(base.capped(secs(60)).expect("valid"), base, "no cap needed");
        assert!(base.capped(Duration::ZERO).is_err());
    }

    #[test]
    fn spec_requires_absolute_paths_and_writable_cwd() {
        assert!(spec(&["/usr"], &["/w"], "/w/run").is_ok());
        assert!(spec(&["usr"], &["/w"], "/w").is_err());
        assert!(spec(&["/usr"], &["w"], "w").is_err());
        assert!(spec(&["/usr"], &["/w"], "/elsewhere").is_err());
        assert!(
            spec(&["/usr"], &[], "/usr").is_err(),
            "cwd must be writable"
        );
    }

    #[test]
    fn spec_rejects_bad_env_names() {
        for name in ["", "A=B"] {
            let env = vec![(name.to_owned(), "x".to_owned())];
            let result = SandboxSpec::new(vec![], vec![p("/w")], p("/w"), env, limits());
            assert!(
                matches!(
                    result,
                    Err(CoreError::InvalidId {
                        kind: "environment variable name",
                        ..
                    })
                ),
                "{name:?}: {result:?}"
            );
        }
    }

    #[test]
    fn reachability_covers_both_directions() {
        let s = spec(&["/usr"], &["/tmp/run"], "/tmp/run").expect("valid");
        assert!(s.can_reach(&p("/usr/lib/x")));
        assert!(s.can_reach(&p("/tmp/run/out")));
        assert!(s.can_reach(&p("/tmp")), "a root inside it");
        assert!(!s.can_reach(&p("/srv/private/labels.txt")));
        assert!(!s.can_reach(&p("/usrx")), "component-wise");
    }

    #[test]
    fn only_exit_zero_succeeds() {
        assert!(Termination::Exited(0).succeeded());
        assert!(!Termination::Exited(1).succeeded());
        assert!(!Termination::Signaled(9).succeeded());
        assert!(!Termination::TimedOut.succeeded());
    }
}
