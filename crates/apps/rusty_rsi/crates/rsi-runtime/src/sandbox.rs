//! The sandbox helper: the process that confines itself and then becomes
//! the untrusted program (ADR-0005 §4).
//!
//! Landlock and seccomp apply to the calling thread and its descendants,
//! and installing them after `fork` in a multithreaded process is unsafe.
//! So the executor never confines itself: it spawns the `rsi` binary with
//! the `__sandbox` subcommand (single-threaded from birth), which runs
//! [`run_helper`]: set rlimits, restrict the filesystem to an allowlist,
//! block internet sockets (or, for the inner agent, every new socket),
//! verify each step was enforced, then `exec`.
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

/// Which sockets a sandboxed process may create.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Sockets {
    /// Any socket, the internet included. Only the coding-agent CLIs run
    /// like this: they must reach their models, and their filesystem confinement
    /// keeps private task data out of reach (see [`crate::agent_cli`]).
    Internet,
    /// Anything but internet sockets (solutions).
    NoInternet,
    /// Nothing that can reach an endpoint: `socket(2)` is refused, so no
    /// network, Unix or abstract socket. Anonymous `socketpair(2)`s still
    /// work: they connect only processes inside the job. This is for the
    /// harness build, because rustc starts its linker through std's
    /// fork-and-exec path, which reports exec errors over a socketpair.
    NoEndpoints,
    /// No new socket of any kind (the inner agent, whose only channel is
    /// the broker socket it inherits).
    None,
}

impl Sockets {
    const fn flag(self) -> Option<&'static str> {
        match self {
            Self::NoInternet => None,
            Self::Internet => Some("internet"),
            Self::NoEndpoints => Some("no-endpoints"),
            Self::None => Some("none"),
        }
    }
}

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
    /// Which sockets the program may create.
    pub sockets: Sockets,
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
        if let Some(rule) = self.sockets.flag() {
            flag("--sockets", rule.into());
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
        let mut sockets = Sockets::NoInternet;
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
                Some("--sockets") => {
                    sockets = [Sockets::Internet, Sockets::NoEndpoints, Sockets::None]
                        .into_iter()
                        .find(|rule| rule.flag().is_some_and(|f| value == f))
                        .ok_or_else(|| {
                            bad(format!("unknown --sockets {}", value.to_string_lossy()))
                        })?;
                }
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
            sockets,
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
    if request.sockets != Sockets::Internet {
        let net = sandbox
            .block_inet_sockets()
            .map_err(|error| RuntimeError::Sandbox(format!("seccomp: {error}")))?;
        require_enforced("seccomp socket blocking", net)?;
    }
    group_lock::install(request.sockets)
}

/// Keeps every descendant in the job's process group (review finding 2),
/// closes `io_uring`, and refuses sockets per [`Sockets`].
///
/// The executor contains a job by killing its process group, and a process
/// can only leave a group through `setsid` or `setpgid`. This second
/// seccomp filter (filters stack, are inherited by every descendant, and
/// the strictest verdict wins) makes both fail with `EPERM`, so the group
/// is the whole job. It also refuses every x32-ABI syscall on x86_64, since
/// those bypass number-based checks.
///
/// `io_uring` is refused for every sandboxed process: its operations
/// (`IORING_OP_SOCKET`, `IORING_OP_CONNECT`, ...) run without passing
/// through seccomp, so it would reopen whatever this filter and the socket
/// filter close.
///
/// [`Sockets::NoEndpoints`] makes `socket(2)` fail with `EPERM` for every
/// address family, and [`Sockets::None`] `socketpair(2)` too. The inner
/// agent then has no way to talk to anything but the broker socket it
/// inherited: not the network, and not a local Unix or abstract socket
/// either, which Landlock does not cover.
#[cfg(target_os = "linux")]
mod group_lock {
    use rusty_libc::arch::{nr, syscall3};

    use super::Sockets;
    use crate::error::RuntimeError;

    #[repr(C)]
    #[derive(Debug, Clone, Copy, PartialEq, Eq)]
    pub(super) struct SockFilter {
        code: u16,
        jt: u8,
        jf: u8,
        k: u32,
    }

