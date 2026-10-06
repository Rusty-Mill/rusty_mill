//! Building and running the inner agent (ADR-0005 §3, §4).
//!
//! - [`build`] compiles a harness candidate's `src/` with plain `rustc` in
//!   the sandbox. The source is copied first (regular files only, so a
//!   symlink cannot pull anything else into the build), and the sandbox can
//!   read only the copy, the toolchain and system directories. A compile
//!   error is an outcome, [`BuildFailure`], not an infrastructure error.
//! - [`HarnessProcess`] runs a built agent in the sandbox with no sockets of
//!   its own: its standard input is one end of a socket pair whose other end
//!   the [`broker`](crate::broker) serves. The wall-clock limit is the
//!   budget plus a grace period, after which the whole job is killed.
//! - [`SandboxedHarness`] is the [`Harness`] port: a live, metered session.
//!   [`HarnessProcess::replay`] re-runs a recorded one.

use std::ffi::OsStr;
use std::path::{Path, PathBuf};
use std::process::Stdio;
use std::sync::atomic::{AtomicU64, Ordering};
use std::time::Duration;

use rsi_core::{
    Budget, ChatModel, ExecOutcome, Harness, InnerOutcome, Limits, PublicTask, SandboxSpec, Seed,
    Solution, Termination,
};

use crate::broker::{final_submission, LiveService, ReplayService, Service};
use crate::error::RuntimeError;
use crate::executor::ProcessExecutor;
use crate::grading::SYSTEM_READ_ROOTS;
use crate::protocol::{decode_transcript, encode_transcript, Exchange};
use crate::sandbox::Sockets;
use crate::task_dir::{DESCRIPTION_FILE, OPEN_FILE_LIMIT, PROCESS_LIMIT};

/// The binary a successful build produces, inside the build's `bin/`.
pub const BINARY_NAME: &str = "rsi-harness";

/// The largest harness source tree accepted, in bytes.
pub const MAX_SOURCE_BYTES: u64 = 4 << 20;

/// How long a sandboxed build may take.
const BUILD_TIME: Duration = Duration::from_secs(300);

/// Address space for `rustc` and the linker.
const BUILD_MEMORY_BYTES: u64 = 4 << 30;

/// Largest file a build may write.
const BUILD_FILE_BYTES: u64 = 1 << 30;

/// Address space for the running agent.
const AGENT_MEMORY_BYTES: u64 = 2 << 30;

/// Largest file the agent may write in its work directory.
const AGENT_FILE_BYTES: u64 = 64 << 20;

static NEXT_SESSION: AtomicU64 = AtomicU64::new(0);

/// The compiler used to build harness candidates.
#[derive(Debug, Clone)]
pub struct Toolchain {
    rustc: PathBuf,
    sysroot: PathBuf,
}

impl Toolchain {
    /// Locates the real compiler behind `rustc` (often a rustup proxy) by
    /// asking it for its sysroot. This runs outside the sandbox: it is the
    /// runtime's own, trusted toolchain.
    ///
    /// # Errors
    /// [`RuntimeError::Io`] when `rustc` cannot run, and
    /// [`RuntimeError::Sandbox`] when its sysroot has no `bin/rustc`.
    pub fn detect(rustc: &OsStr) -> Result<Self, RuntimeError> {
        let output = std::process::Command::new(rustc)
            .args(["--print", "sysroot"])
            .output()
            .map_err(|e| RuntimeError::io(format!("running {}", rustc.to_string_lossy()), e))?;
        if !output.status.success() {
            return Err(RuntimeError::Sandbox(format!(
                "{} --print sysroot failed: {}",
                rustc.to_string_lossy(),
                String::from_utf8_lossy(&output.stderr).trim()
            )));
        }
        let sysroot = PathBuf::from(String::from_utf8_lossy(&output.stdout).trim());
        let sysroot = sysroot
            .canonicalize()
            .map_err(|e| RuntimeError::io(format!("resolving sysroot {}", sysroot.display()), e))?;
        let rustc = sysroot
            .join("bin")
            .join(format!("rustc{}", std::env::consts::EXE_SUFFIX));
        if !rustc.is_file() {
            return Err(RuntimeError::Sandbox(format!(
                "no compiler at {}",
                rustc.display()
            )));
        }
        Ok(Self { rustc, sysroot })
    }
}

