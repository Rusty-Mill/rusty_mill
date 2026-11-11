//! Command-line parsing, with no I/O. Every request subcommand and its flags
//! come from [`schema::CATALOG`].

use std::path::PathBuf;
use std::str::FromStr;
use std::time::Duration;

use rusty_mux_proto::Cursor;
use rusty_mux_proto::schema::{self, CallError, Entity, Method, Presence, Ty, Value};
use rusty_mux_script::hosts::Address;
use rusty_uuid::Uuid;

/// A parsed command line.
#[derive(Debug, PartialEq, Eq)]
pub struct Invocation {
    pub socket: Option<PathBuf>,
    /// `--host NAME`: a host from the config or the hosts file (C35, C49),
    /// never with `socket`.
    pub host: Option<String>,
    pub json: bool,
    pub command: Command,
}

#[derive(Debug, PartialEq, Eq)]
pub enum Command {
    /// Usage, for one method or all of them.
    Help(Option<&'static Method>),
    /// The catalog, for discovery at run time.
    Api(Api),
    /// Requests sent in order on one connection, stopping at the first
    /// failure; `+` separates them on the command line.
    Calls(Vec<Call>),
    /// The events after `after` (or from now), then the cursor to resume
    /// from; with `follow`, every later event until the server goes away.
    Events { after: Option<Cursor>, follow: bool },
    /// Runs a configured action with its arguments.
    Do {
        action: String,
        args: Vec<(String, String)>,
    },
    /// Works with the config file.
    Config(ConfigCommand),
    /// Lists, adds or removes saved hosts (C35).
    Hosts(HostsCommand),
    /// `hosts pull`: merges another machine's saved hosts into this
    /// one's (C49b).
    Pull(Sync),
    /// Checks a host for `rusty-mux --ssh`, or prints its config (C48).
    Ssh(SshCommand),
    /// Checks, installs or removes our terminfo entry on an SSH host (C47).
    Terminfo(Terminfo),
    /// Follows events from `after` (or from now) and runs the config's
    /// rules for each, until the server goes away.
    Watch { after: Option<Cursor> },
    /// Waits for a block's program to end, or for an event.
    Wait(Wait),
    /// Starts a program in a new block, and with `wait` waits for it.
    Run(Run),
    /// Installs, checks or removes agent setup, or runs its hook.
    Agent(AgentCommand),
    /// Joins stdin and stdout to the socket, for a client that runs this
    /// over SSH.
    Bridge,
    /// Holds one connection that owns the blocks it launches, driven by
    /// JSON lines on stdin (C46).
    Own,
    /// Prints a completion script for a shell.
    Completion(Shell),
    /// Stops the server cleanly and waits until it has (C36b).
    ServerStop,
}

/// `agent`: each verb but `hook` and `skill` acts on the chosen agents in
/// one scope.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum AgentCommand {
    /// With `dry_run`, reports what it would do and writes nothing.
    Setup(AgentArgs),
    /// Exits 0 only when every item is installed and current.
    Status(AgentArgs),
    Remove(AgentArgs),
    /// What the session-start hook runs; with `name_session`, it also asks
    /// the agent to name a session that still has a generated name after
    /// its task.
    Hook {
        name_session: bool,
    },
    /// Prints the guide the installed skill reads, so it always matches
    /// this ctl (C73).
    Skill,
    /// What an agent's event hooks run (`agent hook --event STATE ...`):
    /// reports the agent's status for the block it runs in (O2).
    Report(HookReport),
}

/// An agent hook's status report, field by field as
/// `terminal.report-status` takes them; the server checks them.
#[derive(Debug, Clone, PartialEq, Eq, Default)]
pub struct HookReport {
    /// `idle`, `working`, `blocked`, `done`, `error` or `clear`.
    pub state: String,
    pub id: Option<String>,
    pub kind: Option<String>,
    pub app: Option<String>,
    pub msg: Option<String>,
    /// `--tool-from-stdin`: the program running the hook fills `msg` with
    /// the `tool_name` of the hook event's JSON on stdin.
    pub tool_from_stdin: bool,
}

/// Which agents an `agent` verb acts on, and where.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct AgentArgs {
    /// Each named once, in the order [`AgentKind::ALL`] lists them.
    pub agents: Vec<AgentKind>,
    pub scope: AgentScope,
    pub dry_run: bool,
    /// The hook asks the agent to name its session after its task (C50):
    /// setup installs that hook, status checks for it, and setup without
    /// it turns it off again.
    pub name_session: bool,
}

/// A coding agent `agent setup` knows.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord)]
pub enum AgentKind {
    Claude,
    Codex,
    OpenCode,
    Pi,
}

impl AgentKind {
    pub const ALL: [Self; 4] = [Self::Claude, Self::Codex, Self::OpenCode, Self::Pi];

    /// Its name on the command line and in output.
    pub const fn name(self) -> &'static str {
        match self {
            Self::Claude => "claude",
            Self::Codex => "codex",
            Self::OpenCode => "opencode",
            Self::Pi => "pi",
        }
    }
}

/// Where an agent's files go.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum AgentScope {
    /// The agent's own config directory, for every project.
    User,
    /// The project in the current directory, for that project only.
    Project,
    /// Claude Code's config directory given by `--dir`.
    Dir(PathBuf),
}

