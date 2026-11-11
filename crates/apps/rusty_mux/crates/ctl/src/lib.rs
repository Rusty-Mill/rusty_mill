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
//! `own` holds one connection that owns every block it launches (C46),
//! driven by JSON lines on stdin; see the `own` module.
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
mod completion;
mod config;
mod conn;
mod hosts;
#[cfg(any(unix, windows))]
mod own;
mod render;
mod ssh;
mod ssh_write;
mod stop;
pub mod target;
mod terminfo;
mod wait;
mod watch;

use std::ffi::OsString;
use std::io::{self, Write};
use std::num::NonZeroU8;
use std::path::{Path, PathBuf};
use std::time::{Duration, Instant};

use rusty_mux_core::Event;
use rusty_mux_proto::schema::{CATALOG, Returns, Value};
use rusty_mux_proto::{Code, Cursor, Entry, Features, Message};
use rusty_uuid::Uuid;

pub use crate::api::input_schema;
use crate::cli::{
    AgentCommand, Api, Arg, Call, Command, ConfigCommand, Defaults, HookReport, Invocation,
};
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
    /// The default socket (C69, `rusty_mux_proto::endpoint`), used when
    /// neither `--socket` nor `$RUSTY_MUX_SOCKET` names one. Agent hook
    /// reports never use it: outside a block they do nothing.
    pub default_socket: Option<PathBuf>,
    /// `$RUSTY_MUX_SESSION`, `$RUSTY_MUX_WINDOW`, `$RUSTY_MUX_BLOCK` and
    /// the current directory.
    pub here: Defaults,
    /// The config file `do` and `config` read: `$RUSTY_MUX_CONFIG`, else
    /// `rusty_mux/config.json` in the config directory.
    pub config: Option<PathBuf>,
    /// The saved hosts `--host` and `hosts` read (C35):
    /// `rusty_mux/hosts.json` in the config directory.
    pub hosts: Option<PathBuf>,
    /// Where `agent` finds each agent's config directory.
    pub agent_homes: AgentHomes,
    /// This program, which the agent's session-start hook runs.
    pub ctl: Option<PathBuf>,
    /// `$RUSTY_MUX_SSH`: the `ssh` that `terminfo`, `ssh` and `--host`'s
    /// ssh addresses run; `ssh` when unset.
    pub ssh: Option<OsString>,
    /// The folder OpenSSH's `~` means, whose `.ssh` the `ssh` commands
    /// read and write: see [`ssh_home`].
    pub ssh_home: Option<PathBuf>,
}

/// The home OpenSSH uses, by `var`: `%USERPROFILE%` on Windows, where
/// Win32-OpenSSH ignores `$HOME` (which Git Bash or MSYS may set to another
/// folder), else `$HOME`; the other one when that is unset. An empty
/// variable counts as unset.
pub fn ssh_home(var: impl Fn(&str) -> Option<OsString>) -> Option<PathBuf> {
    let set = |name: &str| var(name).filter(|v| !v.is_empty()).map(PathBuf::from);
    let (first, second) = match cfg!(windows) {
        true => ("USERPROFILE", "HOME"),
        false => ("HOME", "USERPROFILE"),
    };
    set(first).or_else(|| set(second))
}

impl Env {
    /// This process's environment. A malformed id is ignored, with a warning
    /// on stderr.
    pub fn from_process() -> Self {
        Self {
            socket: std::env::var_os("RUSTY_MUX_SOCKET").map(Into::into),
            default_socket: rusty_mux_proto::endpoint::default_socket(|name| {
                std::env::var_os(name)
            }),
            here: Defaults {
                session: id_var("RUSTY_MUX_SESSION"),
                window: id_var("RUSTY_MUX_WINDOW"),
                block: id_var("RUSTY_MUX_BLOCK"),
                dir: std::env::current_dir().ok(),
            },
            config: config::default_path(),
            hosts: hosts::default_path(),
            agent_homes: AgentHomes::from_process(),
            ctl: std::env::current_exe().ok(),
            ssh: std::env::var_os("RUSTY_MUX_SSH"),
            ssh_home: ssh_home(|name| std::env::var_os(name)),
        }
    }
}

