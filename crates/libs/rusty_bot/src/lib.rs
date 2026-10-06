//! Per-bot sandboxes.
//!
//! A bot is an AG-UI agent process (anything that serves `POST /api/agent`:
//! `rusty_tick`, `echo_agent`, a Python server) run confined by
//! [`rusty_sandbox`]: its own workspace as the only writable directory, an
//! explicit list of readable roots, a complete environment, hard limits,
//! and a process group that dies as one. The network stays open, because
//! a bot listens on a port and may call a model; the filesystem is what
//! keeps one bot's data from another's. A [`Fleet`] starts a set of bots
//! and stops them.
//!
//! The library builds specs and drives the executor; it reads no clock
//! and spawns nothing itself except through [`rusty_sandbox`]. The
//! `rusty-bot` binary is both the sandbox helper (`rusty-bot __sandbox`,
//! single-threaded from birth) and the fleet runner (`rusty-bot run`).
//!
//! Step 7 of the ADR-0007 follow-ons, second PR.

use std::fmt;
use std::path::{Path, PathBuf};
use std::process::Stdio;
use std::sync::mpsc::{self, Receiver, TryRecvError};
use std::time::Duration;

use rusty_json::Value;
use rusty_sandbox::{ExecOutcome, JobHandle, Limits, ProcessExecutor, SandboxSpec, Sockets};

/// What can go wrong defining or running a bot.
#[derive(Debug)]
pub enum Error {
    /// A bots document that is not what [`load`] expects.
    Config(String),
    /// The sandbox refused the bot's spec, or could not be set up.
    Sandbox(rusty_sandbox::Error),
}

impl fmt::Display for Error {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Error::Config(why) => write!(f, "config: {why}"),
            Error::Sandbox(e) => write!(f, "sandbox: {e}"),
        }
    }
}

impl std::error::Error for Error {}

impl From<rusty_sandbox::Error> for Error {
    fn from(e: rusty_sandbox::Error) -> Self {
        Error::Sandbox(e)
    }
}

/// Limits a bot gets unless its spec says otherwise: a day of CPU and a
/// week of wall clock (a bot is long-lived; the process group, not the
/// clock, is how it is stopped), 2 GiB of address space, 1 GiB files,
/// 1024 descriptors, 256 processes (counted per user on Linux, so a brake
/// rather than a quota).
pub fn default_limits() -> Limits {
    Limits::new(
        Duration::from_secs(24 * 3600),
        Duration::from_secs(7 * 24 * 3600),
        2 << 30,
        1 << 30,
        1024,
        256,
    )
    .unwrap_or_else(|_| unreachable!("the defaults are valid"))
}

/// One bot: what to run and what it may touch.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct BotSpec {
    /// Unique within a fleet; used in logs and the state directory.
    pub name: String,
    /// The agent program, an absolute path.
    pub program: PathBuf,
    /// Its arguments.
    pub args: Vec<String>,
    /// Its complete environment; nothing is inherited.
    pub env: Vec<(String, String)>,
    /// The one directory the bot may write to, and its working directory.
    pub workspace: PathBuf,
    /// Directories the bot may read and execute from (its toolchain:
    /// `/usr`, `/lib`, ...). The program's own directory is added.
    pub read_roots: Vec<PathBuf>,
    /// Its limits.
    pub limits: Limits,
}

impl BotSpec {
    /// The sandbox this bot runs in: write only to its workspace, read its
    /// roots and the program's directory, start in the workspace with
    /// exactly `env`.
    ///
    /// # Errors
    /// [`Error::Sandbox`] for a relative path or a bad environment name.
    pub fn sandbox(&self) -> Result<SandboxSpec, Error> {
        let mut read = self.read_roots.clone();
        if let Some(dir) = self.program.parent() {
            if !read.iter().any(|root| dir.starts_with(root)) {
                read.push(dir.to_path_buf());
            }
        }
        Ok(SandboxSpec::new(
            read,
            vec![self.workspace.clone()],
            self.workspace.clone(),
            self.env.clone(),
            self.limits,
        )?)
    }
}