    #[repr(C)]
    struct SockFprog {
        len: u16,
        filter: *const SockFilter,
    }

    const LD_W_ABS: u16 = 0x20; // BPF_LD | BPF_W | BPF_ABS
    const JEQ_K: u16 = 0x15; // BPF_JMP | BPF_JEQ | BPF_K
    const JSET_K: u16 = 0x45; // BPF_JMP | BPF_JSET | BPF_K
    const RET_K: u16 = 0x06; // BPF_RET | BPF_K
    const RET_ALLOW: u32 = 0x7fff_0000;
    const RET_KILL_PROCESS: u32 = 0x8000_0000;
    const RET_EPERM: u32 = 0x0005_0000 | 1; // SECCOMP_RET_ERRNO | EPERM
    const OFFSET_NR: u32 = 0; // struct seccomp_data.nr
    const OFFSET_ARCH: u32 = 4; // struct seccomp_data.arch
    const X32_SYSCALL_BIT: u32 = 0x4000_0000;
    const PR_SET_SECCOMP: usize = 22;
    const SECCOMP_MODE_FILTER: usize = 2;

    #[cfg(target_arch = "x86_64")]
    const AUDIT_ARCH: u32 = 0xC000_003E;
    #[cfg(target_arch = "aarch64")]
    const AUDIT_ARCH: u32 = 0xC000_00B7;

    // Not in rusty_libc's table. io_uring's numbers are the same on every
    // architecture (asm-generic); socketpair's are per architecture.
    const IO_URING_SETUP: u32 = 425;
    const IO_URING_ENTER: u32 = 426;
    const IO_URING_REGISTER: u32 = 427;
    #[cfg(target_arch = "x86_64")]
    pub(super) const SOCKETPAIR: u32 = 53;
    #[cfg(target_arch = "aarch64")]
    pub(super) const SOCKETPAIR: u32 = 199;

    const fn stmt(code: u16, k: u32) -> SockFilter {
        SockFilter {
            code,
            jt: 0,
            jf: 0,
            k,
        }
    }

    const fn jump(code: u16, k: u32, jt: u8, jf: u8) -> SockFilter {
        SockFilter { code, jt, jf, k }
    }

    /// The syscalls refused with `EPERM`.
    pub(super) fn denied(sockets: Sockets) -> Vec<u32> {
        let mut denied = vec![
            nr::SETSID as u32,
            nr::SETPGID as u32,
            IO_URING_SETUP,
            IO_URING_ENTER,
            IO_URING_REGISTER,
        ];
        match sockets {
            Sockets::Internet | Sockets::NoInternet => {}
            Sockets::NoEndpoints => denied.push(nr::SOCKET as u32),
            Sockets::None => denied.extend([nr::SOCKET as u32, SOCKETPAIR]),
        }
        denied
    }

    /// The filter: foreign architecture kills; x32 and `denied` get
    /// `EPERM`; everything else is allowed. Jump offsets count from the
    /// next instruction.
    pub(super) fn program(denied: &[u32]) -> Vec<SockFilter> {
        let n = denied.len();
        // Layout: 0 load arch, 1 check arch, 2 load nr, 3 x32 check,
        // 4..4+n denied checks, then ALLOW, EPERM, KILL.
        let allow = 4 + n;
        let (eperm, kill) = (allow + 1, allow + 2);
        let offset = |from: usize, to: usize| (to - from - 1) as u8;
        let mut program = vec![
            stmt(LD_W_ABS, OFFSET_ARCH),
            jump(JEQ_K, AUDIT_ARCH, 0, offset(1, kill)),
            stmt(LD_W_ABS, OFFSET_NR),
            jump(JSET_K, X32_SYSCALL_BIT, offset(3, eperm), 0),
        ];
        for (i, &syscall) in denied.iter().enumerate() {
            program.push(jump(JEQ_K, syscall, offset(4 + i, eperm), 0));
        }
        program.extend([
            stmt(RET_K, RET_ALLOW),
            stmt(RET_K, RET_EPERM),
            stmt(RET_K, RET_KILL_PROCESS),
        ]);
        program
    }