/// The environment that says where coding agents keep their config. An
/// empty variable counts as unset.
#[derive(Debug, Clone, Default)]
pub struct AgentHomes {
    /// `$HOME`, else `%USERPROFILE%`.
    pub home: Option<PathBuf>,
    /// `$CLAUDE_CONFIG_DIR`: Claude Code's directory, else `~/.claude`.
    pub claude: Option<PathBuf>,
    /// `$CODEX_HOME`: Codex's directory, else `~/.codex`.
    pub codex: Option<PathBuf>,
    /// `$XDG_CONFIG_HOME` when absolute: holds OpenCode's `opencode`, else
    /// `~/.config` does.
    pub config: Option<PathBuf>,
    /// `$PI_CODING_AGENT_DIR`: Pi's directory, else `~/.pi/agent`.
    pub pi: Option<PathBuf>,
}

impl AgentHomes {
    fn from_process() -> Self {
        let var = |name: &str| {
            std::env::var_os(name)
                .filter(|v| !v.is_empty())
                .map(PathBuf::from)
        };
        Self {
            home: var("HOME").or_else(|| var("USERPROFILE")),
            claude: var("CLAUDE_CONFIG_DIR"),
            codex: var("CODEX_HOME"),
            config: var("XDG_CONFIG_HOME").filter(|dir| dir.is_absolute()),
            pi: var("PI_CODING_AGENT_DIR"),
        }
    }
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

/// Why [`run`] refuses `own`.
const OWN_ONLY: &str =
    "own runs only as the rusty-mux-ctl program: it takes over the process's stdin and stdout";

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
        Ok(invocation) if matches!(invocation.command, Command::Own) => {
            return match socket(&invocation, env, &Stop::default()) {
                Ok(reached) => finish(own(&reached.socket), &reached.socket, &mut err),
                Err((exit, reason)) => report(&mut err, exit, &reason),
            };
        }
        Ok(Invocation {
            command: Command::Agent(AgentCommand::Report(mut hook)),
            ref socket,
            ..
        }) if hook.tool_from_stdin => {
            // One budget for the whole hook: stdin gets the first part of
            // it, so a writer that never closes still leaves time to report.
            let stop = Stop::after(HOOK_PATIENCE);
            let input = hook_input_within(io::stdin, HOOK_INPUT_PATIENCE);
            hook.msg = input.and_then(|input| hook_tool_name(&input));
            let socket = socket.clone().or_else(|| env.socket.clone());
            return send_hook_report(socket.as_deref(), env, &hook, &stop);
        }
        _ => return run(args, env, &mut io::stdout(), &mut err),
    };
    match socket(&bridged, env, &Stop::default()) {
        Ok(reached) => finish(bridge(&reached.socket), &reached.socket, &mut err),
        Err((exit, reason)) => report(&mut err, exit, &reason),
    }
}

/// Sends an agent's event-hook report for the block it runs in. A hook
/// must never fail, print or hold the agent up (Claude Code adds some hook
/// output to the model's context, Codex fails a `Stop` hook that prints), so
/// a report that cannot be sent is dropped: the program's own OSC 7501
/// status still shows.
fn send_hook_report(socket: Option<&Path>, env: &Env, hook: &HookReport, stop: &Stop) -> Exit {
    if let (Some(socket), Some(block)) = (socket, env.here.block) {
        let _dropped = hook_report(socket, block, hook, stop);
    }
    Exit::Ok
}

/// How long `--tool-from-stdin` waits for the event's input to end: half of
/// [`HOOK_PATIENCE`], so the report keeps the rest.
const HOOK_INPUT_PATIENCE: Duration = Duration::from_secs(1);

/// The most of a hook event's stdin `--tool-from-stdin` reads.
const HOOK_INPUT_LIMIT: usize = 1 << 20;