/// Read bots from a JSON array of `{"name", "program", "workspace",
/// "args"?, "env"?, "read"?}`. `env` is an object of strings; `read`
/// defaults to `/usr`, `/lib`, `/lib64`, `/bin` and `/etc` (a toolchain,
/// resolvers and certificates). Names must be unique.
pub fn load(json: &str) -> Result<Vec<BotSpec>, Error> {
    let value = Value::parse(json).map_err(|e| Error::Config(e.to_string()))?;
    let items = value
        .as_array()
        .ok_or_else(|| Error::Config("expected an array of bots".into()))?;
    let mut bots: Vec<BotSpec> = Vec::with_capacity(items.len());
    for (i, item) in items.iter().enumerate() {
        let field = |key: &str| {
            item.get(key)
                .and_then(Value::as_str)
                .ok_or_else(|| Error::Config(format!("bot {i}: no string {key:?}")))
        };
        let strings = |key: &str| -> Result<Vec<String>, Error> {
            match item.get(key) {
                None => Ok(Vec::new()),
                Some(v) => v
                    .as_array()
                    .map(|items| {
                        items
                            .iter()
                            .filter_map(Value::as_str)
                            .map(String::from)
                            .collect()
                    })
                    .filter(|out: &Vec<String>| Some(out.len()) == v.as_array().map(Vec::len))
                    .ok_or_else(|| {
                        Error::Config(format!("bot {i}: {key} must be an array of strings"))
                    }),
            }
        };
        let name = field("name")?;
        if bots.iter().any(|b| b.name == name) {
            return Err(Error::Config(format!("bot {i}: duplicate name {name:?}")));
        }
        let env = match item.get("env") {
            None => Vec::new(),
            Some(v) => v
                .as_object()
                .ok_or_else(|| Error::Config(format!("bot {i}: env must be an object")))?
                .iter()
                .map(|(k, v)| {
                    v.as_str()
                        .map(|v| (k.clone(), v.to_string()))
                        .ok_or_else(|| Error::Config(format!("bot {i}: env {k} must be a string")))
                })
                .collect::<Result<Vec<_>, _>>()?,
        };
        let read_roots = if item.get("read").is_some() {
            strings("read")?.into_iter().map(PathBuf::from).collect()
        } else {
            DEFAULT_READ_ROOTS.iter().map(PathBuf::from).collect()
        };
        bots.push(BotSpec {
            name: name.into(),
            program: PathBuf::from(field("program")?),
            args: strings("args")?,
            env,
            workspace: PathBuf::from(field("workspace")?),
            read_roots,
            limits: default_limits(),
        });
    }
    Ok(bots)
}

/// Read roots a bot gets unless its spec names its own.
pub const DEFAULT_READ_ROOTS: [&str; 5] = ["/usr", "/lib", "/lib64", "/bin", "/etc"];

/// A bot that has been started.
#[derive(Debug)]
pub struct Running {
    /// The bot's name.
    pub name: String,
    handle: JobHandle,
    done: Receiver<Result<ExecOutcome, rusty_sandbox::Error>>,
    outcome: Option<Result<ExecOutcome, rusty_sandbox::Error>>,
}

impl Running {
    /// Kill the bot's whole process group. Its outcome then reports the
    /// signal; see [`Running::outcome`].
    ///
    /// # Errors
    /// [`Error::Sandbox`] if the kill itself fails.
    pub fn stop(&self) -> Result<(), Error> {
        Ok(self.handle.kill()?)
    }

    /// How the bot ended, once it has; `None` while it runs.
    pub fn outcome(&mut self) -> Option<&Result<ExecOutcome, rusty_sandbox::Error>> {
        if self.outcome.is_none() {
            match self.done.try_recv() {
                Ok(outcome) => self.outcome = Some(outcome),
                Err(TryRecvError::Empty) => {}
                Err(TryRecvError::Disconnected) => {
                    self.outcome = Some(Err(rusty_sandbox::Error::Sandbox(
                        "the bot's waiting thread is gone".into(),
                    )));
                }
            }
        }
        self.outcome.as_ref()
    }

    /// Block until the bot ends.
    pub fn wait(mut self) -> Result<ExecOutcome, rusty_sandbox::Error> {
        if let Some(outcome) = self.outcome.take() {
            return outcome;
        }
        self.done.recv().unwrap_or_else(|_| {
            Err(rusty_sandbox::Error::Sandbox(
                "the bot's waiting thread is gone".into(),
            ))
        })
    }
}

/// Start `spec` under `executor`: the agent process in its sandbox, with
/// the network open, and a thread waiting on it.
///
/// # Errors
/// [`Error::Sandbox`] when the spec is refused or the sandbox cannot be set
/// up; nothing runs then.
pub fn start(executor: &ProcessExecutor, spec: &BotSpec) -> Result<Running, Error> {
    let sandbox = spec.sandbox()?;
    let program = spec.program.display().to_string();
    let job = executor.start(
        &sandbox,
        &program,
        &spec.args,
        Stdio::null(),
        Sockets::Internet,
    )?;
    let handle = job.handle();
    let wall = sandbox.limits().wall();
    let (sender, done) = mpsc::channel();
    std::thread::spawn(move || {
        // The receiver may be gone; there is nobody left to tell.
        let _ = sender.send(job.wait(wall));
    });
    Ok(Running {
        name: spec.name.clone(),
        handle,
        done,
        outcome: None,
    })
}

/// A set of running bots.
#[derive(Debug, Default)]
pub struct Fleet {
    /// Every bot started, in order.
    pub bots: Vec<Running>,
}

