//! rusty_mux actions, key bindings and event rules (C19), from a JSON
//! config.
//!
//! An action is a named chain of catalog requests, run in order on one
//! connection the way `rusty-mux-ctl a + b` runs them. Its string values
//! may hold `${arg}` placeholders for the arguments it declares. A binding
//! maps a key chord sequence to an action or a catalog method. A rule (`on`)
//! runs one when an event happens, its values taking the event's fields,
//! such as `${block}` or `${pwd}`.
//!
//! [`parse`] checks every method, parameter, placeholder and binding
//! against the catalog and reports every problem at once, so a config that
//! parses can only fail at run time on the server's answer. No I/O and no
//! language: logic beyond a chain belongs in a program driving
//! `rusty-mux-ctl` or `rusty-mux-mcp`.
//!
//! ```
//! let config = rusty_mux_script::parse(r#"{
//!   "actions": [{
//!     "name": "scratch",
//!     "args": ["name"],
//!     "run": [
//!       {"call": "create-window", "args": {"name": "${name}"}},
//!       {"call": "create-block", "args": {"window": "${name}", "cols": 80, "rows": 24}}
//!     ]
//!   }],
//!   "binds": [{"keys": "ctrl+b>c", "action": "scratch", "args": {"name": "new"}}]
//! }"#).expect("valid");
//! let calls = config.action("scratch").expect("defined").expand(&[("name".into(), "logs".into())]);
//! let calls = calls.expect("all arguments given");
//! assert!(calls[1].params.contains(&("window", "logs".to_owned())));
//! ```

use std::fmt;

mod parse;

pub use parse::parse;

/// A checked config.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct Config {
    /// In the order they are defined, names unique.
    pub actions: Vec<Action>,
    /// In the order they are defined, key sequences unique.
    pub binds: Vec<Bind>,
    /// In the order they are defined.
    pub rules: Vec<Rule>,
}

impl Config {
    /// The action called `name`.
    pub fn action(&self, name: &str) -> Option<&Action> {
        self.actions.iter().find(|a| a.name == name)
    }
}

/// A named chain of requests.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Action {
    pub name: String,
    /// A human name, for menus and listings.
    pub title: Option<String>,
    /// The arguments it takes, all required.
    pub args: Vec<String>,
    /// At least one request.
    pub run: Vec<Call>,
}

impl Action {
    /// The requests to send, each `${arg}` replaced by its value. Every
    /// declared argument must be `given`, and nothing else; the error names
    /// the first one that is missing or unknown.
    pub fn expand(&self, given: &[(String, String)]) -> Result<Vec<Call>, String> {
        if let Some((name, _)) = given.iter().find(|(n, _)| !self.args.contains(n)) {
            return Err(format!("{}: unknown argument `{name}`", self.name));
        }
        if let Some(name) = self
            .args
            .iter()
            .find(|a| !given.iter().any(|(n, _)| n == *a))
        {
            return Err(format!("{}: missing argument `{name}`", self.name));
        }
        let calls = self
            .run
            .iter()
            .map(|call| Call {
                method: call.method,
                params: call
                    .params
                    .iter()
                    .map(|(name, value)| (*name, substitute(value, given)))
                    .collect(),
            })
            .collect();
        Ok(calls)
    }
}

/// A catalog request with its parameters as text, as `--name=value` flags.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Call {
    pub method: &'static str,
    pub params: Vec<(&'static str, String)>,
}

/// A key chord sequence, such as `ctrl+b>c`, and what it runs.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Bind {
    pub keys: String,
    /// An action's name or a catalog method.
    pub action: String,
    /// Its arguments or parameters, as text.
    pub args: Vec<(String, String)>,
}

