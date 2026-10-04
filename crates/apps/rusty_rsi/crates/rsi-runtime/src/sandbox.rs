//! The sandbox helper: the process that confines itself and then becomes
//! the untrusted program (ADR-0005 §4).
//!
//! Landlock and seccomp apply to the calling thread and its descendants,
//! and installing them after `fork` in a multithreaded process is unsafe.
//! So the executor never confines itself: it spawns the `rsi` binary with
//! the `__sandbox` subcommand (single-threaded from birth), which runs
//! [`run_helper`]: set rlimits, restrict the filesystem to an allowlist,
//! block internet sockets, verify each step was enforced, then `exec`.
//!
//! Setup failures are reported through a status file the parent created
//! and the helper opens *before* confining itself. The descriptor is
//! close-on-exec, so after a successful `exec` the file stays empty and
//! the untrusted program can neither reach nor forge it.

use std::ffi::{OsStr, OsString};
use std::path::PathBuf;

use crate::error::RuntimeError;

/// Exit status of a helper that could not set up the sandbox.
pub const SETUP_FAILED: u8 = 125;

/// Everything the helper needs, as decoded from its command line.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct HelperRequest {
    /// File the helper writes a setup error into.
    pub status: PathBuf,
    /// Working directory of the program.
    pub cwd: PathBuf,
    /// CPU seconds before `SIGXCPU`.
    pub cpu_secs: u64,
    /// Address-space limit in bytes.
    pub memory_bytes: u64,
    /// Largest writable file in bytes.
    pub file_bytes: u64,
    /// Open descriptor limit.
    pub open_files: u64,
    /// Process limit.
    pub processes: u64,
    /// Readable and executable directories.
    pub read: Vec<PathBuf>,
    /// Writable directories.
    pub write: Vec<PathBuf>,
    /// The program's complete environment.
    pub env: Vec<(String, String)>,
    /// The program to execute (looked up on the environment's `PATH`).
    pub program: OsString,
    /// Its arguments.
    pub args: Vec<OsString>,
}

impl HelperRequest {
    /// Encodes the request as helper arguments (after the subcommand).
    #[must_use]
    pub fn encode(&self) -> Vec<OsString> {
        let mut out: Vec<OsString> = Vec::new();
        let mut flag = |name: &str, value: OsString| {
            out.push(OsString::from(name));
            out.push(value);
        };
        flag("--status", self.status.clone().into_os_string());
        flag("--cwd", self.cwd.clone().into_os_string());
        flag("--cpu", self.cpu_secs.to_string().into());
        flag("--mem", self.memory_bytes.to_string().into());
        flag("--fsize", self.file_bytes.to_string().into());
        flag("--nofile", self.open_files.to_string().into());
        flag("--nproc", self.processes.to_string().into());
        for path in &self.read {
            flag("--read", path.clone().into_os_string());
        }
        for path in &self.write {
            flag("--write", path.clone().into_os_string());
        }
        for (name, value) in &self.env {
            flag("--env", format!("{name}={value}").into());
        }
        out.push("--".into());
        out.push(self.program.clone());
        out.extend(self.args.iter().cloned());
        out
    }

