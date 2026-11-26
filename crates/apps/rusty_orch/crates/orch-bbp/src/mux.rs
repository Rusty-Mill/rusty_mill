//! orch's sender to the mux (#1318 S4; #1571 step 3): find or make the
//! project's session and window with one-shot `rusty-mux-ctl` calls, then
//! drive one fresh `rusty-mux-ctl own` child per builder session.
//!
//! orch reaches the mux only through `rusty-mux-ctl` processes, never a
//! crate edge (AGENT-HOSTING.md, Gates). Each `own` is spawned with the
//! session's slot at fd 3 and the agent's ACP end at fd 4
//! (`LinuxSpawner::spawn_passing`), and launches the builder block with
//! `fds: 2`, which hands exactly those two on; `own` closes its copies once
//! the launch is answered. A second, display-only block in the same window
//! tails `acp.log`, outside the jail (D104 item 4, D187).
//!
//! Losing the mux is not a failed launch: a `lost` line, or `own` ending,
//! while orch waits is [`MuxError::Lost`], so the session ends `aborted`
//! rather than `failed` (`orch_lens::builder::recover`).

use std::collections::VecDeque;
use std::ffi::OsString;
use std::io::{BufRead, BufReader, Write};
use std::os::fd::{BorrowedFd, OwnedFd};
use std::path::PathBuf;
use std::sync::mpsc::{self, Receiver, RecvTimeoutError};
use std::time::Duration;

use platform::process::{Child, Command, EnvSpec, Signal, Stdio};
use platform_linux::{LinuxChild, LinuxFile, LinuxSpawner};
use rusty_json::{Map, Value};

/// How long orch waits for `own` to answer one request before treating
/// the mux as lost.
pub const ANSWER_WAIT: Duration = Duration::from_secs(30);

/// Why a mux call did not do what was asked.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum MuxError {
    /// The server refused the request; nothing was started.
    Refused(String),
    /// The connection, or `own`, is gone, or stopped answering. A launch in
    /// flight may have started its program: the session ends `aborted`.
    Lost(String),
    /// `rusty-mux-ctl` could not be run, or printed something unreadable.
    Ctl(String),
}

impl std::fmt::Display for MuxError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::Refused(e) => write!(f, "the mux refused: {e}"),
            Self::Lost(e) => write!(f, "the mux is lost: {e}"),
            Self::Ctl(e) => write!(f, "rusty-mux-ctl: {e}"),
        }
    }
}

impl std::error::Error for MuxError {}

/// How orch runs `rusty-mux-ctl`.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Ctl {
    /// The `rusty-mux-ctl` binary.
    pub bin: PathBuf,
    /// The server's socket; `None` uses the ctl's own default.
    pub socket: Option<PathBuf>,
}

impl Ctl {
    fn args(&self, rest: &[&str]) -> Vec<OsString> {
        let mut args: Vec<OsString> = Vec::new();
        if let Some(socket) = &self.socket {
            args.push("--socket".into());
            args.push(socket.into());
        }
        args.push("--json".into());
        args.extend(rest.iter().map(OsString::from));
        args
    }

    /// One one-shot call; its stdout lines.
    fn call(&self, rest: &[&str]) -> Result<Vec<String>, MuxError> {
        let out = std::process::Command::new(&self.bin)
            .args(self.args(rest))
            .stdin(std::process::Stdio::null())
            .output()
            .map_err(|e| MuxError::Ctl(format!("{}: {e}", self.bin.display())))?;
        let stdout = String::from_utf8_lossy(&out.stdout).into_owned();
        if !out.status.success() {
            let why = String::from_utf8_lossy(&out.stderr).trim().to_owned();
            return Err(MuxError::Ctl(format!("{} failed: {why}", rest.join(" "))));
        }
        Ok(stdout.lines().map(str::to_owned).collect())
    }

    /// The window named `window` in the first session named `session`,
    /// made if either is missing. One session per project is orch's
    /// convention; the mux doesn't enforce unique names (#1571 §2).
    ///
    /// # Errors
    /// When a call fails or prints no id.
    pub fn window(&self, session: &str, window: &str) -> Result<String, MuxError> {
        let listed = self.call(&["list"])?;
        let session_id = match find(&listed, session, window)? {
            Found::Window(id) => return Ok(id),
            Found::Session(id) => id,
            Found::Nothing => created(&self.call(&["create-session", "--name", session])?)?,
        };
        created(&self.call(&["create-window", "--session", &session_id, "--name", window])?)
    }
}