/// A hook event's input, read up to [`HOOK_INPUT_LIMIT`] bytes, or `None`
/// when it is longer, cannot be read or has not ended within `budget`. It is
/// read on a thread of its own, so a writer that never closes the pipe
/// holds the hook no longer than `budget`; that thread drains the rest of a
/// longer input, so the writer never blocks on a full pipe, until the
/// process exits.
fn hook_input_within<R: io::Read + 'static>(
    open: impl FnOnce() -> R + Send + 'static,
    budget: Duration,
) -> Option<Vec<u8>> {
    let (sent, received) = std::sync::mpsc::sync_channel(1);
    let reader = std::thread::Builder::new()
        .name("hook-input".into())
        .spawn(move || {
            let mut input = open();
            let _gone = sent.send(read_hook_input(&mut input));
            let _drained = io::copy(&mut input, &mut io::sink());
        });
    reader.ok()?;
    received.recv_timeout(budget).ok().flatten()
}

/// `input` up to [`HOOK_INPUT_LIMIT`] bytes; `None` when it is longer or
/// cannot be read.
fn read_hook_input(input: &mut impl io::Read) -> Option<Vec<u8>> {
    use io::Read as _;
    let mut bytes = Vec::new();
    let read = input
        .take(HOOK_INPUT_LIMIT as u64 + 1)
        .read_to_end(&mut bytes)
        .ok()?;
    (read <= HOOK_INPUT_LIMIT).then_some(bytes)
}

/// The longest tool name a report carries.
const MAX_TOOL_NAME: usize = 128;

/// The `tool_name` of a hook event's JSON, when it can be a report's
/// message: 1 to [`MAX_TOOL_NAME`] bytes with no control characters (D89).
/// A name that cannot leaves the report without one rather than dropping it.
pub fn hook_tool_name(input: &[u8]) -> Option<String> {
    let value = rusty_json::Value::from_json_str(std::str::from_utf8(input).ok()?).ok()?;
    let name = value.get("tool_name")?.as_str()?;
    let fits = (1..=MAX_TOOL_NAME).contains(&name.len()) && !name.chars().any(char::is_control);
    fits.then(|| name.to_owned())
}

/// The socket of the host `invocation` names (see [`hosts::resolve`]),
/// else the socket it names, else `$RUSTY_MUX_SOCKET`, else the default
/// socket; the error carries its exit code.
fn socket(
    invocation: &Invocation,
    env: &Env,
    stop: &Stop,
) -> Result<hosts::Reached, (Exit, String)> {
    if let Some(name) = &invocation.host {
        return hosts::resolve(name, env, stop);
    }
    invocation
        .socket
        .clone()
        .or_else(|| env.socket.clone())
        .or_else(|| env.default_socket.clone())
        .map(hosts::Reached::local)
        .ok_or((Exit::Usage, "no socket: pass --socket or --host, set RUSTY_MUX_SOCKET, or set XDG_RUNTIME_DIR (LOCALAPPDATA on Windows) for the default".to_owned()))
}