/// What `wait` waits for, and for how long at most.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Wait {
    pub until: Until,
    pub timeout: Option<Duration>,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Until {
    /// The block's program exits, or the block closes; the target is
    /// resolved like an id flag's.
    Exit { block: String },
    /// Every block in the window or session has ended: its program exited
    /// or it closed, including blocks made there during the wait. The
    /// target is resolved like an id flag's.
    Ended { scope: Scope, target: String },
    /// An event of `kind`, one of [`rusty_mux_script::EVENTS`], whose
    /// fields have these values. A `session`, `window`, `block` or
    /// `client` value is a target.
    Event {
        kind: &'static str,
        fields: Vec<(&'static str, String)>,
    },
}

/// What holds the blocks an [`Until::Ended`] wait follows.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Scope {
    Window,
    Session,
}

/// `run`: a `create-block` request that names a program, then, with
/// `wait`, a wait for that program with an optional timeout.
#[derive(Debug, PartialEq, Eq)]
pub struct Run {
    pub create: Call,
    pub wait: Option<Option<Duration>>,
}

/// What `config` does.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum ConfigCommand {
    /// Checks a config, the given file or the usual one, without running it.
    Check(Option<PathBuf>),
    /// Lists the configured actions, bindings and rules.
    List,
}

/// `terminfo VERB DEST`: our terminfo entry on the SSH host `dest`.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Terminfo {
    pub verb: TerminfoVerb,
    /// What `ssh` gets after `--`.
    pub dest: String,
    /// Print the command and run nothing (`setup` and `remove` only).
    pub dry_run: bool,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum TerminfoVerb {
    /// Reports whether the entry is installed; reads only.
    Check,
    /// Compiles the entry into the remote `~/.terminfo`.
    Setup,
    /// Removes the entry from the remote `~/.terminfo`.
    Remove,
}

/// `ssh`: none of its verbs connects or writes.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum SshCommand {
    /// Problems and warnings for `rusty-mux --ssh DEST`.
    Check(String),
    /// As `check`, with every value it used and a fix for each finding.
    Diagnose(String),
    /// Prints a `Host` block for the ssh config; with `write`, puts it in
    /// `~/.ssh/rusty_mux/config` (pack D193).
    Setup(SshSetup),
    /// Takes `dest`'s block out of `~/.ssh/rusty_mux/config`; without
    /// `write`, says what it would do.
    Remove { dest: String, write: bool },
}

/// What `ssh setup` puts in its block; only what is given.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct SshSetup {
    pub dest: String,
    pub hostname: Option<String>,
    pub user: Option<String>,
    pub port: Option<String>,
    pub identity: Option<String>,
    /// Write the block instead of printing it.
    pub write: bool,
}

/// What `api` prints.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Api {
    /// Every method's name and summary.
    List,
    /// One method, with its parameters as a JSON Schema.
    Describe(&'static Method),
    /// Every method and command as one Markdown document for a language
    /// model.
    Llms,
}

/// A shell `completion` writes a script for.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Shell {
    Bash,
    Zsh,
    Fish,
    Nu,
    PowerShell,
}

impl FromStr for Shell {
    type Err = String;

    fn from_str(name: &str) -> Result<Self, String> {
        match name {
            "bash" => Ok(Self::Bash),
            "zsh" => Ok(Self::Zsh),
            "fish" => Ok(Self::Fish),
            "nu" => Ok(Self::Nu),
            "powershell" | "pwsh" => Ok(Self::PowerShell),
            _ => Err(format!(
                "completion: unknown shell `{name}`; expected bash, zsh, fish, nu or powershell"
            )),
        }
    }
}

/// One request with its checked arguments.
#[derive(Debug, PartialEq, Eq)]
pub struct Call {
    pub method: &'static Method,
    pub args: Vec<(&'static str, Arg)>,
}

impl Call {
    /// Whether an argument still names its entity by something other than
    /// a full id.
    pub fn has_target(&self) -> bool {
        self.args
            .iter()
            .any(|(_, a)| matches!(a, Arg::Target { .. }))
    }
}

/// An argument: a value, or a target resolved against the server's listing
/// before the request is sent.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Arg {
    Value(Value),
    /// A name, position or id suffix; see [`crate::target`].
    Target {
        entity: Entity,
        spec: String,
    },
}

/// The session, window and block a command runs in, the defaults for id
/// flags it leaves out, and its working directory, where `run` starts its
/// program.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct Defaults {
    pub session: Option<Uuid>,
    pub window: Option<Uuid>,
    pub block: Option<Uuid>,
    /// Absolute.
    pub dir: Option<PathBuf>,
}

impl Defaults {
    fn get(&self, entity: Entity) -> Option<Uuid> {
        match entity {
            Entity::Session => self.session,
            Entity::Window => self.window,
            Entity::Block => self.block,
            Entity::Client => None,
        }
    }
}

