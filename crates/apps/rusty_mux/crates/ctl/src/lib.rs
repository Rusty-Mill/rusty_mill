//! `rusty-mux-ctl`: a scriptable rusty_mux client.
//!
//! Every request in [`rusty_mux_proto::schema::CATALOG`] is a subcommand and
//! each of its parameters a `--flag`, so the CLI grows with the protocol.
//! Requests joined by `+` run in order on one connection (attach, take the
//! lease, then send input), stopping at the first failure. `events` prints
//! the events after a cursor and the cursor to resume from; with `--follow`
//! it keeps printing until the server goes away.
//!
//! An id flag also takes a name, a position or an id suffix (see
//! [`target`]), and a missing one defaults to the block, window or session
//! the command runs in, from `$RUSTY_MUX_BLOCK` and the like.
//!
//! `do ACTION --ARG VALUE` runs an action from the config file, a named
//! chain of requests (see [`rusty_mux_script`]); `config check` and
//! `config list` check and list it. `watch` follows events and runs the
//! config's `on` rules for each.
//!
//! `agent setup|status|remove` installs, checks and removes a skill and a
//! session-start hook for Claude Code.
//!
//! `bridge` joins stdin and stdout to the socket, byte for byte, so a client
//! on another machine reaches the server with `ssh host rusty-mux-ctl
//! bridge`.
//!
//! `wait --block B` blocks until the block's program exits and exits with
//! its status; `wait --event KIND --FIELD VALUE` until a matching event.
//! `run -- COMMAND ARG...` starts a program in a new block, in ctl's
//! directory, and with `--wait` waits for it the same way. Both give up
//! after `--timeout SECS` with exit code 124.
//!
//! Output is text, or one JSON object per line with `--json`. The exit code
//! is an [`Exit`]; an interrupt ends the process with the signal, which a
//! shell reports as 130. An embedder bounds a command with [`run_until`].

// Off Unix and Windows the connection code compiles but `execute` never
// reaches it.
#![cfg_attr(not(any(unix, windows)), allow(dead_code))]

mod agent;
mod api;
#[cfg(any(unix, windows))]
mod bridge;
pub mod cli;
mod config;
mod conn;
mod render;
mod stop;
pub mod target;
mod wait;
mod watch;

use std::io::{self, Write};
use std::num::NonZeroU8;
use std::path::{Path, PathBuf};
use std::time::Instant;

use rusty_mux_core::Event;
use rusty_mux_proto::schema::{CATALOG, Returns, Value};
use rusty_mux_proto::{Code, Cursor, Entry, Features, Message};
use rusty_uuid::Uuid;

pub use crate::api::input_schema;
use crate::cli::{Api, Arg, Call, Command, ConfigCommand, Defaults, Invocation};
use crate::conn::Conn;
pub use crate::stop::Stop;
use crate::target::Miss;

/// The process exit code; see [`Exit::code`].
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Exit {
    Ok,
    /// A request was refused, a stream could not be resumed, the server
    /// sent something this client cannot follow, or a waited-for block
    /// closed before its program exited.
    Failed,
    /// The command line is wrong.
    Usage,
    /// A target names nothing, or more than one thing.
    Target,
    /// No server at the socket, or the connection broke.
    Unreachable,
    /// `wait` or `run --wait` gave up at its `--timeout`.
    TimedOut,
    /// The status of the program `wait` or `run --wait` waited for (see
    /// [`Exit::of_program`]), which may coincide with one of the codes
    /// above.
    Program(NonZeroU8),
}

impl Exit {
    /// 0 to 4 in the order above, 124 for a timeout as `timeout(1)` exits,
    /// or the program's own status.
    pub const fn code(self) -> u8 {
        match self {
            Self::Ok => 0,
            Self::Failed => 1,
            Self::Usage => 2,
            Self::Target => 3,
            Self::Unreachable => 4,
            Self::TimedOut => 124,
            Self::Program(code) => code.get(),
        }
    }
}