/// What `list` holds of a session and window by name.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum Found {
    Window(String),
    Session(String),
    Nothing,
}

/// Look up `session` and its `window` in `list --json` lines.
///
/// # Errors
/// When a line is not a JSON object.
pub fn find(lines: &[String], session: &str, window: &str) -> Result<Found, MuxError> {
    let entries: Vec<Value> = lines
        .iter()
        .filter(|l| !l.trim().is_empty())
        .map(|l| Value::parse(l).map_err(|e| MuxError::Ctl(format!("list: {e}"))))
        .collect::<Result<_, _>>()?;
    let text = |v: &Value, k: &str| v.get(k).and_then(Value::as_str).map(str::to_owned);
    let kind = |v: &Value| text(v, "kind");
    let Some(session_id) = entries
        .iter()
        .find(|e| {
            kind(e).as_deref() == Some("session") && text(e, "name").as_deref() == Some(session)
        })
        .and_then(|e| text(e, "session"))
    else {
        return Ok(Found::Nothing);
    };
    let window_id = entries
        .iter()
        .find(|e| {
            kind(e).as_deref() == Some("window")
                && text(e, "session").as_deref() == Some(session_id.as_str())
                && text(e, "name").as_deref() == Some(window)
        })
        .and_then(|e| text(e, "window"));
    Ok(window_id.map_or(Found::Session(session_id), Found::Window))
}

/// The id a `create-*` call printed.
fn created(lines: &[String]) -> Result<String, MuxError> {
    lines
        .iter()
        .filter_map(|l| Value::parse(l).ok())
        .find_map(|v| v.get("created").and_then(Value::as_str).map(str::to_owned))
        .ok_or_else(|| MuxError::Ctl(format!("no id created: {}", lines.join(" "))))
}

/// One block launch, as `own` takes it (C75–C78, D336).
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Launch {
    pub window: String,
    pub program: String,
    pub args: Vec<String>,
    pub cwd: Option<String>,
    /// The program's exact environment; `None` lets the server choose.
    pub env: Option<Vec<(String, String)>>,
    /// How many of `own`'s descriptors 3, 4, … the program gets.
    pub fds: u8,
    pub hold: bool,
    pub owner_only: bool,
    /// `agent`, `role` and `card` (D336), all or none.
    pub meta: Option<[String; 3]>,
}

impl Launch {
    /// The request line's fields, without `id` and `op`.
    pub fn fields(&self) -> Map {
        let mut m = Map::new();
        m.insert("window".into(), Value::from(self.window.as_str()));
        m.insert("program".into(), Value::from(self.program.as_str()));
        let args: Vec<Value> = self.args.iter().map(|a| Value::from(a.as_str())).collect();
        m.insert("args".into(), Value::Array(args));
        if let Some(cwd) = &self.cwd {
            m.insert("cwd".into(), Value::from(cwd.as_str()));
        }
        if let Some(env) = &self.env {
            let env: Map = env
                .iter()
                .map(|(k, v)| (k.clone(), Value::from(v.as_str())))
                .collect();
            m.insert("env".into(), Value::Object(env));
        }
        if self.fds > 0 {
            m.insert("fds".into(), Value::from(self.fds));
        }
        m.insert("hold".into(), Value::Bool(self.hold));
        let input = if self.owner_only {
            "owner-only"
        } else {
            "open"
        };
        m.insert("input".into(), Value::from(input));
        if let Some([agent, role, card]) = &self.meta {
            m.insert("agent".into(), Value::from(agent.as_str()));
            m.insert("role".into(), Value::from(role.as_str()));
            m.insert("card".into(), Value::from(card.as_str()));
        }
        m
    }
}

/// A line `own` printed.
#[derive(Clone, Debug, PartialEq)]
pub enum Line {
    /// The answer to request `id`: `Ok` with the answer's fields, or the
    /// server's reason.
    Answer {
        id: u64,
        outcome: Result<Value, String>,
    },
    /// An owner event: `program-exited`, `block-closed`, `reconnected`,
    /// `lost`, ….
    Event { kind: String, body: Value },
}