impl Fleet {
    /// Start every spec; the first failure stops the ones already started
    /// and is returned, so a fleet is all up or all down.
    ///
    /// # Errors
    /// As [`start`], naming the bot.
    pub fn start(executor: &ProcessExecutor, specs: &[BotSpec]) -> Result<Self, Error> {
        let mut fleet = Fleet::default();
        for spec in specs {
            match start(executor, spec) {
                Ok(running) => fleet.bots.push(running),
                Err(e) => {
                    fleet.stop();
                    return Err(Error::Config(format!("bot {:?}: {e}", spec.name)));
                }
            }
        }
        Ok(fleet)
    }

    /// Kill every bot; failures to kill are ignored (a bot already gone is
    /// the usual one).
    pub fn stop(&self) {
        for bot in &self.bots {
            let _ = bot.stop();
        }
    }

    /// Whether any bot is still running.
    pub fn any_running(&mut self) -> bool {
        self.bots.iter_mut().any(|b| b.outcome().is_none())
    }
}

/// The executor a `rusty-bot` fleet uses: this binary as the helper, with
/// status files in `state_dir` (which must be outside every workspace).
pub fn executor(helper: &Path, state_dir: &Path) -> ProcessExecutor {
    ProcessExecutor::new(
        helper.to_path_buf(),
        vec!["__sandbox".into()],
        state_dir.to_path_buf(),
    )
}

#[cfg(test)]
mod tests {
    use super::*;

    /// An absolute path on this platform: `/`-rooted on Unix, `C:\`-rooted
    /// on Windows, where a bare `/srv/...` is not absolute.
    fn abs(rel: &str) -> PathBuf {
        let root = if cfg!(windows) { "C:\\" } else { "/" };
        PathBuf::from(root).join(rel)
    }

    fn spec() -> BotSpec {
        BotSpec {
            name: "echo".into(),
            program: abs("opt/bots/echo/agent"),
            args: vec!["--port".into(), "4000".into()],
            env: vec![("PATH".into(), "/usr/bin".into())],
            workspace: abs("srv/bots/echo"),
            read_roots: vec![abs("usr"), abs("lib")],
            limits: default_limits(),
        }
    }

    #[test]
    fn the_sandbox_writes_only_to_the_workspace_and_reads_the_program() {
        let sandbox = spec().sandbox().expect("valid");
        assert_eq!(sandbox.write_roots(), [abs("srv/bots/echo")]);
        assert_eq!(sandbox.cwd(), &abs("srv/bots/echo"));
        assert!(sandbox.read_roots().contains(&abs("opt/bots/echo")));
        assert!(sandbox.can_reach(&abs("usr/bin/python3")));
        assert!(!sandbox.can_reach(&abs("srv/bots/other")));

        let mut inside = spec();
        inside.program = abs("usr/bin/python3");
        let sandbox = inside.sandbox().expect("valid");
        assert_eq!(
            sandbox.read_roots().len(),
            2,
            "a program under a root adds nothing"
        );

        let mut relative = spec();
        relative.workspace = "bots/echo".into();
        assert!(matches!(relative.sandbox(), Err(Error::Sandbox(_))));
    }

    #[test]
    fn bots_load_from_json_with_defaults() {
        let bots = load(
            r#"[{"name":"echo","program":"/opt/echo","workspace":"/srv/echo"},
                {"name":"tick","program":"/opt/tick","workspace":"/srv/tick","args":["--port","4001"],
                 "env":{"RUST_LOG":"info"},"read":["/usr"]}]"#,
        )
        .expect("loads");
        assert_eq!(bots.len(), 2);
        assert_eq!(bots[0].read_roots.len(), DEFAULT_READ_ROOTS.len());
        assert!(bots[0].env.is_empty() && bots[0].args.is_empty());
        assert_eq!(bots[1].args, vec!["--port", "4001"]);
        assert_eq!(
            bots[1].env,
            vec![("RUST_LOG".to_string(), "info".to_string())]
        );
        assert_eq!(bots[1].read_roots, vec![PathBuf::from("/usr")]);
        assert_eq!(bots[1].limits, default_limits());

        for (json, word) in [
            (r#"{}"#, "array"),
            (r#"[{"program":"/x","workspace":"/w"}]"#, "name"),
            (
                r#"[{"name":"a","program":"/x","workspace":"/w","env":["x"]}]"#,
                "object",
            ),
            (
                r#"[{"name":"a","program":"/x","workspace":"/w","args":[1]}]"#,
                "array of strings",
            ),
            (
                r#"[{"name":"a","program":"/x","workspace":"/w"},{"name":"a","program":"/y","workspace":"/v"}]"#,
                "duplicate",
            ),
        ] {
            let err = load(json).expect_err(json).to_string();
            assert!(err.contains(word), "{err} should mention {word}");
        }
    }
}