/// Where the command runs: the environment a block's program gets.
#[derive(Debug, Clone, Default)]
pub struct Env {
    /// `$RUSTY_MUX_SOCKET`.
    pub socket: Option<PathBuf>,
    /// `$RUSTY_MUX_SESSION`, `$RUSTY_MUX_WINDOW`, `$RUSTY_MUX_BLOCK` and
    /// the current directory.
    pub here: Defaults,
    /// The config file `do` and `config` read: `$RUSTY_MUX_CONFIG`, else
    /// `rusty_mux/config.json` in the config directory.
    pub config: Option<PathBuf>,
    /// The agent config directory `agent` works in: `$CLAUDE_CONFIG_DIR`,
    /// else `.claude` in the home directory.
    pub agent_dir: Option<PathBuf>,
    /// This program, which the agent's session-start hook runs.
    pub ctl: Option<PathBuf>,
}

impl Env {
    /// This process's environment. A malformed id is ignored, with a warning
    /// on stderr.
    pub fn from_process() -> Self {
        Self {
            socket: std::env::var_os("RUSTY_MUX_SOCKET").map(Into::into),
            here: Defaults {
                session: id_var("RUSTY_MUX_SESSION"),
                window: id_var("RUSTY_MUX_WINDOW"),
                block: id_var("RUSTY_MUX_BLOCK"),
                dir: std::env::current_dir().ok(),
            },
            config: config::default_path(),
            agent_dir: agent_dir(),
            ctl: std::env::current_exe().ok(),
        }
    }
}

fn agent_dir() -> Option<PathBuf> {
    if let Some(dir) = std::env::var_os("CLAUDE_CONFIG_DIR") {
        return Some(dir.into());
    }
    let home = std::env::var_os("HOME").or_else(|| std::env::var_os("USERPROFILE"))?;
    Some(Path::new(&home).join(".claude"))
}

fn id_var(name: &str) -> Option<Uuid> {
    let value = std::env::var(name).ok()?;
    match value.parse() {
        Ok(id) => Some(id),
        Err(_) => {
            eprintln!("rusty-mux: ignoring ${name}: `{value}` is not an id");
            None
        }
    }
}

/// What a request answered with.
enum Reply {
    /// Done, with its new id if any, after the entries of a listing.
    Done {
        created: Option<Uuid>,
        entries: Vec<Entry>,
    },
    Text(String),
}

/// Why a command stopped early.
enum Failure {
    Failed(String),
    /// A wait ran past its `--timeout`.
    TimedOut,
    /// A target names nothing, or more than one thing.
    Missing(String),
    Unreachable(io::Error),
}

impl From<io::Error> for Failure {
    fn from(err: io::Error) -> Self {
        match err.kind() {
            io::ErrorKind::InvalidData => Self::Failed(err.to_string()),
            _ => Self::Unreachable(err),
        }
    }
}

impl From<Miss> for Failure {
    fn from(miss: Miss) -> Self {
        Self::Missing(match miss {
            Miss::NotFound(why) => why,
            Miss::Ambiguous(what, ids) => {
                let ids: Vec<String> = ids.iter().map(Uuid::to_string).collect();
                format!("{what} is ambiguous: {}", ids.join(", "))
            }
        })
    }
}

/// Why [`run`] refuses `bridge`.
const BRIDGE_ONLY: &str =
    "bridge runs only as the rusty-mux-ctl program: it takes over the process's stdin and stdout";

/// Runs a command line (without the program name) in `env`; output goes to
/// `out`, diagnostics to `err`. `bridge` is refused: see [`run_process`].
pub fn run(args: &[String], env: &Env, out: &mut dyn Write, err: &mut dyn Write) -> Exit {
    run_until(args, env, &Stop::default(), out, err)
}