/// Why a candidate did not build: the compiler's output, or the reason its
/// source was refused. The candidate is buggy; nothing ran.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct BuildFailure(pub String);

/// A built agent binary.
#[derive(Debug, Clone)]
pub struct HarnessBinary {
    dir: PathBuf,
}

impl HarnessBinary {
    /// The binary's path.
    #[must_use]
    pub fn path(&self) -> PathBuf {
        self.dir.join(BINARY_NAME)
    }
}

/// Builds the harness crate at `crate_dir` (its `src/lib.rs` is the binary
/// root) into the fresh directory `out`.
///
/// # Errors
/// Infrastructure failures only: `out` already exists, a file cannot be
/// copied, or the sandbox cannot run.
pub fn build(
    executor: &ProcessExecutor,
    toolchain: &Toolchain,
    crate_dir: &Path,
    out: &Path,
) -> Result<Result<HarnessBinary, BuildFailure>, RuntimeError> {
    std::fs::create_dir_all(out.parent().unwrap_or(out))
        .map_err(|e| RuntimeError::io("creating the build parent", e))?;
    std::fs::create_dir(out).map_err(|e| RuntimeError::io("creating a fresh build dir", e))?;
    let out = out
        .canonicalize()
        .map_err(|e| RuntimeError::io("resolving the build dir", e))?;
    let mut budget = MAX_SOURCE_BYTES;
    if let Err(failure) = copy_source(&crate_dir.join("src"), &out.join("src"), &mut budget)? {
        return Ok(Err(failure));
    }
    for dir in ["bin", "tmp"] {
        std::fs::create_dir(out.join(dir))
            .map_err(|e| RuntimeError::io("creating build dirs", e))?;
    }
    let mut read = system_roots()?;
    read.push(toolchain.sysroot.clone());
    let tmp = out.join("tmp").display().to_string();
    let env = [
        ("PATH", "/usr/local/bin:/usr/bin:/bin".to_owned()),
        ("HOME", tmp.clone()),
        ("TMPDIR", tmp),
    ];
    let spec = SandboxSpec::new(
        read,
        vec![out.clone()],
        out.clone(),
        env.into_iter().map(|(k, v)| (k.to_owned(), v)).collect(),
        Limits::new(
            BUILD_TIME,
            BUILD_TIME,
            BUILD_MEMORY_BYTES,
            BUILD_FILE_BYTES,
            1024,
            PROCESS_LIMIT,
        )?,
    )?;
    let binary = format!("bin/{BINARY_NAME}");
    let args = [
        "--edition",
        "2021",
        "-O",
        "--crate-type",
        "bin",
        "--crate-name",
        "rsi_harness",
        "src/lib.rs",
        "-o",
        &binary,
    ]
    .map(str::to_owned);
    // rustc starts its linker over a socketpair (see `Sockets::NoEndpoints`).
    let outcome = executor.exec_with(
        &spec,
        &toolchain.rustc.display().to_string(),
        &args,
        Stdio::null(),
        Sockets::NoEndpoints,
    )?;
    let built = HarnessBinary {
        dir: out.join("bin"),
    };
    if outcome.termination == Termination::Exited(0) && built.path().is_file() {
        return Ok(Ok(built));
    }
    Ok(Err(BuildFailure(format!(
        "rustc {:?}\n{}",
        outcome.termination,
        String::from_utf8_lossy(&outcome.stderr)
    ))))
}