    /// Decodes helper arguments produced by [`HelperRequest::encode`].
    ///
    /// # Errors
    /// [`RuntimeError::Sandbox`] on an unknown flag, a missing value, a
    /// non-numeric limit, a missing required flag or a missing program.
    pub fn decode(args: &[OsString]) -> Result<Self, RuntimeError> {
        let bad = |what: String| RuntimeError::Sandbox(format!("helper arguments: {what}"));
        let mut status = None;
        let mut cwd = None;
        let mut limits = [None::<u64>; 5];
        let (mut read, mut write, mut env) = (Vec::new(), Vec::new(), Vec::new());
        let mut iter = args.iter();
        let program = loop {
            let Some(flag) = iter.next() else {
                return Err(bad("missing `--` and program".into()));
            };
            if flag == "--" {
                break iter
                    .next()
                    .cloned()
                    .ok_or_else(|| bad("missing program".into()))?;
            }
            let value = iter
                .next()
                .ok_or_else(|| bad(format!("{} has no value", flag.to_string_lossy())))?;
            let number = |value: &OsStr| {
                value
                    .to_str()
                    .and_then(|text| text.parse::<u64>().ok())
                    .ok_or_else(|| bad(format!("{} is not a number", flag.to_string_lossy())))
            };
            match flag.to_str() {
                Some("--status") => status = Some(PathBuf::from(value)),
                Some("--cwd") => cwd = Some(PathBuf::from(value)),
                Some("--cpu") => limits[0] = Some(number(value)?),
                Some("--mem") => limits[1] = Some(number(value)?),
                Some("--fsize") => limits[2] = Some(number(value)?),
                Some("--nofile") => limits[3] = Some(number(value)?),
                Some("--nproc") => limits[4] = Some(number(value)?),
                Some("--read") => read.push(PathBuf::from(value)),
                Some("--write") => write.push(PathBuf::from(value)),
                Some("--env") => {
                    let text = value
                        .to_str()
                        .ok_or_else(|| bad("non-UTF-8 --env".into()))?;
                    let (name, val) = text
                        .split_once('=')
                        .ok_or_else(|| bad(format!("--env {text} has no `=`")))?;
                    env.push((name.to_owned(), val.to_owned()));
                }
                _ => return Err(bad(format!("unknown flag {}", flag.to_string_lossy()))),
            }
        };
        let [Some(cpu_secs), Some(memory_bytes), Some(file_bytes), Some(open_files), Some(processes)] =
            limits
        else {
            return Err(bad("a resource limit is missing".into()));
        };
        Ok(Self {
            status: status.ok_or_else(|| bad("--status is missing".into()))?,
            cwd: cwd.ok_or_else(|| bad("--cwd is missing".into()))?,
            cpu_secs,
            memory_bytes,
            file_bytes,
            open_files,
            processes,
            read,
            write,
            env,
            program,
            args: iter.cloned().collect(),
        })
    }
}

/// Requires that a confinement step was actually enforced (fail closed).
///
/// # Errors
/// [`RuntimeError::Sandbox`] naming `what` for anything but `Enforced`.
#[cfg(target_os = "linux")]
pub fn require_enforced(
    what: &str,
    status: platform::security::SandboxStatus,
) -> Result<(), RuntimeError> {
    match status {
        platform::security::SandboxStatus::Enforced => Ok(()),
        other => Err(RuntimeError::Sandbox(format!(
            "{what} is {other:?} on this kernel"
        ))),
    }
}

/// The helper's entry point: confine this process and `exec` the program.
///
/// Returns only on failure; the caller reports the error and exits with
/// [`SETUP_FAILED`]. The error has also been written to the status file
/// when that file could be opened.
#[must_use]
pub fn run_helper(args: &[OsString]) -> RuntimeError {
    let request = match HelperRequest::decode(args) {
        Ok(request) => request,
        Err(error) => return error,
    };
    let mut status = match std::fs::OpenOptions::new()
        .write(true)
        .open(&request.status)
    {
        Ok(file) => file,
        Err(error) => return RuntimeError::io("opening the sandbox status file", error),
    };
    let error = confine_and_exec(&request);
    if let Err(write_error) = std::io::Write::write_all(&mut status, error.to_string().as_bytes()) {
        return RuntimeError::Sandbox(format!("{error} (and the status file: {write_error})"));
    }
    error
}

#[cfg(target_os = "linux")]
fn confine_and_exec(request: &HelperRequest) -> RuntimeError {
    use std::os::unix::process::CommandExt;

    if let Err(error) = set_limits(request) {
        return error;
    }
    if let Err(error) = confine(request) {
        return error;
    }
    let exec_error = std::process::Command::new(&request.program)
        .args(&request.args)
        .env_clear()
        .envs(request.env.iter().map(|(name, value)| (name, value)))
        .current_dir(&request.cwd)
        .exec();
    RuntimeError::io(
        format!("executing {}", request.program.to_string_lossy()),
        exec_error,
    )
}

#[cfg(not(target_os = "linux"))]
fn confine_and_exec(_request: &HelperRequest) -> RuntimeError {
    RuntimeError::Sandbox("sandboxed execution is only implemented on Linux".into())
}