/// [`run`] as the `rusty-mux-ctl` program, with this process's stdout and
/// stderr; it alone runs `bridge`, which joins this process's stdin and
/// stdout to the server's socket until the server closes it, and is
/// bounded by nothing but that.
pub fn run_process(args: &[String], env: &Env) -> Exit {
    let mut err = io::stderr();
    let bridged = match cli::parse(args, &env.here) {
        Ok(invocation) if matches!(invocation.command, Command::Bridge) => invocation,
        _ => return run(args, env, &mut io::stdout(), &mut err),
    };
    match socket(&bridged, env) {
        Ok(socket) => finish(bridge(&socket), &socket, &mut err),
        Err(reason) => report(&mut err, Exit::Usage, reason),
    }
}

/// The socket `invocation` names, else `$RUSTY_MUX_SOCKET`.
fn socket(invocation: &Invocation, env: &Env) -> Result<PathBuf, &'static str> {
    invocation
        .socket
        .clone()
        .or_else(|| env.socket.clone())
        .ok_or("no socket: pass --socket or set RUSTY_MUX_SOCKET")
}

/// [`run`], giving up on the server once `stop` says so (past its deadline,
/// or cancelled) with [`Exit::Unreachable`]. Requests already sent stand.
///
/// `stop` bounds reads everywhere. It bounds connecting only on Linux, and
/// writes only on Unix: on Windows a connect or a write blocked on a server
/// that stopped reading runs past the deadline.
pub fn run_until(
    args: &[String],
    env: &Env,
    stop: &Stop,
    out: &mut dyn Write,
    err: &mut dyn Write,
) -> Exit {
    let mut invocation = match cli::parse(args, &env.here) {
        Ok(invocation) => invocation,
        Err(reason) => return report(err, Exit::Usage, &reason),
    };
    match invocation.command {
        Command::Help(method) => return write_line(out, cli::usage(method).trim_end()),
        Command::Api(Api::List) => return write_line(out, &api::list(invocation.json)),
        Command::Api(Api::Describe(method)) => return write_line(out, &api::describe(method)),
        Command::Config(command) => return run_config(&command, env, invocation.json, out, err),
        Command::Agent(command) => {
            return agent::run(&command, env, invocation.json, out)
                .unwrap_or_else(|reason| report(err, Exit::Failed, &reason));
        }
        Command::Do { action, args } => match action_calls(&action, &args, env) {
            Ok(calls) => invocation.command = Command::Calls(calls),
            Err((exit, reason)) => return report(err, exit, &reason),
        },
        // Unbounded, and on the process's own stdin and stdout, so never
        // through `stop` and `out`.
        Command::Bridge => return report(err, Exit::Usage, BRIDGE_ONLY),
        Command::Calls(_)
        | Command::Events { .. }
        | Command::Watch { .. }
        | Command::Wait(_)
        | Command::Run(_) => {}
    }
    let socket = match socket(&invocation, env) {
        Ok(socket) => socket,
        Err(reason) => return report(err, Exit::Usage, reason),
    };
    if let Command::Watch { after } = invocation.command {
        return run_watch(after, &socket, env, invocation.json, stop, out, err);
    }
    let deadline = timeout(&invocation.command).and_then(|t| Instant::now().checked_add(t));
    let stop = deadline.map_or_else(|| stop.clone(), |d| stop.sooner(d));
    let result = execute(&invocation, &socket, env.here.session, &stop, out);
    finish(timed_out(result, deadline), &socket, err)
}

/// How long a `wait` or `run --wait` may wait.
fn timeout(command: &Command) -> Option<std::time::Duration> {
    match command {
        Command::Wait(wait) => wait.timeout,
        Command::Run(run) => run.wait.flatten(),
        _ => None,
    }
}