/// Parses `args` (without the program name), filling id flags left out
/// from `defaults`. The error is a one-line reason.
pub fn parse(args: &[String], defaults: &Defaults) -> Result<Invocation, String> {
    let mut socket = None;
    let mut host = None;
    let mut json = false;
    let mut rest = args;
    loop {
        match rest {
            [flag, path, tail @ ..] if flag == "--socket" => {
                socket = Some(PathBuf::from(path));
                rest = tail;
            }
            [flag, name, tail @ ..] if flag == "--host" => {
                host = Some(name.clone());
                rest = tail;
            }
            [flag, tail @ ..] if flag.starts_with("--host=") => {
                host = Some(flag["--host=".len()..].to_owned());
                rest = tail;
            }
            [flag, tail @ ..] if flag == "--json" => {
                json = true;
                rest = tail;
            }
            _ => break,
        }
    }
    if socket.is_some() && host.is_some() {
        return Err("--host and --socket name the server twice: pass one".into());
    }
    let command = command(rest, defaults)?;
    Ok(Invocation {
        socket,
        host,
        json,
        command,
    })
}

fn command(args: &[String], defaults: &Defaults) -> Result<Command, String> {
    let Some((name, rest)) = args.split_first() else {
        return Ok(Command::Help(None));
    };
    match name.as_str() {
        "help" | "--help" | "-h" => help(rest),
        "api" => api(rest),
        "events" => events(rest),
        "do" => run_action(rest),
        "config" => config(rest),
        "hosts" => match rest {
            [verb, dest, flags @ ..] if verb == "pull" => sync(dest, flags).map(Command::Pull),
            _ => hosts(rest).map(Command::Hosts),
        },
        "ssh" => ssh(rest).map(Command::Ssh),
        "terminfo" => terminfo(rest).map(Command::Terminfo),
        "watch" => watch(rest),
        "wait" => wait(rest).map(Command::Wait),
        "run" => run(rest, defaults).map(Command::Run),
        "agent" => agent(rest).map(Command::Agent),
        "bridge" if rest.is_empty() => Ok(Command::Bridge),
        "bridge" => Err("bridge takes no arguments".into()),
        "own" if rest.is_empty() => Ok(Command::Own),
        "own" => Err("own takes no arguments".into()),
        "server" => match rest {
            [verb] if verb == "stop" => Ok(Command::ServerStop),
            _ => Err("usage: server stop".into()),
        },
        "completion" => match rest {
            [shell] => shell.parse().map(Command::Completion),
            _ => Err("usage: completion bash|zsh|fish|nu|powershell".into()),
        },
        _ => args
            .split(|a| a == "+")
            .map(|args| call(args, defaults))
            .collect::<Result<_, _>>()
            .map(Command::Calls),
    }
}

fn help(args: &[String]) -> Result<Command, String> {
    match args {
        [] => Ok(Command::Help(None)),
        [name] => known(name).map(|m| Command::Help(Some(m))),
        _ => Err("help takes at most one method".into()),
    }
}

fn api(args: &[String]) -> Result<Command, String> {
    match args {
        [verb] if verb == "list" => Ok(Command::Api(Api::List)),
        [verb, name] if verb == "describe" => known(name).map(|m| Command::Api(Api::Describe(m))),
        [verb] if verb == "llms" => Ok(Command::Api(Api::Llms)),
        _ => Err("usage: api list | api describe METHOD | api llms".into()),
    }
}

fn run_action(args: &[String]) -> Result<Command, String> {
    let Some((action, rest)) = args.split_first() else {
        return Err("usage: do ACTION [--ARG VALUE]...".into());
    };
    let args = flags(action, rest, |_| false)?
        .into_iter()
        .map(|(k, v)| (k.to_owned(), v.to_owned()))
        .collect();
    Ok(Command::Do {
        action: action.clone(),
        args,
    })
}

fn config(args: &[String]) -> Result<Command, String> {
    match args {
        [verb] if verb == "check" => Ok(Command::Config(ConfigCommand::Check(None))),
        [verb, path] if verb == "check" => {
            Ok(Command::Config(ConfigCommand::Check(Some(path.into()))))
        }
        [verb] if verb == "list" => Ok(Command::Config(ConfigCommand::List)),
        _ => Err("usage: config check [PATH] | config list".into()),
    }
}

/// `hosts`: each verb works on the hosts file, reading the config's declared
/// hosts but never changing them, with no server.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum HostsCommand {
    List,
    /// Appends `address` to the host called `name`, creating it.
    Add {
        name: String,
        address: Address,
    },
    Remove(String),
}

/// A sync with the machine at `dest` (term pack D202).
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Sync {
    /// What `ssh` gets after `--`.
    pub dest: String,
    /// The sender's entry wins a name both sides have.
    pub replace: bool,
    /// Read and print the plan; write nothing.
    pub dry_run: bool,
}

const HOSTS_USAGE: &str = "usage: hosts list | hosts add NAME --socket PATH | \
     hosts add NAME --ssh DEST --socket PATH [--udp-host HOST] | hosts remove NAME | \
     hosts pull DEST [--replace] [--dry-run]";