/// The socket an agent hook reports to: only one named outright, since a
/// hook must not wait on a host's addresses.
fn hook_socket(invocation: &Invocation, env: &Env) -> Option<PathBuf> {
    if invocation.host.is_some() {
        return None;
    }
    invocation
        .socket
        .clone()
        .or_else(|| env.socket.clone())
        .or_else(|| env.default_socket.clone())
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
    // Where the agent hook asks for its session's name.
    let hook_socket = hook_socket(&invocation, env);
    match invocation.command {
        Command::Help(method) => return write_line(out, cli::usage(method).trim_end()),
        Command::Api(Api::List) => return write_line(out, &api::list(invocation.json)),
        Command::Api(Api::Describe(method)) => return write_line(out, &api::describe(method)),
        Command::Api(Api::Llms) => return write_line(out, api::llms().trim_end()),
        Command::Completion(shell) => {
            return write_line(out, completion::script(shell).trim_end());
        }
        Command::Config(command) => return run_config(&command, env, invocation.json, out, err),
        Command::Hosts(command) => {
            return match hosts::run(&command, env, invocation.json, out) {
                Ok(()) => Exit::Ok,
                Err((exit, reason)) => report(err, exit, &reason),
            };
        }
        Command::Ssh(command) => {
            let program = env.ssh.clone().unwrap_or_else(|| "ssh".into());
            let home = env.ssh_home.as_deref();
            let config = home.map(|h| h.join(".ssh").join("config"));
            return ssh::run(&command, &program, config.as_deref(), home, out)
                .unwrap_or_else(|(exit, reason)| report(err, exit, &reason));
        }
        Command::Terminfo(command) => {
            let ssh = env.ssh.clone().unwrap_or_else(|| "ssh".into());
            return terminfo::run(&command, &ssh, out)
                .unwrap_or_else(|(exit, reason)| report(err, exit, &reason));
        }
        Command::Agent(AgentCommand::Report(hook)) => {
            return send_hook_report(hook_socket.as_deref(), env, &hook, stop);
        }
        Command::Agent(command) => {
            let generated_name = || {
                let (socket, session) = (hook_socket.as_deref()?, env.here.session?);
                hook_generated_name(socket, session, stop).ok().flatten()
            };
            return agent::run(&command, env, invocation.json, out, &generated_name)
                .unwrap_or_else(|reason| report(err, Exit::Failed, &reason));
        }
        Command::Do { action, args } => match action_calls(&action, &args, env) {
            Ok(calls) => invocation.command = Command::Calls(calls),
            Err((exit, reason)) => return report(err, exit, &reason),
        },
        // Unbounded, and on the process's own stdin and stdout, so never
        // through `stop` and `out`.
        Command::Bridge => return report(err, Exit::Usage, BRIDGE_ONLY),
        Command::Own => return report(err, Exit::Usage, OWN_ONLY),
        Command::Calls(_)
        | Command::Events { .. }
        | Command::Watch { .. }
        | Command::Wait(_)
        | Command::Run(_)
        | Command::ServerStop => {}
    }
    let reached = match socket(&invocation, env, stop) {
        Ok(reached) => reached,
        Err((exit, reason)) => return report(err, exit, &reason),
    };
    let socket = &reached.socket;
    // A follower also looks at the hosts file between reads.
    let follows = matches!(
        invocation.command,
        Command::Watch { .. } | Command::Events { follow: true, .. }
    );
    let tracker = (follows && (env.hosts.is_some() || env.config.is_some()))
        .then(|| hosts::Tracker::new(env.hosts.clone(), env.config.clone()));
    let stop = match tracker {
        Some(_) => stop.polling(),
        None => stop.clone(),
    };
    if let Command::Watch { after } = invocation.command {
        return run_watch(
            after,
            socket,
            env,
            invocation.json,
            &stop,
            tracker,
            out,
            err,
        );
    }
    let deadline = timeout(&invocation.command).and_then(|t| Instant::now().checked_add(t));
    let stop = deadline.map_or_else(|| stop.clone(), |d| stop.sooner(d));
    let result = execute(
        &invocation,
        socket,
        env.here.session,
        &stop,
        tracker,
        out,
        err,
    );
    finish(timed_out(result, deadline), socket, err)
}

/// How long `server stop` waits for the server to finish stopping.
const STOP_PATIENCE: std::time::Duration = std::time::Duration::from_secs(15);

/// Whether `e` says the server closed the connection.
fn closed(e: &io::Error) -> bool {
    matches!(
        e.kind(),
        io::ErrorKind::UnexpectedEof | io::ErrorKind::ConnectionReset | io::ErrorKind::BrokenPipe
    )
}