/// `result`, with running out of time at `deadline` as a timeout rather
/// than an unreachable server.
fn timed_out(result: Result<Exit, Failure>, deadline: Option<Instant>) -> Result<Exit, Failure> {
    match result {
        Err(Failure::Unreachable(e))
            if e.kind() == io::ErrorKind::TimedOut
                && deadline.is_some_and(|d| Instant::now() >= d) =>
        {
            Err(Failure::TimedOut)
        }
        other => other,
    }
}

/// `watch`: follows events and runs the config's rules for each, until
/// the server goes away.
fn run_watch(
    after: Option<Cursor>,
    socket: &Path,
    env: &Env,
    json: bool,
    stop: &Stop,
    out: &mut dyn Write,
    err: &mut dyn Write,
) -> Exit {
    let config = match config::load(env.config.as_deref()) {
        Ok(config) => config,
        Err(problems) => return report(err, Exit::Failed, &problems),
    };
    if config.rules.is_empty() {
        return report(err, Exit::Failed, "no `on` rules in the config");
    }
    let env = Env {
        socket: Some(socket.to_path_buf()),
        ..env.clone()
    };
    let watcher = watch::Watcher {
        config: &config,
        env: &env,
        json,
        stop,
    };
    let needs: Vec<(&str, Features)> = config
        .rules
        .iter()
        .map(|rule| (rule.event, watch::needs(rule.event)))
        .collect();
    let how = Follow {
        after,
        needs: &needs,
        json,
        here: env.here.session,
    };
    let result = follow(socket, &how, stop, out, &mut |seq, event, out| {
        watcher
            .event(seq, event, out, &mut *err)
            .map_err(Failure::from)
    });
    finish(result.map(|()| Exit::Ok), socket, err)
}

/// The exit code for a command's `result`, its reason reported on `err`.
fn finish(result: Result<Exit, Failure>, socket: &Path, err: &mut dyn Write) -> Exit {
    match result {
        Ok(exit) => exit,
        Err(Failure::Failed(reason)) => report(err, Exit::Failed, &reason),
        Err(Failure::TimedOut) => report(err, Exit::TimedOut, "timed out"),
        Err(Failure::Missing(reason)) => report(err, Exit::Target, &reason),
        Err(Failure::Unreachable(e)) => report(
            err,
            Exit::Unreachable,
            &format!("{}: {e}", socket.display()),
        ),
    }
}

/// The requests `do ACTION` sends: the action expanded with `args`, parsed
/// like a typed chain so ids resolve and default the same way.
fn action_calls(
    action: &str,
    args: &[(String, String)],
    env: &Env,
) -> Result<Vec<Call>, (Exit, String)> {
    let config = config::load(env.config.as_deref()).map_err(|e| (Exit::Failed, e))?;
    let action = config
        .action(action)
        .ok_or_else(|| (Exit::Usage, format!("no action `{action}` in the config")))?;
    let calls = action.expand(args).map_err(|e| (Exit::Usage, e))?;
    let chain = config::chain(&calls);
    match cli::parse(&chain, &env.here) {
        Ok(Invocation {
            command: Command::Calls(calls),
            ..
        }) => Ok(calls),
        Ok(_) => Err((
            Exit::Failed,
            format!("{}: not a chain of requests", action.name),
        )),
        Err(reason) => Err((Exit::Usage, format!("{}: {reason}", action.name))),
    }
}

fn run_config(
    command: &ConfigCommand,
    env: &Env,
    json: bool,
    out: &mut dyn Write,
    err: &mut dyn Write,
) -> Exit {
    let path = match command {
        ConfigCommand::Check(Some(path)) => Some(path.as_path()),
        ConfigCommand::Check(None) | ConfigCommand::List => env.config.as_deref(),
    };
    let config = match config::load(path) {
        Ok(config) => config,
        Err(problems) => return report(err, Exit::Failed, &problems),
    };
    match command {
        ConfigCommand::Check(_) => write_line(out, &config::summary(&config)),
        ConfigCommand::List => write_line(out, &config::list(&config, json)),
    }
}