/// Copies `from` to `to`: directories and regular files only, within the
/// byte budget.
fn copy_source(
    from: &Path,
    to: &Path,
    budget: &mut u64,
) -> Result<Result<(), BuildFailure>, RuntimeError> {
    let refuse = |what: String| Ok(Err(BuildFailure(what)));
    let kind = std::fs::symlink_metadata(from)
        .map_err(|e| RuntimeError::io(format!("inspecting {}", from.display()), e))?;
    if !kind.is_dir() {
        return refuse(format!("{} is not a directory", from.display()));
    }
    std::fs::create_dir(to).map_err(|e| RuntimeError::io("staging source", e))?;
    let entries = std::fs::read_dir(from)
        .map_err(|e| RuntimeError::io(format!("listing {}", from.display()), e))?;
    for entry in entries {
        let entry = entry.map_err(|e| RuntimeError::io("listing source", e))?;
        let path = entry.path();
        let meta = std::fs::symlink_metadata(&path)
            .map_err(|e| RuntimeError::io(format!("inspecting {}", path.display()), e))?;
        let target = to.join(entry.file_name());
        if meta.is_dir() {
            if let Err(failure) = copy_source(&path, &target, budget)? {
                return Ok(Err(failure));
            }
        } else if meta.is_file() {
            if meta.len() > *budget {
                return refuse(format!(
                    "the source exceeds {MAX_SOURCE_BYTES} bytes at {}",
                    path.display()
                ));
            }
            *budget -= meta.len();
            std::fs::copy(&path, &target)
                .map_err(|e| RuntimeError::io(format!("copying {}", path.display()), e))?;
        } else {
            return refuse(format!(
                "{} is not a regular file or directory",
                path.display()
            ));
        }
    }
    Ok(Ok(()))
}

fn system_roots() -> Result<Vec<PathBuf>, RuntimeError> {
    let mut roots = Vec::new();
    for root in SYSTEM_READ_ROOTS
        .iter()
        .map(Path::new)
        .filter(|r| r.exists())
    {
        let root = root
            .canonicalize()
            .map_err(|e| RuntimeError::io(format!("resolving {}", root.display()), e))?;
        if !roots.contains(&root) {
            roots.push(root);
        }
    }
    Ok(roots)
}

/// Runs a built agent in the sandbox against a broker [`Service`].
#[derive(Debug)]
pub struct HarnessProcess<'a> {
    executor: &'a ProcessExecutor,
    binary: HarnessBinary,
    read_roots: Vec<PathBuf>,
    scratch: PathBuf,
    protected: Vec<PathBuf>,
    grace: Duration,
}

/// What one agent session produced.
#[derive(Debug)]
pub struct Session {
    /// Every request and response, in order.
    pub exchanges: Vec<Exchange>,
    /// How the agent process ended.
    pub outcome: ExecOutcome,
}

impl<'a> HarnessProcess<'a> {
    /// Runs `binary` with fresh work directories under `scratch`. Every
    /// session refuses to start if its sandbox could reach any path in
    /// `protected` (task directories, lineage, the executor's state).
    /// `grace` is how long past its wall-clock budget the agent may run
    /// before the job is killed.
    ///
    /// # Errors
    /// [`RuntimeError::Io`] when a path cannot be created or resolved.
    pub fn new(
        executor: &'a ProcessExecutor,
        binary: HarnessBinary,
        scratch: &Path,
        protected: &[PathBuf],
        grace: Duration,
    ) -> Result<Self, RuntimeError> {
        std::fs::create_dir_all(scratch).map_err(|e| RuntimeError::io("creating scratch", e))?;
        let resolve = |path: &Path| {
            path.canonicalize()
                .map_err(|e| RuntimeError::io(format!("resolving {}", path.display()), e))
        };
        let binary = HarnessBinary {
            dir: resolve(&binary.dir)?,
        };
        let mut read_roots = system_roots()?;
        read_roots.push(binary.dir.clone());
        Ok(Self {
            executor,
            binary,
            read_roots,
            scratch: resolve(scratch)?,
            protected: protected
                .iter()
                .map(|p| resolve(p))
                .collect::<Result<_, _>>()?,
            grace,
        })
    }