#[cfg(target_os = "linux")]
fn set_limits(request: &HelperRequest) -> Result<(), RuntimeError> {
    use rusty_libc::rlimit::{
        setrlimit, Rlimit, RLIMIT_AS, RLIMIT_CORE, RLIMIT_CPU, RLIMIT_FSIZE, RLIMIT_NOFILE,
        RLIMIT_NPROC,
    };
    // CPU: SIGXCPU at the soft limit, SIGKILL one second later at the hard one.
    let limits = [
        (
            RLIMIT_CPU,
            request.cpu_secs,
            request.cpu_secs.saturating_add(1),
            "CPU",
        ),
        (
            RLIMIT_AS,
            request.memory_bytes,
            request.memory_bytes,
            "memory",
        ),
        (
            RLIMIT_FSIZE,
            request.file_bytes,
            request.file_bytes,
            "file size",
        ),
        (
            RLIMIT_NOFILE,
            request.open_files,
            request.open_files,
            "open files",
        ),
        (
            RLIMIT_NPROC,
            request.processes,
            request.processes,
            "processes",
        ),
        (RLIMIT_CORE, 0, 0, "core dumps"),
    ];
    for (resource, cur, max, name) in limits {
        setrlimit(resource, &Rlimit { cur, max })
            .map_err(|errno| RuntimeError::Sandbox(format!("setting the {name} limit: {errno}")))?;
    }
    Ok(())
}

#[cfg(target_os = "linux")]
fn confine(request: &HelperRequest) -> Result<(), RuntimeError> {
    use platform::security::Sandbox;

    let sandbox = platform_linux::LinuxSandbox;
    let read: Vec<&std::path::Path> = request.read.iter().map(PathBuf::as_path).collect();
    let write: Vec<&std::path::Path> = request.write.iter().map(PathBuf::as_path).collect();
    let fs = sandbox
        .confine_filesystem(&read, &write)
        .map_err(|error| RuntimeError::Sandbox(format!("Landlock: {error}")))?;
    require_enforced("Landlock filesystem confinement", fs)?;
    let net = sandbox
        .block_inet_sockets()
        .map_err(|error| RuntimeError::Sandbox(format!("seccomp: {error}")))?;
    require_enforced("seccomp socket blocking", net)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn request() -> HelperRequest {
        HelperRequest {
            status: "/run/status".into(),
            cwd: "/work/run 1".into(),
            cpu_secs: 2,
            memory_bytes: 1 << 29,
            file_bytes: 1 << 20,
            open_files: 64,
            processes: 32,
            read: vec!["/usr".into(), "/lib".into()],
            write: vec!["/work/run 1".into()],
            env: vec![
                ("PATH".into(), "/usr/bin".into()),
                ("RSI_SEED".into(), "7".into()),
            ],
            program: "python3".into(),
            args: vec!["solution.py".into(), "--".into(), "--cpu".into()],
        }
    }

    #[test]
    fn encode_decode_round_trips() {
        let original = request();
        assert_eq!(
            HelperRequest::decode(&original.encode()).ok(),
            Some(original)
        );
    }

    #[test]
    fn program_arguments_that_look_like_flags_are_kept() {
        let decoded = HelperRequest::decode(&request().encode()).expect("valid");
        assert_eq!(
            decoded.args,
            vec![OsString::from("solution.py"), "--".into(), "--cpu".into()]
        );
    }

    #[test]
    fn decode_rejects_malformed_arguments() {
        let encoded = request().encode();
        let without = |flag: &str| {
            let at = encoded
                .iter()
                .position(|a| a == flag)
                .expect("flag present");
            let mut args = encoded.clone();
            args.drain(at..at + 2);
            args
        };
        for flag in [
            "--status", "--cwd", "--cpu", "--mem", "--fsize", "--nofile", "--nproc",
        ] {
            assert!(HelperRequest::decode(&without(flag)).is_err(), "{flag}");
        }
        let os = |items: &[&str]| items.iter().map(OsString::from).collect::<Vec<_>>();
        assert!(HelperRequest::decode(&os(&["--bogus", "1", "--", "x"])).is_err());
        assert!(HelperRequest::decode(&os(&["--cpu", "two", "--", "x"])).is_err());
        assert!(HelperRequest::decode(&os(&["--env", "NOEQUALS", "--", "x"])).is_err());
        assert!(HelperRequest::decode(&os(&["--cpu"])).is_err());
        let truncated = &encoded[..encoded.len() - 4];
        assert!(HelperRequest::decode(truncated).is_err(), "no program");
    }

    #[cfg(target_os = "linux")]
    #[test]
    fn only_enforced_confinement_is_accepted() {
        use platform::security::SandboxStatus;
        assert!(require_enforced("x", SandboxStatus::Enforced).is_ok());
        for status in [SandboxStatus::NotEnforced, SandboxStatus::Unsupported] {
            assert!(matches!(
                require_enforced("x", status),
                Err(RuntimeError::Sandbox(_))
            ));
        }
    }
}