fn report(err: &mut dyn Write, exit: Exit, reason: &str) -> Exit {
    // Nothing better to do if stderr itself is gone.
    let _ = writeln!(err, "rusty-mux-ctl: {reason}");
    exit
}

fn write_line(out: &mut dyn Write, line: &str) -> Exit {
    match writeln!(out, "{line}") {
        Ok(()) => Exit::Ok,
        Err(_) => Exit::Failed,
    }
}

#[cfg(any(unix, windows))]
fn execute(
    invocation: &Invocation,
    socket: &Path,
    here: Option<Uuid>,
    stop: &Stop,
    out: &mut dyn Write,
) -> Result<Exit, Failure> {
    let mut conn = Conn::new(conn::connect(socket, stop)?, stop.clone());
    let mut session = Session {
        conn: &mut conn,
        out,
        json: invocation.json,
        here,
    };
    let json = invocation.json;
    match &invocation.command {
        Command::Calls(calls) => session.calls(calls).map(|()| Exit::Ok),
        Command::Events { after, follow } => session
            .events(*after, *follow, &[], &mut |seq, event, out| {
                writeln!(out, "{}", render::event_line(seq, event, json)).map_err(Failure::from)
            })
            .map(|()| Exit::Ok),
        Command::Wait(wait) => session.wait(&wait.until),
        Command::Run(run) => session.run(run),
        Command::Help(_)
        | Command::Api(_)
        | Command::Do { .. }
        | Command::Config(_)
        | Command::Agent(_)
        | Command::Watch { .. }
        | Command::Bridge => Ok(Exit::Ok),
    }
}

/// `bridge`, for [`run_process`] only: until the server closes the
/// connection.
#[cfg(any(unix, windows))]
fn bridge(socket: &Path) -> Result<Exit, Failure> {
    bridge::run(socket)?;
    Ok(Exit::Ok)
}

#[cfg(not(any(unix, windows)))]
fn bridge(_: &Path) -> Result<Exit, Failure> {
    Err(unsupported())
}

/// What [`follow`] follows, and how it prints.
struct Follow<'a> {
    after: Option<Cursor>,
    /// Event kinds the follower needs, each with the feature it takes.
    needs: &'a [(&'a str, Features)],
    json: bool,
    here: Option<Uuid>,
}

/// Follows the events after `how.after` on a connection of its own,
/// passing each to `on_event`; fails first unless the server sends every
/// event kind `how.needs`.
#[cfg(any(unix, windows))]
fn follow(
    socket: &Path,
    how: &Follow<'_>,
    stop: &Stop,
    out: &mut dyn Write,
    on_event: &mut OnEvent<'_>,
) -> Result<(), Failure> {
    let mut conn = Conn::new(conn::connect(socket, stop)?, stop.clone());
    let mut session = Session {
        conn: &mut conn,
        out,
        json: how.json,
        here: how.here,
    };
    session.events(how.after, true, how.needs, on_event)
}

#[cfg(not(any(unix, windows)))]
fn follow(
    _: &Path,
    _: &Follow<'_>,
    _: &Stop,
    _: &mut dyn Write,
    _: &mut OnEvent<'_>,
) -> Result<(), Failure> {
    Err(unsupported())
}

/// What happens to each numbered event that is followed.
type OnEvent<'a> = dyn FnMut(u64, &Event, &mut dyn Write) -> Result<(), Failure> + 'a;

#[cfg(not(any(unix, windows)))]
fn execute(
    _: &Invocation,
    _: &Path,
    _: Option<Uuid>,
    _: &Stop,
    _: &mut dyn Write,
) -> Result<Exit, Failure> {
    Err(unsupported())
}

#[cfg(not(any(unix, windows)))]
fn unsupported() -> Failure {
    Failure::Unreachable(io::Error::new(
        io::ErrorKind::Unsupported,
        "rusty-mux-ctl connects over a Unix socket (Unix or Windows)",
    ))
}