/// How long a `wait` or `run --wait` may wait.
fn timeout(command: &Command) -> Option<std::time::Duration> {
    match command {
        Command::Wait(wait) => wait.timeout,
        Command::Run(run) => run.wait.flatten(),
        Command::ServerStop => Some(STOP_PATIENCE),
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
#[allow(clippy::too_many_arguments)] // Each is one part of the command.
fn run_watch(
    after: Option<Cursor>,
    socket: &Path,
    env: &Env,
    json: bool,
    stop: &Stop,
    tracker: Option<hosts::Tracker>,
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
    let result = follow(socket, &how, stop, tracker, out, &mut |seen, out| {
        let done = match seen {
            Seen::Event(seq, event) => watcher.event(seq, event, out, &mut *err),
            Seen::HostsChanged => watcher.local(hosts::CHANGED, out, &mut *err),
            Seen::HostsBroken(reason) => writeln!(err, "rusty-mux-ctl: {reason}"),
        };
        done.map_err(Failure::from)
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
        Err(Failure::Unreachable(e)) => report(err, Exit::Unreachable, &unreachable(socket, &e)),
    }
}

/// Why `socket` could not be reached: plainly "no server" when nothing is
/// there at all (the raw error says little, and on Windows it is
/// `WSAENETDOWN`), else the error.
fn unreachable(socket: &Path, e: &io::Error) -> String {
    let lookup = std::fs::symlink_metadata(socket).err().map(|e| e.kind());
    describe(socket, e, lookup)
}

/// [`unreachable`]'s message, given how looking `socket` up failed: only
/// a lookup that found nothing says no server is running; any other
/// failure (no permission, a loop) keeps the connect error.
fn describe(socket: &Path, e: &io::Error, lookup: Option<io::ErrorKind>) -> String {
    let shown = socket.display();
    match lookup {
        Some(io::ErrorKind::NotFound) => format!("no server is running: nothing at {shown}"),
        _ => format!("{shown}: {e}"),
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
        ConfigCommand::Check(_) => {
            match hosts::check_home(&config.declared, env.hosts.as_deref()) {
                Ok(()) => write_line(out, &config::summary(&config)),
                Err(reason) => report(err, Exit::Failed, &reason),
            }
        }
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
    tracker: Option<hosts::Tracker>,
    out: &mut dyn Write,
    err: &mut dyn Write,
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
            .events(*after, *follow, &[], tracker, &mut |seen, out| {
                match seen {
                    Seen::Event(seq, event) => {
                        writeln!(out, "{}", render::event_line(seq, event, json))
                    }
                    Seen::HostsChanged => writeln!(out, "{}", hosts::changed_line(json)),
                    Seen::HostsBroken(reason) => writeln!(err, "rusty-mux-ctl: {reason}"),
                }
                .map_err(Failure::from)
            })
            .map(|()| Exit::Ok),
        Command::Wait(wait) => session.wait(&wait.until),
        Command::Run(run) => session.run(run),
        Command::ServerStop => session.stop_server(),
        Command::Help(_)
        | Command::Api(_)
        | Command::Do { .. }
        | Command::Config(_)
        | Command::Hosts(_)
        | Command::Ssh(_)
        | Command::Terminfo(_)
        | Command::Agent(_)
        | Command::Watch { .. }
        | Command::Bridge
        | Command::Own
        | Command::Completion(_) => Ok(Exit::Ok),
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

/// `own`, for [`run_process`] only: until stdin ends or the ownership is
/// lost.
#[cfg(any(unix, windows))]
fn own(socket: &Path) -> Result<Exit, Failure> {
    own::run(socket, io::BufReader::new(io::stdin()), &mut io::stdout())
}

#[cfg(not(any(unix, windows)))]
fn own(_: &Path) -> Result<Exit, Failure> {
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
    tracker: Option<hosts::Tracker>,
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
    session.events(how.after, true, how.needs, tracker, on_event)
}

#[cfg(not(any(unix, windows)))]
fn follow(
    _: &Path,
    _: &Follow<'_>,
    _: &Stop,
    _: Option<hosts::Tracker>,
    _: &mut dyn Write,
    _: &mut OnEvent<'_>,
) -> Result<(), Failure> {
    Err(unsupported())
}

/// How long the agent hook waits for the server to tell a session's name,
/// well inside the 5 s the OpenCode and Pi scripts give the whole hook.
/// A hard bound on reads and writes, and on connecting where
/// [`conn::connect`] bounds it (Linux); elsewhere the host's hook timeout
/// bounds a connect.
const HOOK_PATIENCE: std::time::Duration = std::time::Duration::from_secs(2);

/// Sends an agent hook's report on `block` to the server at `socket` with
/// `terminal.report-status`, within [`HOOK_PATIENCE`]. Nothing is sent to
/// a server that does not know the method.
#[cfg(any(unix, windows))]
fn hook_report(socket: &Path, block: Uuid, hook: &HookReport, stop: &Stop) -> Result<(), Failure> {
    let method = rusty_mux_proto::schema::block_method("terminal.report-status")
        .ok_or_else(|| Failure::Failed("terminal.report-status is not in the catalog".into()))?;
    let stop = stop.sooner(Instant::now() + HOOK_PATIENCE);
    let mut conn = Conn::new(conn::connect(socket, &stop)?, stop);
    let mut sink = io::sink();
    let mut client = Session {
        conn: &mut conn,
        out: &mut sink,
        json: false,
        here: None,
    };
    let (features, _) = client.hello(method.needs, None)?;
    if !features.contains(method.needs) {
        return Ok(());
    }
    let mut args = vec![
        ("block", Value::Id(block)),
        ("state", Value::Text(hook.state.clone())),
    ];
    let optional = [
        ("id", &hook.id),
        ("kind", &hook.kind),
        ("app", &hook.app),
        ("msg", &hook.msg),
    ];
    for (name, value) in optional {
        if let Some(value) = value {
            args.push((name, Value::Text(value.clone())));
        }
    }
    let message = method
        .request(1, &args)
        .map_err(|e| Failure::Failed(e.to_string()))?;
    client.conn.send(&message)?;
    client.reply(1, 1, method.name)?;
    Ok(())
}

#[cfg(not(any(unix, windows)))]
fn hook_report(_: &Path, _: Uuid, _: &HookReport, _: &Stop) -> Result<(), Failure> {
    Err(unsupported())
}

/// The name of `session` on the server at `socket` while it is one the
/// server made up and nobody has renamed it since, for the agent hook.
/// `None` from a server without [`Features::NAME_SESSION`], which could
/// not keep a name chosen after the hook ran.
#[cfg(any(unix, windows))]
fn hook_generated_name(
    socket: &Path,
    session: Uuid,
    stop: &Stop,
) -> Result<Option<String>, Failure> {
    let stop = stop.sooner(Instant::now() + HOOK_PATIENCE);
    let mut conn = Conn::new(conn::connect(socket, &stop)?, stop);
    let mut sink = io::sink();
    let mut client = Session {
        conn: &mut conn,
        out: &mut sink,
        json: false,
        here: None,
    };
    let (features, _) = client.hello(Features::INSPECT.union(Features::NAME_SESSION), None)?;
    if !features.contains(Features::NAME_SESSION) {
        return Ok(None);
    }
    client
        .list(features, 1)?
        .into_iter()
        .find_map(|entry| match entry {
            Entry::Session {
                session: s,
                name,
                generated,
                ..
            } if s.uuid() == session => Some(generated.then_some(name)),
            _ => None,
        })
        .ok_or_else(|| Failure::Failed(format!("no session {session}")))
}

#[cfg(not(any(unix, windows)))]
fn hook_generated_name(_: &Path, _: Uuid, _: &Stop) -> Result<Option<String>, Failure> {
    Err(unsupported())
}

/// What a follower sees.
enum Seen<'e> {
    /// A numbered server event.
    Event(u64, &'e Event),
    /// ctl's own `hosts-changed`: the saved hosts changed.
    HostsChanged,
    /// The hosts file no longer parses: why, once until it does again.
    HostsBroken(String),
}

/// What happens to each thing a follower sees.
type OnEvent<'a> = dyn FnMut(Seen<'_>, &mut dyn Write) -> Result<(), Failure> + 'a;

#[cfg(not(any(unix, windows)))]
fn execute(
    _: &Invocation,
    _: &Path,
    _: Option<Uuid>,
    _: &Stop,
    _: Option<hosts::Tracker>,
    _: &mut dyn Write,
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
            version: Some(rusty_mux_proto::VERSION.into()),
            features,
            resume,
            takeover: None,
        })?;
        match self.conn.recv()? {
            Message::Welcome {
                features: granted,
                cursor,
                version,
                ..
            } => {
                // Said on stderr, so scripts reading stdout are not upset (C36).
                let older =
                    rusty_mux_proto::older::Older::check(features, granted, version.as_deref());
                if let Some(older) = older {
                    eprintln!("rusty-mux-ctl: {}", older.line("the server"));
                }
                Ok((granted, cursor))
            }
            other => Err(Failure::Failed(format!("expected Welcome, got {other:?}"))),
        }
    }

    /// `server stop` (C36b): asks for a clean stop, then waits for the
    /// server to close the connection, which it does only once its state
    /// is saved and its name released.
    fn stop_server(&mut self) -> Result<Exit, Failure> {
        const REQ: u64 = 1;
        let (features, _) = self.hello(Features::SHUTDOWN, None)?;
        if !features.contains(Features::SHUTDOWN) {
            return Err(Failure::Failed(
                "server stop: this server cannot be stopped by request".into(),
            ));
        }
        self.conn.send(&Message::Shutdown { req: REQ })?;
        self.reply(REQ, REQ, "server stop")?;
        loop {
            match self.conn.recv() {
                Ok(_) => {}
                Err(e) if closed(&e) => return Ok(Exit::Ok),
                Err(e) => return Err(e.into()),
            }
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
        let wanted = CATALOG
            .iter()
            .fold(Features::SYNC, |all, m| all.union(m.all_needs()));
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
        mut tracker: Option<hosts::Tracker>,
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
            .union(Features::ACTIVE_WINDOW)
            .union(Features::CONFIG_RELOAD)
            .union(Features::LAYOUT_EDIT)
            .union(Features::FOCUS);
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
            let message = self.conn.poll()?;
            // Looked at on every turn, so a steady event stream can't
            // starve it; it reads the file at most once a CHECK_EVERY.
            let hosts = match tracker.as_mut().map(hosts::Tracker::check) {
                Some(hosts::Change::Changed) => Some(Seen::HostsChanged),
                Some(hosts::Change::Broken(reason)) => Some(Seen::HostsBroken(reason)),
                Some(hosts::Change::None) | None => None,
            };
            if let Some(seen) = hosts {
                on_event(seen, &mut *self.out)?;
            }
            let Some(message) = message else {
                continue;
            };
            match message {
                Message::Event {
                    seq: Some(seq),
                    event,
                } => {
                    on_event(Seen::Event(seq, &event), &mut *self.out)?;
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
    let given: Vec<&str> = call.args.iter().map(|(name, _)| *name).collect();
    if features.contains(method.needs_for(given.iter().copied())) {
        return Ok(());
    }
    // Each parameter whose presence or absence the server cannot handle.
    let unsupported: Vec<String> = method
        .params
        .iter()
        .filter(|p| p.extra_needs(&given).is_some_and(|n| !features.contains(n)))
        .map(|p| match given.contains(&p.name) {
            true => format!("with --{}", p.name),
            false => format!("without --{}", p.name),
        })
        .collect();
    Err(Failure::Failed(format!(
        "the server does not support {} {}",
        method.name,
        unsupported.join(", ")
    )))
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

    #[test]
    fn ssh_home_is_the_profile_on_windows_and_home_elsewhere() {
        let both = |name: &str| match name {
            "HOME" => Some(OsString::from("/msys/home/me")),
            "USERPROFILE" => Some(OsString::from("C:/Users/me")),
            _ => None,
        };
        let want = match cfg!(windows) {
            true => "C:/Users/me",
            false => "/msys/home/me",
        };
        assert_eq!(ssh_home(both), Some(PathBuf::from(want)));
        let profile_only = |name: &str| (name == "USERPROFILE").then(|| OsString::from("C:/u"));
        assert_eq!(ssh_home(profile_only), Some(PathBuf::from("C:/u")));
        let empty = |name: &str| (name == "HOME").then(OsString::new);
        assert_eq!(ssh_home(empty), None, "empty counts as unset");
    }

    #[test]
    fn only_a_missing_socket_reads_as_no_server() {
        let socket = Path::new("/run/x/default.sock");
        let refused = io::Error::from(io::ErrorKind::ConnectionRefused);
        let missing = describe(socket, &refused, Some(io::ErrorKind::NotFound));
        assert_eq!(
            missing,
            "no server is running: nothing at /run/x/default.sock"
        );
        for lookup in [Some(io::ErrorKind::PermissionDenied), None] {
            let said = describe(socket, &refused, lookup);
            assert!(
                said.starts_with("/run/x/default.sock: "),
                "{lookup:?}: {said}"
            );
        }
    }

    #[test]
    fn a_hook_tool_name_is_kept_only_when_a_report_can_carry_it() {
        let name = |json: &str| hook_tool_name(json.as_bytes());
        assert_eq!(name(r#"{"tool_name":"Bash"}"#), Some("Bash".into()));
        assert_eq!(
            name(r#"{"tool_name":"mcp__github__get_me","tool_input":{}}"#),
            Some("mcp__github__get_me".into())
        );
        let long = format!(r#"{{"tool_name":"{}"}}"#, "a".repeat(MAX_TOOL_NAME + 1));
        for bad in [
            "",
            "not json",
            "[]",
            "{}",
            r#"{"tool_name":7}"#,
            r#"{"tool_name":""}"#,
            r#"{"tool_name":"a\nb"}"#,
            r#"{"tool_name":"a\u001bb"}"#,
            &long,
        ] {
            assert_eq!(name(bad), None, "{bad}");
        }
        assert_eq!(hook_tool_name(b"{\"tool_name\":\"\xff\"}"), None);
    }

    #[test]
    fn hook_input_is_read_up_to_its_limit() {
        let at_limit = vec![b'x'; HOOK_INPUT_LIMIT];
        assert_eq!(
            read_hook_input(&mut at_limit.as_slice()),
            Some(at_limit.clone())
        );
        let over = vec![b'x'; HOOK_INPUT_LIMIT + 5];
        assert_eq!(read_hook_input(&mut over.as_slice()), None);
    }

    /// Review on #1516: an input that never ends holds the hook only for
    /// its budget.
    #[test]
    fn hook_input_that_never_ends_gives_up_within_its_budget() {
        struct Endless;
        impl io::Read for Endless {
            fn read(&mut self, _: &mut [u8]) -> io::Result<usize> {
                std::thread::sleep(Duration::from_secs(3600));
                Ok(0)
            }
        }
        let started = Instant::now();
        assert_eq!(
            hook_input_within(|| Endless, Duration::from_millis(100)),
            None
        );
        assert!(
            started.elapsed() < Duration::from_secs(2),
            "{:?}",
            started.elapsed()
        );
        let small = || io::Cursor::new(br#"{"tool_name":"Bash"}"#.to_vec());
        assert_eq!(
            hook_input_within(small, Duration::from_secs(5)),
            Some(br#"{"tool_name":"Bash"}"#.to_vec())
        );
        let over = || io::Cursor::new(vec![b'x'; HOOK_INPUT_LIMIT + 5]);
        assert_eq!(hook_input_within(over, Duration::from_secs(5)), None);
    }

    fn parsed(line: &str) -> Call {
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
            default_socket: None,
            here: Defaults::default(),
            config: None,
            hosts: None,
            agent_homes: AgentHomes::default(),
            ctl: None,
            ssh: None,
            ssh_home: None,
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
        let plain = parsed(
            "create-block --window 00000000-0000-0000-0000-000000000001 --cols 80 --rows 24",
        );
        assert!(require(Features::NONE, &plain).is_ok());
        let launched = parsed(
            "create-block --window 00000000-0000-0000-0000-000000000001 --cols 80 --rows 24 \
             --cwd /tmp",
        );
        assert!(matches!(
            require(Features::NONE, &launched),
            Err(Failure::Failed(why)) if why.ends_with("with --cwd")
        ));
        assert!(require(Features::LAUNCH, &launched).is_ok());
    }

    #[test]
    fn leaving_out_a_session_name_needs_the_server_to_make_one_up() {
        let named = parsed("create-session --name work");
        assert!(require(Features::NONE, &named).is_ok());
        let unnamed = parsed("create-session");
        assert!(matches!(
            require(Features::NONE, &unnamed),
            Err(Failure::Failed(why)) if why == "the server does not support create-session without --name"
        ));
        assert!(require(Features::SESSION_NAMES, &unnamed).is_ok());
    }
}