fn hosts(args: &[String]) -> Result<HostsCommand, String> {
    match args {
        [verb] if verb == "list" => Ok(HostsCommand::List),
        [verb, name] if verb == "remove" => Ok(HostsCommand::Remove(name.clone())),
        [verb, name, rest @ ..] if verb == "add" => {
            let flags = flags("hosts add", rest, |_| false)?;
            let mut socket = None;
            let mut dest = None;
            let mut udp_host = None;
            for (flag, value) in flags {
                let slot = match flag {
                    "socket" => &mut socket,
                    "ssh" => &mut dest,
                    "udp-host" => &mut udp_host,
                    _ => return Err(format!("hosts add: unknown flag --{flag}")),
                };
                if slot.replace(value.to_owned()).is_some() {
                    return Err(format!("hosts add: --{flag} given twice"));
                }
            }
            let socket = socket.ok_or("hosts add: --socket PATH is required")?;
            let address = match (dest, udp_host) {
                (None, None) => Address::Socket { socket },
                (None, Some(_)) => return Err("hosts add: --udp-host needs --ssh".into()),
                (Some(dest), udp_host) => Address::Ssh {
                    dest,
                    socket,
                    udp_host,
                },
            };
            Ok(HostsCommand::Add {
                name: name.clone(),
                address,
            })
        }
        _ => Err(HOSTS_USAGE.into()),
    }
}

/// `DEST [--replace] [--dry-run]`, each flag at most once.
fn sync(dest: &str, flags: &[String]) -> Result<Sync, String> {
    if dest.is_empty() || dest.starts_with('-') || dest.contains(char::is_whitespace) {
        return Err(format!("hosts: `{dest}` is not an ssh destination"));
    }
    let mut sync = Sync {
        dest: dest.to_owned(),
        replace: false,
        dry_run: false,
    };
    for flag in flags {
        let slot = match flag.as_str() {
            "--replace" => &mut sync.replace,
            "--dry-run" => &mut sync.dry_run,
            _ => return Err(HOSTS_USAGE.into()),
        };
        if std::mem::replace(slot, true) {
            return Err(format!("hosts: {flag} given twice"));
        }
    }
    Ok(sync)
}

const TERMINFO_USAGE: &str = "usage: terminfo check DEST | terminfo setup|remove DEST [--dry-run]";

fn terminfo(args: &[String]) -> Result<Terminfo, String> {
    let (verb, dest, dry_run) = match args {
        [verb, dest] => (verb, dest, false),
        [verb, dest, flag] if flag == "--dry-run" && verb != "check" => (verb, dest, true),
        _ => return Err(TERMINFO_USAGE.into()),
    };
    let verb = match verb.as_str() {
        "check" => TerminfoVerb::Check,
        "setup" => TerminfoVerb::Setup,
        "remove" => TerminfoVerb::Remove,
        _ => return Err(TERMINFO_USAGE.into()),
    };
    if dest.is_empty() || dest.starts_with('-') {
        return Err(format!("terminfo: `{dest}` is not an ssh destination"));
    }
    Ok(Terminfo {
        verb,
        dest: dest.clone(),
        dry_run,
    })
}

const SSH_USAGE: &str = "usage: ssh check DEST | ssh diagnose DEST | \
     ssh setup DEST [--hostname H] [--user U] [--port N] [--identity FILE] [--write] | \
     ssh remove DEST [--write]";

fn ssh(args: &[String]) -> Result<SshCommand, String> {
    let [verb, dest, rest @ ..] = args else {
        return Err(SSH_USAGE.into());
    };
    if dest.is_empty() || dest.starts_with('-') || dest.contains(char::is_whitespace) {
        return Err(format!("ssh: `{dest}` is not an ssh destination"));
    }
    match (verb.as_str(), rest) {
        ("check", []) => Ok(SshCommand::Check(dest.clone())),
        ("diagnose", []) => Ok(SshCommand::Diagnose(dest.clone())),
        ("setup", rest) => ssh_setup(dest, rest).map(SshCommand::Setup),
        ("remove", []) => Ok(SshCommand::Remove {
            dest: dest.clone(),
            write: false,
        }),
        ("remove", [flag]) if flag == "--write" => Ok(SshCommand::Remove {
            dest: dest.clone(),
            write: true,
        }),
        _ => Err(SSH_USAGE.into()),
    }
}

fn ssh_setup(dest: &str, rest: &[String]) -> Result<SshSetup, String> {
    let mut setup = SshSetup {
        dest: dest.to_owned(),
        hostname: None,
        user: None,
        port: None,
        identity: None,
        write: false,
    };
    for (flag, value) in flags("ssh setup", rest, |f| f == "write")? {
        if flag == "write" {
            setup.write = true;
            continue;
        }
        let slot = match flag {
            "hostname" => &mut setup.hostname,
            "user" => &mut setup.user,
            "port" => &mut setup.port,
            "identity" => &mut setup.identity,
            _ => return Err(format!("ssh setup: unknown flag --{flag}")),
        };
        if value.is_empty() || value.contains(char::is_whitespace) {
            return Err(format!("ssh setup: --{flag} needs one word"));
        }
        if slot.replace(value.to_owned()).is_some() {
            return Err(format!("ssh setup: --{flag} given twice"));
        }
    }
    if setup
        .port
        .as_deref()
        .is_some_and(|p| p.parse::<u16>().map_or(true, |p| p == 0))
    {
        return Err("ssh setup: --port needs a port from 1 to 65535".into());
    }
    Ok(setup)
}