    /// Runs one session: the agent sees `description` as `task.md` and
    /// `--seed`, and is killed `grace` after `wall`.
    ///
    /// # Errors
    /// [`RuntimeError::Sandbox`] if a protected path would be reachable or
    /// the sandbox fails; whatever `service` fails with.
    pub fn session(
        &self,
        description: &str,
        seed: Seed,
        wall: Duration,
        service: &mut impl Service,
    ) -> Result<Session, RuntimeError> {
        let work = self.scratch.join(format!(
            "agent-{}-{}",
            std::process::id(),
            NEXT_SESSION.fetch_add(1, Ordering::Relaxed)
        ));
        std::fs::create_dir(&work).map_err(|e| RuntimeError::io("creating a work dir", e))?;
        let result = self.session_in(&work, description, seed, wall, service);
        let removed = std::fs::remove_dir_all(&work)
            .map_err(|e| RuntimeError::io(format!("removing {}", work.display()), e));
        let session = result?;
        removed?;
        Ok(session)
    }

    fn session_in(
        &self,
        work: &Path,
        description: &str,
        seed: Seed,
        wall: Duration,
        service: &mut impl Service,
    ) -> Result<Session, RuntimeError> {
        let limit = wall.saturating_add(self.grace);
        let spec = SandboxSpec::new(
            self.read_roots.clone(),
            vec![work.to_path_buf()],
            work.to_path_buf(),
            vec![("HOME".to_owned(), work.display().to_string())],
            Limits::new(
                limit,
                limit,
                AGENT_MEMORY_BYTES,
                AGENT_FILE_BYTES,
                OPEN_FILE_LIMIT,
                PROCESS_LIMIT,
            )?,
        )?;
        if let Some(path) = self.protected.iter().find(|p| spec.can_reach(p)) {
            return Err(RuntimeError::Sandbox(format!(
                "{} is reachable from the agent's sandbox; refusing to run",
                path.display()
            )));
        }
        std::fs::write(work.join(DESCRIPTION_FILE), description)
            .map_err(|e| RuntimeError::io("writing task.md", e))?;
        let program = self.binary.path().display().to_string();
        let args = ["--seed".to_owned(), seed.get().to_string()];
        connect_and_run(service, |stdin| {
            self.executor
                .exec_with(&spec, &program, &args, stdin, Sockets::None)
                .map_err(RuntimeError::from)
        })
    }

    /// Re-runs a recorded session: the agent's requests must match the
    /// recording, and its responses come from the recording, not the model
    /// or the scorer. Returns the replayed final submission.
    ///
    /// # Errors
    /// [`RuntimeError::Broker`] when the transcript is malformed or the
    /// agent diverges from it; otherwise as [`HarnessProcess::session`].
    pub fn replay(
        &self,
        description: &str,
        seed: Seed,
        wall: Duration,
        transcript: &[u8],
    ) -> Result<Option<Solution>, RuntimeError> {
        let mut service = ReplayService::new(decode_transcript(transcript)?);
        let session = self.session(description, seed, wall, &mut service)?;
        service.finish()?;
        Ok(final_submission(&session.exchanges))
    }
}

/// Starts the agent with one end of a socket pair as its stdin and serves
/// the other end on this thread until the agent hangs up or dies.
#[cfg(unix)]
fn connect_and_run(
    service: &mut impl Service,
    spawn: impl FnOnce(Stdio) -> Result<ExecOutcome, RuntimeError> + Send,
) -> Result<Session, RuntimeError> {
    use std::os::fd::OwnedFd;
    use std::os::unix::net::UnixStream;

    use crate::broker::{serve, MAX_TRANSCRIPT_BYTES};

    let (ours, theirs) =
        UnixStream::pair().map_err(|e| RuntimeError::io("creating the broker socket", e))?;
    std::thread::scope(|scope| {
        // The agent runs on its own thread; the service (which may borrow
        // non-thread-safe state, such as a scripted model) stays here.
        let agent = scope.spawn(move || spawn(Stdio::from(OwnedFd::from(theirs))));
        // `serve` takes the stream by value: when it returns, our end
        // closes and an agent still waiting on it reads end-of-file.
        let served = serve(ours, service, MAX_TRANSCRIPT_BYTES);
        let outcome = agent
            .join()
            .map_err(|_| RuntimeError::Sandbox("the agent thread panicked".into()))?;
        let exchanges = served?;
        Ok(Session {
            exchanges,
            outcome: outcome?,
        })
    })
}