/// The event kinds a rule can name, as `rusty-mux-ctl events` prints them,
/// and the fields each gives its placeholders: the ids, numbers and text
/// of the event, and for `pwd-changed` and `title-changed` the new
/// directory or title. A program that exits has a `code` or, killed by a
/// signal, a `signal`, never both. A `layout-changed` has no `layout`
/// once its window is empty, and no `zoomed` unless a block is zoomed.
pub const EVENTS: &[(&str, &[&str])] = &[
    ("session-created", &["session"]),
    ("session-closed", &["session"]),
    ("window-created", &["window", "session"]),
    ("window-closed", &["window"]),
    ("block-created", &["block", "window", "cols", "rows"]),
    ("block-resized", &["block", "cols", "rows"]),
    ("block-closed", &["block"]),
    ("client-connected", &["client", "principal", "user"]),
    ("client-disconnected", &["client"]),
    ("attached", &["client", "block"]),
    ("detached", &["client", "block"]),
    ("lease-changed", &["block", "holder"]),
    ("block-status", &["block"]),
    ("pwd-changed", &["block", "pwd"]),
    ("title-changed", &["block", "title"]),
    ("program-exited", &["block", "code", "signal"]),
    ("bell", &["block"]),
    ("progress-changed", &["block", "state", "percent"]),
    ("notification", &["block", "title", "body"]),
    ("process-changed", &["block", "pid", "name"]),
    ("session-renamed", &["session", "name"]),
    ("window-renamed", &["window", "name"]),
    ("block-renamed", &["block", "name"]),
    ("client-renamed", &["client", "name"]),
    ("layout-changed", &["window", "layout", "zoomed"]),
    (
        "client-connection-changed",
        &["client", "principal", "user", "transport"],
    ),
];

/// What to run when an event of one kind happens.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Rule {
    /// One of [`EVENTS`].
    pub event: &'static str,
    pub run: Run,
}

/// What a rule runs; its values may hold the event's `${field}`s.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Run {
    /// An action of the config, given all its arguments.
    Action {
        name: String,
        args: Vec<(String, String)>,
    },
    /// One catalog request.
    Request(Call),
}

impl Rule {
    /// The requests to send for an event with `fields`: each `${field}`
    /// replaced in one pass, then the action expanded. A field the event
    /// lacks, such as the `holder` of a lease nobody holds, stays as it is.
    pub fn calls(&self, config: &Config, fields: &[(String, String)]) -> Result<Vec<Call>, String> {
        match &self.run {
            Run::Request(call) => Ok(vec![Call {
                method: call.method,
                params: fill(&call.params, fields),
            }]),
            Run::Action { name, args } => config
                .action(name)
                .ok_or_else(|| format!("no action `{name}`"))?
                .expand(&fill(args, fields)),
        }
    }
}

fn fill<K: Clone>(params: &[(K, String)], fields: &[(String, String)]) -> Vec<(K, String)> {
    params
        .iter()
        .map(|(name, value)| (name.clone(), substitute(value, fields)))
        .collect()
}

/// Something wrong in a config, and where.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Problem {
    /// A path into the config, such as `actions[0].run[1]`.
    pub at: String,
    pub reason: String,
}

impl fmt::Display for Problem {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "{}: {}", self.at, self.reason)
    }
}

/// `template` with each `${name}` of a `given` argument replaced by its
/// value, in one pass so a value is never itself expanded; any other
/// `${...}`, such as a shell's `${HOME}`, stays as it is.
fn substitute(template: &str, given: &[(String, String)]) -> String {
    let mut out = String::with_capacity(template.len());
    let mut rest = template;
    while let Some(start) = rest.find("${") {
        out.push_str(&rest[..start]);
        let after = &rest[start + 2..];
        let found = after.find('}').and_then(|end| {
            given
                .iter()
                .find(|(name, _)| *name == after[..end])
                .map(|(_, value)| (end, value))
        });
        match found {
            Some((end, value)) => {
                out.push_str(value);
                rest = &after[end + 1..];
            }
            None => {
                out.push_str("${");
                rest = after;
            }
        }
    }
    out.push_str(rest);
    out
}

/// The names in `text`'s `${name}` placeholders that could be arguments:
/// a lowercase letter, then lowercase letters, digits, `_` or `-`.
fn placeholders(text: &str) -> impl Iterator<Item = &str> {
    text.split("${")
        .skip(1)
        .filter_map(|part| part.split_once('}').map(|(name, _)| name))
        .filter(|name| is_arg_name(name))
}

/// An argument name: a lowercase letter, then lowercase letters, digits,
/// `_` or `-`.
fn is_arg_name(name: &str) -> bool {
    let mut bytes = name.bytes();
    bytes.next().is_some_and(|b| b.is_ascii_lowercase())
        && bytes.all(|b| b.is_ascii_lowercase() || b.is_ascii_digit() || b == b'_' || b == b'-')
}

#[cfg(test)]
mod tests;
