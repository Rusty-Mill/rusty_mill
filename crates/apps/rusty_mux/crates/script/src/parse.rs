//! Reading a config's JSON and checking it against the catalog.

use rusty_json::{Map, Value};
use rusty_mux_proto::schema::{self, Method};

use crate::{Action, Bind, Call, Config, Problem, is_arg_name, placeholders};

/// The config in `text`, or every problem in it. A config is an object
/// with optional `actions` and `binds` arrays:
///
/// - an action is `{"name", "title"?, "args"?, "run"}`: a name of lowercase
///   letters, digits, `.`, `_` or `-` that is not a catalog method, the
///   argument names it takes, and a non-empty array of requests;
/// - a request is `{"call": METHOD, "args"?: {PARAM: VALUE}}`, each value a
///   string or a non-negative integer; a string's `${arg}` must name a
///   declared argument (other `${...}`, such as `${HOME}`, is left alone);
/// - a binding is `{"keys", "action", "args"?}`: chords joined by `>`, each
///   keys joined by `+` (`ctrl+b>c`), and an action given all its
///   arguments, or a catalog method given known parameters.
///
/// Unknown keys are problems, so a typo is not silently ignored.
pub fn parse(text: &str) -> Result<Config, Vec<Problem>> {
    let root = Value::from_json_str(text).map_err(|e| {
        vec![Problem {
            at: "config".into(),
            reason: format!("not JSON: {e}"),
        }]
    })?;
    let mut check = Check::default();
    let config = check.config(&root);
    match check.problems.is_empty() {
        true => Ok(config),
        false => Err(check.problems),
    }
}

#[derive(Default)]
struct Check {
    problems: Vec<Problem>,
}

impl Check {
    fn problem(&mut self, at: &str, reason: impl Into<String>) {
        self.problems.push(Problem {
            at: at.to_owned(),
            reason: reason.into(),
        });
    }

    fn config(&mut self, root: &Value) -> Config {
        let Some(map) = self.object("config", root, &["actions", "binds"]) else {
            return Config::default();
        };
        let mut actions: Vec<Action> = Vec::new();
        for (i, value) in self.array("actions", map.get("actions")).iter().enumerate() {
            let at = format!("actions[{i}]");
            let Some(action) = self.action(&at, value) else {
                continue;
            };
            if actions.iter().any(|a| a.name == action.name) {
                self.problem(&at, format!("action `{}` is defined twice", action.name));
            }
            actions.push(action);
        }
        let mut binds: Vec<Bind> = Vec::new();
        for (i, value) in self.array("binds", map.get("binds")).iter().enumerate() {
            let at = format!("binds[{i}]");
            let Some(bind) = self.bind(&at, value, &actions) else {
                continue;
            };
            if binds.iter().any(|b| b.keys == bind.keys) {
                self.problem(&at, format!("`{}` is bound twice", bind.keys));
            }
            binds.push(bind);
        }
        Config { actions, binds }
    }

    fn action(&mut self, at: &str, value: &Value) -> Option<Action> {
        let map = self.object(at, value, &["name", "title", "args", "run"])?;
        let name = self.string(&format!("{at}.name"), map.get("name"))?;
        if !is_action_name(name) {
            self.problem(
                at,
                format!(
                    "`{name}` is not an action name (lowercase letters, digits, `.`, `_`, `-`)"
                ),
            );
        }
        if schema::method(name).is_some() {
            self.problem(at, format!("`{name}` is a catalog method"));
        }
        let title = match map.get("title") {
            None => None,
            Some(title) => self.string(&format!("{at}.title"), Some(title)),
        };
        let args = self.arg_names(&format!("{at}.args"), map.get("args"));
        let run_at = format!("{at}.run");
        let items = self.array(&run_at, map.get("run"));
        match map.get("run") {
            None => self.problem(at, "missing `run`"),
            Some(run) if run.is_array() && items.is_empty() => {
                self.problem(&run_at, "needs at least one request");
            }
            Some(_) => {}
        }
        let run: Vec<Call> = items
            .iter()
            .enumerate()
            .filter_map(|(j, call)| self.call(&format!("{run_at}[{j}]"), call, &args))
            .collect();
        Some(Action {
            name: name.to_owned(),
            title: title.map(str::to_owned),
            args,
            run,
        })
    }

    fn arg_names(&mut self, at: &str, value: Option<&Value>) -> Vec<String> {
        let mut names: Vec<String> = Vec::new();
        for (i, name) in self.array(at, value).iter().enumerate() {
            let at = format!("{at}[{i}]");
            let Some(name) = self.string(&at, Some(name)) else {
                continue;
            };
            if !is_arg_name(name) {
                self.problem(&at, format!("`{name}` is not an argument name"));
            } else if names.iter().any(|n| n == name) {
                self.problem(&at, format!("argument `{name}` is declared twice"));
            } else {
                names.push(name.to_owned());
            }
        }
        names
    }