#[cfg(not(unix))]
fn connect_and_run(
    _service: &mut impl Service,
    _spawn: impl FnOnce(Stdio) -> Result<ExecOutcome, RuntimeError> + Send,
) -> Result<Session, RuntimeError> {
    Err(RuntimeError::Sandbox(
        "sandboxed execution is only implemented on Linux".into(),
    ))
}

/// The [`Harness`] port: a built agent, a model, and metered sessions.
#[derive(Debug)]
pub struct SandboxedHarness<'a, M> {
    process: HarnessProcess<'a>,
    model: &'a M,
    max_completion_tokens: u64,
}

impl<'a, M> SandboxedHarness<'a, M> {
    /// Runs `process` against `model`, asking for at most
    /// `max_completion_tokens` per call.
    #[must_use]
    pub const fn new(
        process: HarnessProcess<'a>,
        model: &'a M,
        max_completion_tokens: u64,
    ) -> Self {
        Self {
            process,
            model,
            max_completion_tokens,
        }
    }

    /// The process, for replaying a recorded run.
    #[must_use]
    pub const fn process(&self) -> &HarnessProcess<'a> {
        &self.process
    }
}

/// Bytes of the agent's stderr kept in [`InnerOutcome::log`].
const LOG_BYTES: usize = 8 * 1024;

impl<M, T> Harness<T> for SandboxedHarness<'_, M>
where
    M: ChatModel,
    M::Error: std::fmt::Display,
    T: PublicTask<Error = RuntimeError>,
{
    type Error = RuntimeError;

    fn run(&self, task: &T, budget: &Budget, seed: Seed) -> Result<InnerOutcome, RuntimeError> {
        let mut service =
            LiveService::new(self.model, task, *budget, seed, self.max_completion_tokens);
        let session =
            self.process
                .session(task.description(), seed, budget.wall(), &mut service)?;
        let stderr = &session.outcome.stderr;
        Ok(InnerOutcome {
            submission: final_submission(&session.exchanges),
            usage: service.usage(),
            transcript: encode_transcript(&session.exchanges)?,
            log: String::from_utf8_lossy(&stderr[..stderr.len().min(LOG_BYTES)]).into_owned(),
        })
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    struct Dir(PathBuf);

    impl Dir {
        fn new(name: &str) -> Self {
            let dir =
                std::env::temp_dir().join(format!("rsi-harness-{name}-{}", std::process::id()));
            if dir.exists() {
                std::fs::remove_dir_all(&dir).expect("clear");
            }
            std::fs::create_dir_all(&dir).expect("create");
            Self(dir)
        }
    }

    impl Drop for Dir {
        fn drop(&mut self) {
            if !std::thread::panicking() {
                std::fs::remove_dir_all(&self.0).expect("remove");
            }
        }
    }

    #[test]
    fn copies_regular_source_files_only() {
        let dir = Dir::new("copy");
        let src = dir.0.join("src");
        std::fs::create_dir_all(src.join("nested")).expect("mkdir");
        std::fs::write(src.join("lib.rs"), "fn main() {}").expect("write");
        std::fs::write(src.join("nested/mod.rs"), "").expect("write");
        let mut budget = MAX_SOURCE_BYTES;
        assert_eq!(
            copy_source(&src, &dir.0.join("a"), &mut budget).expect("io"),
            Ok(())
        );
        assert!(dir.0.join("a/nested/mod.rs").is_file());
        assert_eq!(budget, MAX_SOURCE_BYTES - 12);

        let mut tiny = 4;
        assert!(copy_source(&src, &dir.0.join("b"), &mut tiny)
            .expect("io")
            .is_err());

        #[cfg(unix)]
        {
            std::os::unix::fs::symlink("/etc/hostname", src.join("leak.rs")).expect("symlink");
            let mut budget = MAX_SOURCE_BYTES;
            let refused = copy_source(&src, &dir.0.join("c"), &mut budget).expect("io");
            assert!(refused.is_err(), "symlinks are refused");
        }
    }
}