struct Session<'a, S> {
    conn: &'a mut Conn<S>,
    out: &'a mut dyn Write,
    json: bool,
    /// The session targets are looked up in when they name none.
    here: Option<Uuid>,
}

impl<S: io::Read + Write> Session<'_, S> {
    fn line(&mut self, line: &str) -> Result<(), Failure> {
        writeln!(self.out, "{line}").map_err(|e| Failure::Failed(format!("writing output: {e}")))
    }

    /// Sends Hello and returns the Welcome's features and cursor.
    fn hello(
        &mut self,
        features: Features,
        resume: Option<Cursor>,
    ) -> Result<(Features, Option<Cursor>), Failure> {
        self.conn.send(&Message::Hello {
            features,
            resume,
            takeover: None,
        })?;
        match self.conn.recv()? {
            Message::Welcome {
                features, cursor, ..
            } => Ok((features, cursor)),
            other => Err(Failure::Failed(format!("expected Welcome, got {other:?}"))),
        }
    }

    /// Sends a Sync numbered `req`, which the server needs to support.
    fn sync(&mut self, features: Features, req: u64) -> Result<(), Failure> {
        require_sync(features)?;
        Ok(self.conn.send(&Message::Sync { req })?)
    }

    fn calls(&mut self, calls: &[Call]) -> Result<(), Failure> {
        // Everything any method needs, and sync to confirm unanswered ones.
        // The screen an attach sends is ignored.
        let wanted = CATALOG.iter().fold(Features::SYNC, |all, m| {
            all.union(m.needs_for(m.params.iter().map(|p| p.name)))
        });
        let (features, _) = self.hello(wanted, None)?;
        // Refuse a chain the server cannot run before any of it runs.
        for call in calls {
            require(features, call)?;
        }
        let mut reqs = 1..;
        for call in calls {
            // Listed afresh for each call that names a target, so it can
            // name what an earlier call in the chain created or renamed.
            let entries = match call.has_target() {
                true => self.list(features, reqs.next().unwrap_or(u64::MAX))?,
                false => Vec::new(),
            };
            let args = self.resolve(call, &entries)?;
            let req = reqs.next().unwrap_or(u64::MAX);
            let message = call
                .method
                .request(req, &args)
                .map_err(|e| Failure::Failed(e.to_string()))?;
            self.conn.send(&message)?;
            let name = call.method.name;
            let reply = if call.method.returns == Returns::Unanswered {
                // Learn the outcome before anything else is sent, so a
                // refusal stops the chain. Sync's reply follows any Failed.
                let barrier = reqs.next().unwrap_or(u64::MAX);
                self.sync(features, barrier)?;
                self.reply(barrier, req, name)?
            } else {
                self.reply(req, req, name)?
            };
            self.print(name, reply)?;
        }
        Ok(())
    }

    /// Every entity, for resolving targets.
    fn list(&mut self, features: Features, req: u64) -> Result<Vec<Entry>, Failure> {
        if !features.contains(Features::INSPECT) {
            return Err(Failure::Failed(
                "the server cannot list, needed to resolve a target; pass full ids".into(),
            ));
        }
        self.conn.send(&Message::List { req })?;
        match self.reply(req, req, "list")? {
            Reply::Done { entries, .. } => Ok(entries),
            Reply::Text(_) => Err(Failure::Failed("list answered with text".into())),
        }
    }

    /// `call`'s arguments with each target replaced by the id it names.
    fn resolve(
        &self,
        call: &Call,
        entries: &[Entry],
    ) -> Result<Vec<(&'static str, Value)>, Failure> {
        call.args
            .iter()
            .map(|(name, arg)| match arg {
                Arg::Value(value) => Ok((*name, value.clone())),
                Arg::Target { entity, spec } => {
                    let id = target::resolve(entries, *entity, spec, self.here)?;
                    Ok((*name, Value::Id(id)))
                }
            })
            .collect()
    }

    /// Waits for the reply to `wait`, collecting its entries, and fails if
    /// it or request `call` (the method `name`) is refused.
    fn reply(&mut self, wait: u64, call: u64, name: &str) -> Result<Reply, Failure> {
        self.reply_keeping(wait, call, name, None)
    }

    /// [`Session::reply`], keeping the numbered events that arrive before
    /// it in `kept`.
    fn reply_keeping(
        &mut self,
        wait: u64,
        call: u64,
        name: &str,
        mut kept: Option<&mut wait::Kept>,
    ) -> Result<Reply, Failure> {
        let mut entries = Vec::new();
        loop {
            match self.conn.recv()? {
                Message::Event {
                    seq: Some(seq),
                    event,
                } => {
                    if let Some(kept) = kept.as_deref_mut() {
                        kept.push((seq, event));
                    }
                }
                Message::Entry { req, entry } if req == wait => entries.push(entry),
                Message::Done { req, created } if req == wait => {
                    return Ok(Reply::Done { created, entries });
                }
                Message::Text { req, text } if req == wait => return Ok(Reply::Text(text)),
                Message::Failed { req, reason, code } if req == wait || req == call => {
                    if self.json {
                        self.line(&render::failed_line(name, &reason))?;
                    }
                    let reason = format!("{name}: {reason}");
                    return Err(match code {
                        Some(Code::NotFound) => Failure::Missing(reason),
                        Some(Code::MethodNotFound) | None => Failure::Failed(reason),
                    });
                }
                _ => {}
            }
        }
    }

    fn print(&mut self, method: &str, reply: Reply) -> Result<(), Failure> {
        let lines = match reply {
            Reply::Text(text) => vec![render::text_line(method, text, self.json)],
            Reply::Done { created, entries } => {
                let mut lines: Vec<String> = entries
                    .iter()
                    .map(|e| render::entry_line(e, self.json))
                    .collect();
                lines.extend(render::done_line(method, created, self.json));
                lines
            }
        };
        lines.iter().try_for_each(|line| self.line(line))
    }

    fn events(
        &mut self,
        after: Option<Cursor>,
        follow: bool,
        needs: &[(&str, Features)],
        on_event: &mut OnEvent<'_>,
    ) -> Result<(), Failure> {
        let wanted = Features::EVENT_SEQ
            .union(Features::SYNC)
            .union(Features::STATUS)
            .union(Features::TERMINAL_EVENTS)
            .union(Features::PROGRAM_EVENTS)
            .union(Features::PROCESS)
            .union(Features::LABELS)
            .union(Features::LAYOUT)
            .union(Features::IDENTITY)
            .union(Features::MORE_PROGRAM_EVENTS)
            .union(Features::ACTIVE_WINDOW);
        let (features, start) = self.hello(wanted, after)?;
        if let Some((kind, _)) = needs.iter().find(|(_, need)| !features.contains(*need)) {
            return Err(Failure::Failed(format!(
                "the server does not send `{kind}` events, which a rule needs"
            )));
        }
        let start =
            start.ok_or_else(|| Failure::Failed("the server does not number events".into()))?;
        self.line(&render::cursor_line(start, self.json))?;
        if let Some(after) = after.filter(|a| *a != start) {
            return Err(Failure::Failed(format!(
                "cannot resume from {}: events since were lost; start again from {}",
                render::cursor_text(after),
                render::cursor_text(start)
            )));
        }
        // Without --follow, the reply to a Sync sent now marks the end of
        // the replay: the server sends it only after the missed events.
        if !follow {
            self.sync(features, 1)?;
        }
        let mut last = start;
        loop {
            match self.conn.recv()? {
                Message::Event {
                    seq: Some(seq),
                    event,
                } => {
                    on_event(seq, &event, &mut *self.out)?;
                    last.seq = seq;
                }
                Message::Event { seq: None, .. } => {
                    return Err(Failure::Failed("an unnumbered event".into()));
                }
                Message::Done { req: 1, .. } | Message::Failed { req: 1, .. } if !follow => {
                    return self.line(&render::cursor_line(last, self.json));
                }
                _ => {}
            }
        }
    }
}