    /// Installs the filter on the calling thread (inherited by children).
    ///
    /// Requires `no_new_privs`, which the Landlock step has already set.
    pub(super) fn install(sockets: Sockets) -> Result<(), RuntimeError> {
        let program = program(&denied(sockets));
        let fprog = SockFprog {
            len: program.len() as u16,
            filter: program.as_ptr(),
        };
        // SAFETY: `fprog` points at `program`, a live, correctly laid out
        // `sock_filter` array of `len` entries that outlives the call; the
        // kernel copies the program before `prctl` returns.
        let ret = unsafe {
            syscall3(
                nr::PRCTL,
                PR_SET_SECCOMP,
                SECCOMP_MODE_FILTER,
                core::ptr::addr_of!(fprog) as usize,
            )
        };
        rusty_libc::from_ret(ret).map(|_| ()).map_err(|errno| {
            RuntimeError::Sandbox(format!("process-group lock (seccomp): {errno}"))
        })
    }

    /// Runs `program` on one syscall the way the kernel would.
    #[cfg(test)]
    pub(super) fn verdict(program: &[SockFilter], arch: u32, syscall: u32) -> u32 {
        let (mut pc, mut acc) = (0usize, 0u32);
        loop {
            let ins = program[pc];
            pc += 1;
            match ins.code {
                LD_W_ABS if ins.k == OFFSET_ARCH => acc = arch,
                LD_W_ABS => acc = syscall,
                JEQ_K | JSET_K => {
                    let hit = if ins.code == JEQ_K {
                        acc == ins.k
                    } else {
                        acc & ins.k != 0
                    };
                    pc += usize::from(if hit { ins.jt } else { ins.jf });
                }
                _ => return ins.k,
            }
        }
    }

    #[cfg(test)]
    pub(super) const VERDICTS: (u32, u32, u32, u32) =
        (AUDIT_ARCH, RET_ALLOW, RET_EPERM, RET_KILL_PROCESS);
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
            sockets: Sockets::None,
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
    fn every_socket_rule_round_trips() {
        for sockets in [
            Sockets::Internet,
            Sockets::NoInternet,
            Sockets::NoEndpoints,
            Sockets::None,
        ] {
            let original = HelperRequest {
                sockets,
                ..request()
            };
            let decoded = HelperRequest::decode(&original.encode()).expect("valid");
            assert_eq!(decoded.sockets, sockets);
        }
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
    fn the_seccomp_program_denies_exactly_what_it_lists() {
        use rusty_libc::arch::nr;
        let (arch, allow, eperm, kill) = group_lock::VERDICTS;
        for sockets in [
            Sockets::Internet,
            Sockets::NoInternet,
            Sockets::NoEndpoints,
            Sockets::None,
        ] {
            let denied = group_lock::denied(sockets);
            let program = group_lock::program(&denied);
            for &syscall in &denied {
                assert_eq!(group_lock::verdict(&program, arch, syscall), eperm);
            }
            let socket = group_lock::verdict(&program, arch, nr::SOCKET as u32);
            let pair = group_lock::verdict(&program, arch, group_lock::SOCKETPAIR);
            let expected = match sockets {
                Sockets::Internet | Sockets::NoInternet => (allow, allow),
                Sockets::NoEndpoints => (eperm, allow),
                Sockets::None => (eperm, eperm),
            };
            assert_eq!((socket, pair), expected, "{sockets:?}");
            for syscall in [nr::PRCTL as u32, 0, 1, 300] {
                assert_eq!(group_lock::verdict(&program, arch, syscall), allow);
            }
            let x32 = 0x4000_0000 | nr::SOCKET as u32;
            assert_eq!(group_lock::verdict(&program, arch, x32), eperm);
            assert_eq!(group_lock::verdict(&program, arch ^ 1, 0), kill);
        }
        assert_eq!(
            group_lock::denied(Sockets::None).len(),
            7,
            "io_uring x3, group x2, sockets x2"
        );
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