fn watch(args: &[String]) -> Result<Command, String> {
    match args {
        [] => Ok(Command::Watch { after: None }),
        [flag, cursor] if flag == "--after" => Ok(Command::Watch {
            after: Some(parse_cursor(cursor)?),
        }),
        _ => Err("usage: watch [--after EPOCH:SEQ]".into()),
    }
}

const AGENT_USAGE: &str = "usage: agent setup|status|remove [--agent claude|codex|opencode|pi|all]... \
     [--project | --dir DIR] [--dry-run] [--name-session] | agent hook [--name-session] | agent hook --event STATE [--id ID] \
     [--kind KIND] [--app APP] [--msg TEXT | --tool-from-stdin] | agent skill";

fn agent(args: &[String]) -> Result<AgentCommand, String> {
    let (verb, rest) = args.split_first().ok_or(AGENT_USAGE)?;
    let make: fn(AgentArgs) -> AgentCommand = match verb.as_str() {
        "setup" => AgentCommand::Setup,
        "status" => AgentCommand::Status,
        "remove" => AgentCommand::Remove,
        "hook" if rest.first().is_some_and(|f| f.starts_with("--event")) => {
            return hook_report(rest).map(AgentCommand::Report);
        }
        "hook" => {
            let name_session = match rest {
                [] => false,
                [flag] if flag == "--name-session" => true,
                _ => return Err(AGENT_USAGE.into()),
            };
            return Ok(AgentCommand::Hook { name_session });
        }
        "skill" if rest.is_empty() => return Ok(AgentCommand::Skill),
        _ => return Err(AGENT_USAGE.into()),
    };
    let mut agents = Vec::new();
    let mut scope = AgentScope::User;
    let mut dry_run = false;
    let mut name_session = false;
    let bare = |f: &str| matches!(f, "project" | "dry-run" | "name-session");
    for (key, text) in flags("agent", rest, bare)? {
        match (key, &scope) {
            ("agent", _) => agents.extend(agent_kinds(text)?),
            ("project", AgentScope::User) if text == "true" => scope = AgentScope::Project,
            ("dir", AgentScope::User) => scope = AgentScope::Dir(PathBuf::from(text)),
            ("dry-run", _) if text == "true" && verb != "status" => dry_run = true,
            ("name-session", _) if text == "true" => name_session = true,
            _ => return Err(AGENT_USAGE.into()),
        }
    }
    agents.sort_unstable();
    agents.dedup();
    if agents.is_empty() {
        agents.push(AgentKind::Claude);
    }
    if matches!(scope, AgentScope::Dir(_)) && agents != [AgentKind::Claude] {
        return Err(
            "agent: --dir is Claude Code's directory; use --project for other agents".into(),
        );
    }
    Ok(make(AgentArgs {
        agents,
        scope,
        dry_run,
        name_session,
    }))
}

/// `agent hook --event STATE ...`: each flag once, `--event` required.
fn hook_report(rest: &[String]) -> Result<HookReport, String> {
    let mut report = HookReport::default();
    let mut state = None;
    for (key, text) in flags("agent hook", rest, |f| f == "tool-from-stdin")? {
        if key == "tool-from-stdin" && text == "true" && !report.tool_from_stdin {
            report.tool_from_stdin = true;
            continue;
        }
        let slot = match key {
            "event" => &mut state,
            "id" => &mut report.id,
            "kind" => &mut report.kind,
            "app" => &mut report.app,
            "msg" => &mut report.msg,
            _ => return Err(AGENT_USAGE.into()),
        };
        if slot.replace(text.to_owned()).is_some() {
            return Err(format!("agent hook: --{key} given twice"));
        }
    }
    report.state = state.ok_or(AGENT_USAGE)?;
    if report.tool_from_stdin && report.msg.is_some() {
        return Err("agent hook: --msg and --tool-from-stdin both set the message".into());
    }
    Ok(report)
}

/// The agents `--agent NAME` names.
fn agent_kinds(name: &str) -> Result<Vec<AgentKind>, String> {
    if name == "all" {
        return Ok(AgentKind::ALL.to_vec());
    }
    AgentKind::ALL
        .into_iter()
        .find(|kind| kind.name() == name)
        .map(|kind| vec![kind])
        .ok_or_else(|| {
            format!("agent: unknown agent `{name}`; expected claude, codex, opencode, pi or all")
        })
}

const WAIT_USAGE: &str = "usage: wait (--block BLOCK | --window WINDOW | --session SESSION) [--timeout SECS] | wait --event KIND [--FIELD VALUE]... [--timeout SECS]";

fn wait(args: &[String]) -> Result<Wait, String> {
    let mut timeout = None;
    let mut rest = Vec::new();
    for (key, text) in flags("wait", args, |_| false)? {
        match key {
            "timeout" => timeout = Some(parse_timeout(text)?),
            _ => rest.push((key, text)),
        }
    }
    let until = match rest.iter().position(|(key, _)| *key == "event") {
        Some(at) => {
            let (_, kind) = rest.remove(at);
            event_filter(kind, &rest)?
        }
        None => match rest.as_slice() {
            [("block", block)] => Until::Exit {
                block: (*block).to_owned(),
            },
            [("window", window)] => Until::Ended {
                scope: Scope::Window,
                target: (*window).to_owned(),
            },
            [("session", session)] => Until::Ended {
                scope: Scope::Session,
                target: (*session).to_owned(),
            },
            _ => return Err(WAIT_USAGE.into()),
        },
    };
    Ok(Wait { until, timeout })
}