/// Fails unless the server can run `call` as ctl sends it, with every
/// argument it is given: a server would ignore an optional one it lacks
/// the feature for.
fn require(features: Features, call: &Call) -> Result<(), Failure> {
    let method = call.method;
    if method.returns == Returns::Unanswered {
        require_sync(features)?;
    }
    if !features.contains(method.needs) {
        return Err(Failure::Failed(format!(
            "the server does not support {}",
            method.name
        )));
    }
    let given = call.args.iter().map(|(name, _)| *name);
    if !features.contains(method.needs_for(given)) {
        let optional: Vec<String> = call
            .args
            .iter()
            .filter(|(name, _)| method.needs_for([*name]) != method.needs)
            .map(|(name, _)| format!("--{name}"))
            .collect();
        return Err(Failure::Failed(format!(
            "the server does not support {} with {}",
            method.name,
            optional.join(", ")
        )));
    }
    Ok(())
}

fn require_sync(features: Features) -> Result<(), Failure> {
    if features.contains(Features::SYNC) {
        return Ok(());
    }
    Err(Failure::Failed(
        "the server does not support sync, needed to confirm input".into(),
    ))
}

#[cfg(test)]
mod tests {
    use super::*;

    fn create_block(line: &str) -> Call {
        let args: Vec<String> = line.split_whitespace().map(String::from).collect();
        match cli::parse(&args, &Defaults::default()) {
            Ok(Invocation {
                command: Command::Calls(mut calls),
                ..
            }) => calls.remove(0),
            other => panic!("{other:?}"),
        }
    }