    fn call(&mut self, at: &str, value: &Value, args: &[String]) -> Option<Call> {
        let map = self.object(at, value, &["call", "args"])?;
        let name = self.string(&format!("{at}.call"), map.get("call"))?;
        let Some(method) = schema::method(name) else {
            self.problem(at, format!("unknown method `{name}`"));
            return None;
        };
        let mut params = Vec::new();
        for (key, value) in self.params(&format!("{at}.args"), map.get("args")) {
            let at = format!("{at}.args.{key}");
            let Some(param) = param(method, &key) else {
                self.problem(&at, format!("`{}` has no parameter `{key}`", method.name));
                continue;
            };
            for unknown in placeholders(&value).filter(|p| !args.iter().any(|a| a == p)) {
                self.problem(
                    &at,
                    format!("`${{{unknown}}}` is not an argument of this action"),
                );
            }
            params.push((param, value));
        }
        Some(Call {
            method: method.name,
            params,
        })
    }

    fn bind(&mut self, at: &str, value: &Value, actions: &[Action]) -> Option<Bind> {
        let map = self.object(at, value, &["keys", "action", "args"])?;
        let keys = self.string(&format!("{at}.keys"), map.get("keys"))?;
        if !is_key_sequence(keys) {
            self.problem(
                at,
                format!("`{keys}` is not a key sequence such as `ctrl+b>c`"),
            );
        }
        let name = self.string(&format!("{at}.action"), map.get("action"))?;
        let args = self.params(&format!("{at}.args"), map.get("args"));
        let names: Vec<&str> = args.iter().map(|(k, _)| k.as_str()).collect();
        // A catalog method wins: an action may not take its name.
        match (
            schema::method(name),
            actions.iter().find(|a| a.name == name),
        ) {
            (None, Some(action)) => {
                for missing in action.args.iter().filter(|a| !names.contains(&a.as_str())) {
                    self.problem(at, format!("`{name}` needs argument `{missing}`"));
                }
                for unknown in names
                    .iter()
                    .filter(|n| !action.args.iter().any(|a| a == *n))
                {
                    self.problem(at, format!("`{name}` has no argument `{unknown}`"));
                }
            }
            (Some(method), _) => {
                for unknown in names.iter().filter(|n| param(method, n).is_none()) {
                    self.problem(at, format!("`{name}` has no parameter `{unknown}`"));
                }
            }
            (None, None) => self.problem(at, format!("unknown action `{name}`")),
        }
        Some(Bind {
            keys: keys.to_owned(),
            action: name.to_owned(),
            args,
        })
    }

    /// An `args` object's entries as text; a missing object is empty.
    fn params(&mut self, at: &str, value: Option<&Value>) -> Vec<(String, String)> {
        let Some(value) = value else {
            return Vec::new();
        };
        let Some(map) = value.as_object() else {
            self.problem(at, "expected an object");
            return Vec::new();
        };
        map.iter()
            .filter_map(|(key, value)| match text(value) {
                Some(text) => Some((key.clone(), text)),
                None => {
                    self.problem(
                        &format!("{at}.{key}"),
                        "expected a string or a non-negative integer",
                    );
                    None
                }
            })
            .collect()
    }

    /// `value` as an object with no keys but `allowed`.
    fn object<'v>(&mut self, at: &str, value: &'v Value, allowed: &[&str]) -> Option<&'v Map> {
        let Some(map) = value.as_object() else {
            self.problem(at, "expected an object");
            return None;
        };
        for key in map.keys().filter(|k| !allowed.contains(&k.as_str())) {
            self.problem(at, format!("unknown key `{key}`"));
        }
        Some(map)
    }

    /// `value` as an array; a missing one is empty.
    fn array<'v>(&mut self, at: &str, value: Option<&'v Value>) -> &'v [Value] {
        match value.map(|v| (v, v.as_array())) {
            None => &[],
            Some((_, Some(items))) => items,
            Some((_, None)) => {
                self.problem(at, "expected an array");
                &[]
            }
        }
    }

    /// A required string.
    fn string<'v>(&mut self, at: &str, value: Option<&'v Value>) -> Option<&'v str> {
        match value.map(Value::as_str) {
            Some(Some(text)) => Some(text),
            Some(None) => {
                self.problem(at, "expected a string");
                None
            }
            None => {
                self.problem(at, "missing");
                None
            }
        }
    }
}

fn param(method: &Method, name: &str) -> Option<&'static str> {
    method
        .params
        .iter()
        .find(|p| p.name == name)
        .map(|p| p.name)
}

/// A string as itself, a non-negative integer in decimal.
fn text(value: &Value) -> Option<String> {
    match (value.as_str(), value.as_u64()) {
        (Some(text), _) => Some(text.to_owned()),
        (None, Some(n)) => Some(n.to_string()),
        (None, None) => None,
    }
}

/// Lowercase letters, digits, `.`, `_` and `-`, starting with a letter or
/// digit.
fn is_action_name(name: &str) -> bool {
    let mut bytes = name.bytes();
    bytes
        .next()
        .is_some_and(|b| b.is_ascii_lowercase() || b.is_ascii_digit())
        && bytes.all(|b| b.is_ascii_lowercase() || b.is_ascii_digit() || b"._-".contains(&b))
}

#[cfg(test)]
pub(crate) fn is_key_sequence_for_tests(keys: &str) -> bool {
    is_key_sequence(keys)
}

/// Chords joined by `>`, each non-empty keys joined by `+`, no spaces.
fn is_key_sequence(keys: &str) -> bool {
    !keys.contains(char::is_whitespace)
        && keys
            .split('>')
            .all(|chord| chord.split('+').all(|key| !key.is_empty()))
}