/// An event kind and the field values to match, checked against
/// [`rusty_mux_script::EVENTS`].
fn event_filter(kind: &str, fields: &[(&str, &str)]) -> Result<Until, String> {
    if kind == "hosts-changed" {
        return Err(
            "wait: hosts-changed is ctl's own event: follow it with events --follow or watch"
                .into(),
        );
    }
    let (kind, names) = rusty_mux_script::EVENTS
        .iter()
        .find(|(name, _)| *name == kind)
        .ok_or_else(|| format!("wait: unknown event `{kind}`"))?;
    let fields = fields
        .iter()
        .map(|(key, text)| {
            let name = names
                .iter()
                .find(|name| *name == key)
                .ok_or_else(|| format!("wait: a `{kind}` event has no field `{key}`"))?;
            Ok((*name, (*text).to_owned()))
        })
        .collect::<Result<_, String>>()?;
    Ok(Until::Event { kind, fields })
}

/// Seconds, possibly fractional.
fn parse_timeout(text: &str) -> Result<Duration, String> {
    text.parse::<f64>()
        .ok()
        .and_then(|secs| Duration::try_from_secs_f64(secs).ok())
        .ok_or_else(|| format!("`{text}` is not a timeout in seconds"))
}

const RUN_USAGE: &str = "usage: run [--window WINDOW] [--cols N] [--rows N] [--cwd DIR] \
                         [--wait [--timeout SECS]] [--] COMMAND [ARG]...";

/// `run`'s flags, up to `--` or the first argument that is not a flag,
/// then the command and its arguments, as a `create-block` request.
fn run(args: &[String], defaults: &Defaults) -> Result<Run, String> {
    let (mut wait, mut timeout, mut cwd) = (false, None, None);
    let mut create = vec!["create-block".to_owned()];
    let (mut cols, mut rows) = ("80", "24");
    let mut rest = args;
    while let Some((flag, tail)) = rest.split_first() {
        if flag == "--" {
            rest = tail;
            break;
        }
        let Some(name) = flag.strip_prefix("--") else {
            break;
        };
        if name == "wait" {
            (wait, rest) = (true, tail);
            continue;
        }
        let (value, tail) = tail
            .split_first()
            .ok_or_else(|| format!("run: --{name} needs a value"))?;
        match name {
            "window" => create.extend(["--window".to_owned(), value.clone()]),
            "cols" => cols = value,
            "rows" => rows = value,
            "cwd" => cwd = Some(PathBuf::from(value)),
            "timeout" => timeout = Some(parse_timeout(value)?),
            _ => return Err(format!("run: unknown flag `{flag}`; {RUN_USAGE}")),
        }
        rest = tail;
    }
    let Some((command, command_args)) = rest.split_first() else {
        return Err(RUN_USAGE.into());
    };
    if timeout.is_some() && !wait {
        return Err("run: --timeout needs --wait".into());
    }
    create.extend(["--cols", cols, "--rows", rows, "--command", command].map(String::from));
    for arg in command_args {
        create.extend(["--arg".to_owned(), arg.clone()]);
    }
    if let Some(dir) = start_dir(cwd, defaults)? {
        create.extend(["--cwd".to_owned(), dir]);
    }
    Ok(Run {
        create: call(&create, defaults)?,
        wait: wait.then_some(timeout),
    })
}

/// Where `run` starts its program: `cwd` against the directory ctl runs
/// in, or that directory.
fn start_dir(cwd: Option<PathBuf>, defaults: &Defaults) -> Result<Option<String>, String> {
    let dir = match (cwd, &defaults.dir) {
        (Some(cwd), _) if cwd.is_absolute() => cwd,
        (Some(cwd), Some(here)) => here.join(cwd),
        (Some(cwd), None) => {
            return Err(format!(
                "run: --cwd {} is relative and the current directory is unknown",
                cwd.display()
            ));
        }
        (None, Some(here)) => here.clone(),
        (None, None) => return Ok(None),
    };
    dir.into_os_string()
        .into_string()
        .map(Some)
        .map_err(|dir| format!("run: the directory {dir:?} is not UTF-8"))
}

fn events(mut args: &[String]) -> Result<Command, String> {
    let (mut after, mut follow) = (None, false);
    loop {
        match args {
            [] => return Ok(Command::Events { after, follow }),
            [flag, tail @ ..] if flag == "--follow" => (follow, args) = (true, tail),
            [flag, cursor, tail @ ..] if flag == "--after" => {
                after = Some(parse_cursor(cursor)?);
                args = tail;
            }
            [other, ..] => return Err(format!("events: unexpected `{other}`")),
        }
    }
}

