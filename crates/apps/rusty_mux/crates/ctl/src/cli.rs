//! Command-line parsing, with no I/O. Every request subcommand and its flags
//! come from [`schema::CATALOG`].

use std::path::PathBuf;
use std::time::Duration;

use rusty_mux_proto::Cursor;
use rusty_mux_proto::schema::{self, CallError, Entity, Method, Presence, Ty, Value};
use rusty_uuid::Uuid;

/// A parsed command line.
#[derive(Debug, PartialEq, Eq)]
pub struct Invocation {
    pub socket: Option<PathBuf>,
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
    /// Follows events from `after` (or from now) and runs the config's
    /// rules for each, until the server goes away.
    Watch { after: Option<Cursor> },
    /// Waits for a block's program to end, or for an event.
    Wait(Wait),
    /// Starts a program in a new block, and with `wait` waits for it.
    Run(Run),
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
    /// An event of `kind`, one of [`rusty_mux_script::EVENTS`], whose
    /// fields have these values. A `session`, `window`, `block` or
    /// `client` value is a target.
    Event {
        kind: &'static str,
        fields: Vec<(&'static str, String)>,
    },
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

/// What `api` prints.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Api {
    /// Every method's name and summary.
    List,
    /// One method, with its parameters as a JSON Schema.
    Describe(&'static Method),
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
    let mut json = false;
    let mut rest = args;
    loop {
        match rest {
            [flag, path, tail @ ..] if flag == "--socket" => {
                socket = Some(PathBuf::from(path));
                rest = tail;
            }
            [flag, tail @ ..] if flag == "--json" => {
                json = true;
                rest = tail;
            }
            _ => break,
        }
    }
    let command = command(rest, defaults)?;
    Ok(Invocation {
        socket,
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
        "watch" => watch(rest),
        "wait" => wait(rest).map(Command::Wait),
        "run" => run(rest, defaults).map(Command::Run),
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
        _ => Err("usage: api list | api describe METHOD".into()),
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

fn watch(args: &[String]) -> Result<Command, String> {
    match args {
        [] => Ok(Command::Watch { after: None }),
        [flag, cursor] if flag == "--after" => Ok(Command::Watch {
            after: Some(parse_cursor(cursor)?),
        }),
        _ => Err("usage: watch [--after EPOCH:SEQ]".into()),
    }
}

const WAIT_USAGE: &str = "usage: wait --block BLOCK [--timeout SECS] | wait --event KIND [--FIELD VALUE]... [--timeout SECS]";

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
            _ => return Err(WAIT_USAGE.into()),
        },
    };
    Ok(Wait { until, timeout })
}

/// An event kind and the field values to match, checked against
/// [`rusty_mux_script::EVENTS`].
fn event_filter(kind: &str, fields: &[(&str, &str)]) -> Result<Until, String> {
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

/// The usage text: every method, or one method's flags.
pub fn usage(method: Option<&Method>) -> String {
    match method {
        None => {
            let mut text = String::from(
                "usage: rusty-mux-ctl [--socket PATH] [--json] COMMAND [--FLAG VALUE]... [+ COMMAND ...]\n\
                 The socket defaults to $RUSTY_MUX_SOCKET. An id flag also takes a name, a\n\
                 position or an id suffix, optionally as a path SESSION/WINDOW/BLOCK, and\n\
                 defaults to $RUSTY_MUX_SESSION, $RUSTY_MUX_WINDOW or $RUSTY_MUX_BLOCK.\n\
                 Actions come from $RUSTY_MUX_CONFIG, or rusty_mux/config.json in the\n\
                 config directory.\n\ncommands:\n",
            );
            for m in schema::CATALOG {
                text.push_str(&format!("  {:<16} {}\n", m.name, m.doc));
            }
            text.push_str(
                "  events           Print missed events: [--after EPOCH:SEQ] [--follow]\n",
            );
            text.push_str(
                "  do               Run a configured action: do ACTION [--ARG VALUE]...\n",
            );
            text.push_str(
                "  config           Check or list the config: config check [PATH] | config list\n",
            );
            text.push_str(
                "  watch            Run the config's rules on events: watch [--after EPOCH:SEQ]\n",
            );
            text.push_str(
                "  wait             Wait for a block's program to exit, exiting with its status,\n\
                 \x20                  or for an event whose fields match: wait --block BLOCK |\n\
                 \x20                  wait --event KIND [--FIELD VALUE]...; [--timeout SECS] exits 124\n",
            );
            text.push_str(
                "  run              Start a program in a new block; with --wait, exit with its status:\n\
                 \x20                  run [--window W] [--cols N] [--rows N] [--cwd DIR] [--wait\n\
                 \x20                  [--timeout SECS]] [--] COMMAND [ARG]...\n",
            );
            text.push_str("  api              List methods, or describe one as JSON: api list | api describe METHOD\n");
            text.push_str("  help             Show a command's flags: help COMMAND\n");
            text
        }
        Some(m) => {
            let mut text = format!("usage: rusty-mux-ctl {}", m.name);
            for p in m.params {
                text.push_str(&match (p.presence, p.ty) {
                    (Presence::Optional { .. }, Ty::Flag) => format!(" [--{}]", p.name),
                    (Presence::Required, _) => format!(" --{} <{}>", p.name, p.ty),
                    (Presence::Optional { .. }, _) => format!(" [--{} <{}>]", p.name, p.ty),
                });
            }
            text.push_str(&format!("\n{}\n", m.doc));
            for p in m.params {
                text.push_str(&format!("  --{:<10} {}\n", p.name, p.doc));
            }
            text
        }
    }
}

#[cfg(test)]
mod tests;
