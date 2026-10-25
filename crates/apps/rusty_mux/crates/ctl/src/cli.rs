//! Command-line parsing, with no I/O. Every request subcommand and its flags
//! come from [`schema::CATALOG`].

use std::path::PathBuf;

use rusty_mux_proto::Cursor;
use rusty_mux_proto::schema::{self, CallError, Method, Value};

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
    /// Requests sent in order on one connection, stopping at the first
    /// failure; `+` separates them on the command line.
    Calls(Vec<Call>),
    /// The events after `after` (or from now), then the cursor to resume
    /// from; with `follow`, every later event until the server goes away.
    Events { after: Option<Cursor>, follow: bool },
}

/// One request with its checked arguments.
#[derive(Debug, PartialEq, Eq)]
pub struct Call {
    pub method: &'static Method,
    pub args: Vec<(&'static str, Value)>,
}

/// Parses `args` (without the program name). The error is a one-line reason.
pub fn parse(args: &[String]) -> Result<Invocation, String> {
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
    let command = command(rest)?;
    Ok(Invocation {
        socket,
        json,
        command,
    })
}

fn command(args: &[String]) -> Result<Command, String> {
    let Some((name, rest)) = args.split_first() else {
        return Ok(Command::Help(None));
    };
    match name.as_str() {
        "help" | "--help" | "-h" => help(rest),
        "events" => events(rest),
        _ => args
            .split(|a| a == "+")
            .map(call)
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

fn call(args: &[String]) -> Result<Call, String> {
    let Some((name, mut rest)) = args.split_first() else {
        return Err("empty request between `+`".into());
    };
    let method = known(name)?;
    let mut values = Vec::new();
    while let Some((flag, tail)) = rest.split_first() {
        let flag = flag
            .strip_prefix("--")
            .ok_or_else(|| format!("{name}: expected a --flag, got `{flag}`"))?;
        let (key, text, tail) = match flag.split_once('=') {
            Some((key, text)) => (key, text, tail),
            None => {
                let (text, tail) = tail
                    .split_first()
                    .ok_or_else(|| format!("{name}: --{flag} needs a value"))?;
                (flag, text.as_str(), tail)
            }
        };
        let param = method
            .params
            .iter()
            .find(|p| p.name == key)
            .ok_or_else(|| reason(name, &CallError::UnknownParam(key.into())))?;
        let value = param.ty.parse(text).map_err(|e| reason(name, &e))?;
        values.push((param.name, value));
        rest = tail;
    }
    // Validates now, so a bad call fails before anything is sent.
    method.request(0, &values).map_err(|e| reason(name, &e))?;
    Ok(Call {
        method,
        args: values,
    })
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
                 The socket defaults to $RUSTY_MUX_SOCKET.\n\ncommands:\n",
            );
            for m in schema::CATALOG {
                text.push_str(&format!("  {:<14} {}\n", m.name, m.doc));
            }
            text.push_str("  events         Print missed events: [--after EPOCH:SEQ] [--follow]\n");
            text.push_str("  help           Show a command's flags: help COMMAND\n");
            text
        }
        Some(m) => {
            let mut text = format!("usage: rusty-mux-ctl {}", m.name);
            for p in m.params {
                text.push_str(&format!(" --{} <{}>", p.name, p.ty));
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