    /// `bridge` would ignore `stop` and `out` and take over the process's
    /// stdin, so the library refuses it before connecting, even with a
    /// stop already passed and a server that would never answer.
    #[cfg(unix)]
    #[test]
    fn the_library_refuses_bridge_without_connecting() {
        let socket =
            std::env::temp_dir().join(format!("rusty-mux-ctl-silent-{}.sock", std::process::id()));
        let _ = std::fs::remove_file(&socket);
        let listener = std::os::unix::net::UnixListener::bind(&socket).unwrap();
        listener.set_nonblocking(true).unwrap();
        let env = Env {
            socket: Some(socket.clone()),
            here: Defaults::default(),
            config: None,
            agent_dir: None,
            ctl: None,
        };
        let stop = Stop::after(std::time::Duration::ZERO);
        let (mut out, mut err) = (Vec::new(), Vec::new());
        let exit = run_until(&["bridge".to_owned()], &env, &stop, &mut out, &mut err);
        let connected = listener.accept().is_ok();
        std::fs::remove_file(&socket).unwrap();
        assert_eq!(exit, Exit::Usage);
        assert!(out.is_empty(), "{out:?}");
        assert!(String::from_utf8_lossy(&err).contains("only as the rusty-mux-ctl program"));
        assert!(!connected, "the library connected for bridge");
    }

    #[test]
    fn only_launch_arguments_need_the_launch_feature() {
        let plain = create_block(
            "create-block --window 00000000-0000-0000-0000-000000000001 --cols 80 --rows 24",
        );
        assert!(require(Features::NONE, &plain).is_ok());
        let launched = create_block(
            "create-block --window 00000000-0000-0000-0000-000000000001 --cols 80 --rows 24 \
             --cwd /tmp",
        );
        assert!(matches!(
            require(Features::NONE, &launched),
            Err(Failure::Failed(why)) if why.ends_with("with --cwd")
        ));
        assert!(require(Features::LAUNCH, &launched).is_ok());
    }
}