/// Parse one `own` line.
///
/// # Errors
/// When it is neither an answer nor an event.
pub fn parse_line(line: &str) -> Result<Line, MuxError> {
    let v = Value::parse(line.trim()).map_err(|e| MuxError::Ctl(format!("own: {e}")))?;
    if let Some(kind) = v.get("event").and_then(Value::as_str) {
        return Ok(Line::Event {
            kind: kind.to_owned(),
            body: v.clone(),
        });
    }
    let id = v
        .get("id")
        .and_then(Value::as_u64)
        .ok_or_else(|| MuxError::Ctl(format!("own: not an answer: {line}")))?;
    let outcome = match v.get("ok").and_then(Value::as_bool) {
        Some(true) => Ok(v.clone()),
        _ => Err(v
            .get("error")
            .and_then(Value::as_str)
            .unwrap_or("refused")
            .to_owned()),
    };
    Ok(Line::Answer { id, outcome })
}

/// One live `rusty-mux-ctl own` child and the owner connection it holds.
pub struct Own {
    child: LinuxChild,
    to_own: Option<std::io::PipeWriter>,
    from_own: Receiver<Result<Line, MuxError>>,
    events: VecDeque<Line>,
    next_id: u64,
    wait: Duration,
}

impl Own {
    /// Spawn `own` with `passed` at fds 3, 4, … (nothing else above 2).
    ///
    /// # Errors
    /// When the pipes or the child cannot be made.
    pub fn spawn(ctl: &Ctl, passed: &[BorrowedFd<'_>]) -> Result<Own, MuxError> {
        let spawn_err = |e: &dyn std::fmt::Display| MuxError::Ctl(format!("spawn own: {e}"));
        let (child_in, to_own) = std::io::pipe().map_err(|e| spawn_err(&e))?;
        let (from_child, child_out) = std::io::pipe().map_err(|e| spawn_err(&e))?;
        let stdio = |fd: OwnedFd| Stdio::File(Box::new(LinuxFile::from(fd)));
        let mut cmd = Command::new(ctl.bin.as_os_str(), ".").args(ctl.args(&["own"]));
        cmd.cwd = std::env::current_dir()
            .map_err(|e| spawn_err(&e))?
            .into_os_string();
        cmd.env = EnvSpec::Inherit;
        cmd.stdin = stdio(OwnedFd::from(child_in));
        cmd.stdout = stdio(OwnedFd::from(child_out));
        let child = LinuxSpawner
            .spawn_passing(&cmd, passed)
            .map_err(|e| spawn_err(&e))?;
        // The child's pipe ends close here, so EOF reaches each side.
        drop(cmd);
        let (tx, from_own) = mpsc::channel();
        std::thread::spawn(move || {
            for line in BufReader::new(from_child).lines() {
                let parsed = match line {
                    Ok(l) if l.trim().is_empty() => continue,
                    Ok(l) => parse_line(&l),
                    Err(e) => Err(MuxError::Lost(format!("own: {e}"))),
                };
                if tx.send(parsed).is_err() {
                    return;
                }
            }
        });
        Ok(Own {
            child,
            to_own: Some(to_own),
            from_own,
            events: VecDeque::new(),
            next_id: 1,
            wait: ANSWER_WAIT,
        })
    }

    /// Wait at most `wait` for each answer (tests shorten it).
    pub fn answer_wait(mut self, wait: Duration) -> Self {
        self.wait = wait;
        self
    }

    /// Launch a block; its id.
    ///
    /// # Errors
    /// [`MuxError::Refused`] when the server refused it, [`MuxError::Lost`]
    /// when the mux went away first.
    pub fn launch(&mut self, launch: &Launch) -> Result<String, MuxError> {
        let answer = self.request("launch", launch.fields())?;
        answer
            .get("block")
            .and_then(Value::as_str)
            .map(str::to_owned)
            .ok_or_else(|| MuxError::Ctl("launch answered no block".into()))
    }

    /// Signal a block's program group: `term`, `kill`, ….
    ///
    /// # Errors
    /// As [`Own::launch`].
    pub fn signal(&mut self, block: &str, signal: &str) -> Result<(), MuxError> {
        let mut m = Map::new();
        m.insert("block".into(), Value::from(block));
        m.insert("signal".into(), Value::from(signal));
        self.request("signal", m).map(|_| ())
    }

    /// Close a block's pty and wait for its program, never killing it.
    ///
    /// # Errors
    /// As [`Own::launch`].
    pub fn release(&mut self, block: &str) -> Result<(), MuxError> {
        self.block_op("release", block)
    }

    /// Close a block.
    ///
    /// # Errors
    /// As [`Own::launch`].
    pub fn close(&mut self, block: &str) -> Result<(), MuxError> {
        self.block_op("close", block)
    }

    /// The next owner event, waiting at most `timeout`; `None` if none came.
    ///
    /// # Errors
    /// [`MuxError::Lost`] on `lost` or when `own` has ended.
    pub fn next_event(&mut self, timeout: Duration) -> Result<Option<Line>, MuxError> {
        if let Some(line) = self.events.pop_front() {
            return Ok(Some(line));
        }
        match self.from_own.recv_timeout(timeout) {
            Ok(line) => lost_if(line?).map(Some),
            Err(RecvTimeoutError::Timeout) => Ok(None),
            Err(RecvTimeoutError::Disconnected) => Err(MuxError::Lost("own ended".into())),
        }
    }

    /// End the owner connection: close `own`'s stdin and reap it. Blocks
    /// the server has not been told to close orphan after its grace.
    pub fn finish(mut self) {
        drop(self.to_own.take());
        let deadline = std::time::Instant::now() + self.wait;
        while std::time::Instant::now() < deadline {
            if matches!(self.child.try_wait(), Ok(Some(_))) {
                return;
            }
            std::thread::sleep(Duration::from_millis(20));
        }
        let _ = self.child.kill_single(Signal::Kill);
        let _ = Box::new(self.child).wait();
    }

    fn block_op(&mut self, op: &str, block: &str) -> Result<(), MuxError> {
        let mut m = Map::new();
        m.insert("block".into(), Value::from(block));
        self.request(op, m).map(|_| ())
    }

    fn request(&mut self, op: &str, mut fields: Map) -> Result<Value, MuxError> {
        let id = self.next_id;
        self.next_id += 1;
        fields.insert("id".into(), Value::from(id));
        fields.insert("op".into(), Value::from(op));
        let line = Value::Object(fields).to_json_string();
        let to_own = self
            .to_own
            .as_mut()
            .ok_or_else(|| MuxError::Lost("own already finished".into()))?;
        writeln!(to_own, "{line}")
            .and_then(|()| to_own.flush())
            .map_err(|e| MuxError::Lost(format!("own: {e}")))?;
        loop {
            let line = match self.from_own.recv_timeout(self.wait) {
                Ok(line) => lost_if(line?)?,
                Err(RecvTimeoutError::Timeout) => {
                    return Err(MuxError::Lost(format!(
                        "no answer to {op} in {:?}",
                        self.wait
                    )));
                }
                Err(RecvTimeoutError::Disconnected) => {
                    return Err(MuxError::Lost(format!("own ended during {op}")));
                }
            };
            match line {
                Line::Answer { id: got, outcome } if got == id => {
                    return outcome.map_err(MuxError::Refused);
                }
                Line::Answer { .. } => {}
                event @ Line::Event { .. } => self.events.push_back(event),
            }
        }
    }
}

/// `lost` as an error; any other line as itself.
fn lost_if(line: Line) -> Result<Line, MuxError> {
    match &line {
        Line::Event { kind, .. } if kind == "lost" => {
            Err(MuxError::Lost("the owner connection is gone".into()))
        }
        _ => Ok(line),
    }
}

/// The display-only block that tails `acp.log` beside the builder, outside
/// the jail and held once it ends (D104 item 4, D187).
pub fn viewer(window: &str, acp_log: &str) -> Launch {
    Launch {
        window: window.to_owned(),
        program: "tail".into(),
        args: vec!["-F".into(), acp_log.to_owned()],
        cwd: None,
        env: None,
        fds: 0,
        hold: true,
        owner_only: true,
        meta: None,
    }
}

/// An exact environment carrying only `PATH`, `HOME`, `LANG` and `TERM`
/// from `from`, so nothing else of orch's, and no `RUSTY_MUX_*`, reaches
/// the program (#1571 §2, D107).
pub fn exact_env(from: impl Fn(&str) -> Option<String>) -> Vec<(String, String)> {
    ["PATH", "HOME", "LANG", "TERM"]
        .into_iter()
        .filter_map(|k| from(k).map(|v| (k.to_owned(), v)))
        .collect()
}

/// The request line `own` gets for `launch` as request `id`, for tests and
/// logs.
pub fn launch_line(id: u64, launch: &Launch) -> String {
    let mut m = launch.fields();
    m.insert("id".into(), Value::from(id));
    m.insert("op".into(), Value::from("launch"));
    Value::Object(m).to_json_string()
}