/// `EPOCH:SEQ`, as printed after `events`.
pub fn parse_cursor(text: &str) -> Result<Cursor, String> {
    let bad = || format!("`{text}` is not a cursor (EPOCH:SEQ)");
    let (epoch, seq) = text.split_once(':').ok_or_else(bad)?;
    Ok(Cursor {
        epoch: epoch.parse().map_err(|_| bad())?,
        seq: seq.parse().map_err(|_| bad())?,
    })
}

fn known(name: &str) -> Result<&'static Method, String> {
    schema::method(name).ok_or_else(|| format!("unknown command `{name}`; try `help`"))
}

fn call(args: &[String], defaults: &Defaults) -> Result<Call, String> {
    let Some((name, rest)) = args.split_first() else {
        return Err("empty request between `+`".into());
    };
    let method = known(name)?;
    let mut values: Vec<(&'static str, Arg)> = Vec::new();
    let bare = |key: &str| {
        method
            .params
            .iter()
            .any(|p| p.name == key && p.ty == Ty::Flag)
    };
    for (key, text) in flags(name, rest, bare)? {
        let param = method
            .params
            .iter()
            .find(|p| p.name == key)
            .ok_or_else(|| reason(name, &CallError::UnknownParam(key.into())))?;
        let value = arg(param.ty, text).map_err(|e| reason(name, &e))?;
        // A repeated list flag adds to the list.
        let earlier = values.iter_mut().find(|(n, _)| *n == param.name);
        if let (Some((_, Arg::Value(Value::Words(words)))), Arg::Value(Value::Words(more))) =
            (earlier, &value)
        {
            words.extend(more.iter().cloned());
            continue;
        }
        values.push((param.name, value));
    }
    for param in method.params {
        let given = values.iter().any(|(n, _)| *n == param.name);
        if let (false, Ty::Id(entity)) = (given, param.ty)
            && let Some(id) = defaults.get(entity)
        {
            values.push((param.name, Arg::Value(Value::Id(id))));
        }
    }
    // Validates now, so a bad call fails before anything is sent; a target
    // stands in as the nil id until it is resolved.
    let checked: Vec<(&str, Value)> = values
        .iter()
        .map(|(n, a)| match a {
            Arg::Value(v) => (*n, v.clone()),
            Arg::Target { .. } => (*n, Value::Id(Uuid::nil())),
        })
        .collect();
    method.request(0, &checked).map_err(|e| reason(name, &e))?;
    Ok(Call {
        method,
        args: values,
    })
}

/// `--flag=value` or `--flag value` pairs after `command`, in order. A
/// `--flag` alone that `bare` admits is `--flag=true`.
fn flags<'a>(
    command: &str,
    mut rest: &'a [String],
    bare: impl Fn(&str) -> bool,
) -> Result<Vec<(&'a str, &'a str)>, String> {
    let mut flags = Vec::new();
    while let Some((flag, tail)) = rest.split_first() {
        let flag = flag
            .strip_prefix("--")
            .ok_or_else(|| format!("{command}: expected a --flag, got `{flag}`"))?;
        let (key, text, tail) = match flag.split_once('=') {
            Some((key, text)) => (key, text, tail),
            None if bare(flag) => (flag, "true", tail),
            None => {
                let (text, tail) = tail
                    .split_first()
                    .ok_or_else(|| format!("{command}: --{flag} needs a value"))?;
                (flag, text.as_str(), tail)
            }
        };
        flags.push((key, text));
        rest = tail;
    }
    Ok(flags)
}

/// A flag's value: an id flag that is not a full id is a target.
fn arg(ty: Ty, text: &str) -> Result<Arg, CallError> {
    match (ty, text.parse::<Uuid>()) {
        (Ty::Id(entity), Err(_)) if !text.is_empty() => Ok(Arg::Target {
            entity,
            spec: text.to_owned(),
        }),
        _ => ty.parse(text).map(Arg::Value),
    }
}

fn reason(method: &str, error: &CallError) -> String {
    format!("{method}: {error}")
}

/// The commands that are not catalog methods, with their help as [`usage`]
/// prints it: wrapped, each further line indented to the text.
pub(crate) const COMMANDS: &[(&str, &str)] = &[
    (
        "events",
        "Print missed events: [--after EPOCH:SEQ] [--follow]",
    ),
    ("do", "Run a configured action: do ACTION [--ARG VALUE]..."),
    (
        "config",
        "Check or list the config: config check [PATH] | config list",
    ),
    (
        "hosts",
        "List, add or remove saved hosts for --host: hosts list |\n\
         \x20                  hosts add NAME --socket PATH |\n\
         \x20                  hosts add NAME --ssh DEST --socket PATH [--udp-host HOST] |\n\
         \x20                  hosts remove NAME |\n\
         \x20                  hosts pull DEST [--replace] [--dry-run]",
    ),
    (
        "ssh",
        "Check a host for rusty-mux --ssh (connects nowhere, though ssh -G\n\
         \x20                  runs your config's Match exec), or set one up:\n\
         \x20                  ssh check DEST | ssh diagnose DEST |\n\
         \x20                  ssh setup DEST [--hostname H] [--user U] [--port N]\n\
         \x20                  [--identity FILE] [--write] | ssh remove DEST [--write];\n\
         \x20                  --write keeps hosts in ~/.ssh/rusty_mux/config, read by one\n\
         \x20                  Include line in ~/.ssh/config; check exits 1 on a problem",
    ),
    (
        "terminfo",
        "Check, install or remove rusty_term's terminfo entry in ~/.terminfo\n\
         \x20                  on an SSH host: terminfo check DEST |\n\
         \x20                  terminfo setup|remove DEST [--dry-run]",
    ),
    (
        "watch",
        "Run the config's rules on events: watch [--after EPOCH:SEQ]",
    ),
    (
        "wait",
        "Wait for a block's program to exit, exiting with its status;\n\
         \x20                  for every block in a window or session to end, exiting with\n\
         \x20                  the last failure; or for an event whose fields match:\n\
         \x20                  wait --block BLOCK | --window WINDOW | --session SESSION |\n\
         \x20                  wait --event KIND [--FIELD VALUE]...; [--timeout SECS] exits 124",
    ),
    (
        "run",
        "Start a program in a new block; with --wait, exit with its status:\n\
         \x20                  run [--window W] [--cols N] [--rows N] [--cwd DIR] [--wait\n\
         \x20                  [--timeout SECS]] [--] COMMAND [ARG]...",
    ),
    (
        "agent",
        "Set up coding agents to use rusty_mux: a skill and a session-start\n\
         \x20                  hook: agent setup|status|remove\n\
         \x20                  [--agent claude|codex|opencode|pi|all]... [--project | --dir DIR]\n\
         \x20                  [--dry-run] [--name-session];\n\
         \x20                  Claude Code when no --agent; status exits 0 only when every\n\
         \x20                  item is current; --name-session has the agent name a session\n\
         \x20                  that still has a generated name after its task;\n\
         \x20                  agent skill prints the guide the installed skill reads;\n\
         \x20                  agent hook --event STATE [--id ID] [--kind KIND] [--app APP]\n\
         \x20                  [--msg TEXT | --tool-from-stdin] reports an agent's status for\n\
         \x20                  its block, silently; --tool-from-stdin takes the tool name\n\
         \x20                  from the hook event's JSON on stdin",
    ),
    (
        "bridge",
        "Join stdin and stdout to the socket, for a client that runs\n\
         \x20                  `ssh HOST rusty-mux-ctl bridge`",
    ),
    (
        "own",
        "Hold one connection that owns every block it launches: JSON\n\
         \x20                  requests on stdin (launch, close), one JSON answer or\n\
         \x20                  owner event per line on stdout; ends at the end of stdin",
    ),
    (
        "server",
        "Stop the server cleanly, as SIGTERM or Ctrl+C does, and wait until\n\
         \x20                  it has: server stop; exits 4 when no server answers",
    ),
    (
        "api",
        "List methods, describe one as JSON, or print the whole API as\n\
         \x20                  Markdown for a language model: api list | api describe METHOD |\n\
         \x20                  api llms",
    ),
    ("help", "Show a command's flags: help COMMAND"),
    (
        "completion",
        "Print a shell completion script: completion bash|zsh|fish|nu|powershell",
    ),
];

/// The usage text: every command, or one method's flags.
pub fn usage(method: Option<&Method>) -> String {
    let Some(m) = method else {
        let mut text = String::from(
            "usage: rusty-mux-ctl [--socket PATH | --host NAME] [--json] COMMAND [--FLAG VALUE]... [+ COMMAND ...]\n\
             The socket defaults to $RUSTY_MUX_SOCKET, else the config's home host,\n\
             else the default socket ($XDG_RUNTIME_DIR/rusty_mux/default.sock).\n\
             --host tries a host's addresses in order (see hosts). An id flag also takes a name, a\n\
             position or an id suffix, optionally as a path SESSION/WINDOW/BLOCK, and\n\
             defaults to $RUSTY_MUX_SESSION, $RUSTY_MUX_WINDOW or $RUSTY_MUX_BLOCK.\n\
             Actions come from $RUSTY_MUX_CONFIG, or rusty_mux/config.json in the\n\
             config directory.\n\ncommands:\n",
        );
        for m in schema::CATALOG {
            text.push_str(&format!("  {:<16} {}\n", m.name, m.doc));
        }
        for (name, help) in COMMANDS {
            text.push_str(&format!("  {name:<16} {help}\n"));
        }
        return text;
    };
    let mut text = format!("usage: {}\n{}\n", synopsis(m), m.doc);
    for p in m.params {
        text.push_str(&format!("  --{:<10} {}\n", p.name, p.doc));
    }
    text
}

/// `m` as a command line: `rusty-mux-ctl NAME` and its flags, optional
/// ones in brackets.
pub(crate) fn synopsis(m: &Method) -> String {
    let mut text = format!("rusty-mux-ctl {}", m.name);
    for p in m.params {
        text.push_str(&match (p.presence, p.ty) {
            (Presence::Optional { .. }, Ty::Flag) => format!(" [--{}]", p.name),
            (Presence::Required, _) => format!(" --{} <{}>", p.name, p.ty),
            (Presence::Optional { .. } | Presence::Defaulted { .. }, _) => {
                format!(" [--{} <{}>]", p.name, p.ty)
            }
        });
    }
    text
}

#[cfg(test)]
mod tests;
