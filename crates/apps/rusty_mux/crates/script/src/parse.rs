//! Reading a config's JSON and checking it against the catalog.

use rusty_json::{Map, Value};
use rusty_mux_proto::schema::{self, Method, Param, Presence, Ty};

use crate::{
    Action, Bind, Call, Config, Defaults, EVENTS, Problem, Rule, Run, is_arg_name, placeholders,
};

/// The config in `text`, or every problem in it. A config is an object
/// with optional `actions`, `binds` and `on` arrays:
///
/// - an action is `{"name", "title"?, "args"?, "run"}`: a name of lowercase
///   letters, digits, `.`, `_` or `-` that is not a catalog method, the
///   argument names it takes, and a non-empty array of requests;
/// - a request is `{"call": METHOD, "args"?: {PARAM: VALUE}}`, each value a
///   string, a non-negative integer or a boolean; a string's `${arg}` must name a
///   declared argument (other `${...}`, such as `${HOME}`, is left alone);
///   every parameter but an id is required, since an id can come from the
///   environment, and a value without a placeholder must suit its type;
/// - a binding is `{"keys", "action", "args"?}`: chords joined by `>`, each
///   keys joined by `+` (`ctrl+b>c`), and an action given all its
///   arguments, which then make valid values, or a catalog method given its
///   parameters as a request would be;
/// - a rule is `{"event", "action", "args"?}`: one of [`EVENTS`] and what
///   it runs, as a binding, whose values may also hold the event's
///   `${field}`s; a value with one is checked once the event fills it.
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
        let Some(map) = self.object("config", root, &["actions", "binds", "on", "server"]) else {
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
        let rules = self
            .array("on", map.get("on"))
            .iter()
            .enumerate()
            .filter_map(|(i, value)| self.rule(&format!("on[{i}]"), value, &actions))
            .collect();
        let server = match map.get("server") {
            None => Defaults::default(),
            Some(value) => self.defaults("server", value),
        };
        Config {
            actions,
            binds,
            rules,
            server,
        }
    }

    fn defaults(&mut self, at: &str, value: &Value) -> Defaults {
        let Some(map) = self.object(at, value, &["command", "args", "cwd"]) else {
            return Defaults::default();
        };
        let command = map
            .get("command")
            .and_then(|c| self.string(&format!("{at}.command"), Some(c)));
        if command == Some("") {
            self.problem(&format!("{at}.command"), "must not be empty");
        }
        let args_at = format!("{at}.args");
        let args: Vec<String> = self
            .array(&args_at, map.get("args"))
            .iter()
            .enumerate()
            .filter_map(|(i, arg)| self.string(&format!("{args_at}[{i}]"), Some(arg)))
            .map(str::to_owned)
            .collect();
        if !args.is_empty() && command.is_none() {
            self.problem(&args_at, "needs a `command`");
        }
        let cwd = map
            .get("cwd")
            .and_then(|c| self.string(&format!("{at}.cwd"), Some(c)));
        if let Some(cwd) = cwd
            && !std::path::Path::new(cwd).is_absolute()
        {
            self.problem(
                &format!("{at}.cwd"),
                format!("`{cwd}` is not an absolute path"),
            );
        }
        let texts = [("command", command), ("cwd", cwd)]
            .into_iter()
            .chain(args.iter().map(|a| ("args", Some(a.as_str()))));
        let nul: Vec<&str> = texts
            .filter(|(_, text)| text.is_some_and(|t| t.contains('\0')))
            .map(|(key, _)| key)
            .collect();
        for key in nul {
            self.problem(&format!("{at}.{key}"), "must not hold a NUL");
        }
        Defaults {
            command: command.map(str::to_owned),
            args,
            cwd: cwd.map(str::to_owned),
        }
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
            let unknown: Vec<&str> = placeholders(&value)
                .filter(|p| !args.iter().any(|a| a == p))
                .collect();
            for name in &unknown {
                self.problem(
                    &at,
                    format!("`${{{name}}}` is not an argument of this action"),
                );
            }
            // A placeholder's value is checked once a binding or `do` gives it.
            if !has_placeholder(&value) {
                self.value(&at, param, &value);
            }
            params.push((param.name, value));
        }
        self.required(at, method, params.iter().map(|(name, _)| *name));
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
        self.target(at, name, &args, actions, false);
        Some(Bind {
            keys: keys.to_owned(),
            action: name.to_owned(),
            args,
        })
    }

    /// A problem if `text` cannot be a value of `param`, as `rusty-mux-ctl`
    /// would refuse it. An id may be a name or other target, which only
    /// the server resolves.
    fn value(&mut self, at: &str, param: &Param, text: &str) {
        let refused = match param.ty {
            Ty::Id(_) if !text.is_empty() => None,
            ty => ty.parse(text).err(),
        };
        if let Some(error) = refused {
            self.problem(at, format!("`{}`: {error}", param.name));
        }
    }

    /// A problem for each required parameter of `method` that is not
    /// `given`, but for ids, which may come from the environment.
    fn required<'n>(&mut self, at: &str, method: &Method, given: impl Iterator<Item = &'n str>) {
        let given: Vec<&str> = given.collect();
        for missing in method.params.iter().filter(|p| {
            p.presence == Presence::Required
                && !matches!(p.ty, Ty::Id(_))
                && !given.contains(&p.name)
        }) {
            self.problem(
                at,
                format!("`{}` needs parameter `{}`", method.name, missing.name),
            );
        }
    }

    /// Checks the values that `action`, expanded by a binding or rule into
    /// `calls`, takes from its placeholders; the action checked the rest.
    /// With `deferred`, a value still holding a placeholder is skipped.
    fn bound_values(&mut self, at: &str, action: &Action, calls: &[Call], deferred: bool) {
        for (template, call) in action.run.iter().zip(calls) {
            let Some(method) = schema::method(call.method) else {
                continue;
            };
            let filled = template.params.iter().zip(&call.params);
            for ((name, template), (_, value)) in filled {
                if !has_placeholder(template) || (deferred && has_placeholder(value)) {
                    continue;
                }
                if let Some(param) = param(method, name) {
                    self.value(at, param, value);
                }
            }
        }
    }

    fn rule(&mut self, at: &str, value: &Value, actions: &[Action]) -> Option<Rule> {
        let map = self.object(at, value, &["event", "action", "args"])?;
        let event = self.string(&format!("{at}.event"), map.get("event"));
        let name = self.string(&format!("{at}.action"), map.get("action"));
        let args = self.params(&format!("{at}.args"), map.get("args"));
        let (event, name) = (event?, name?);
        let Some(&(event, fields)) = EVENTS.iter().find(|(kind, _)| *kind == event) else {
            self.problem(&format!("{at}.event"), format!("unknown event `{event}`"));
            return None;
        };
        for (key, value) in &args {
            for unknown in placeholders(value).filter(|p| !fields.contains(p)) {
                self.problem(
                    &format!("{at}.args.{key}"),
                    format!("`${{{unknown}}}` is not a field of `{event}`"),
                );
            }
        }
        let run = self.target(at, name, &args, actions, true)?;
        Some(Rule { event, run })
    }

    /// Checks what a binding or rule runs: a catalog method, which wins
    /// since an action may not take its name, given its parameters as a
    /// request would be; or an action given exactly its arguments, which
    /// then make valid values. With `deferred`, a value that holds a
    /// placeholder is checked once it is filled instead.
    fn target(
        &mut self,
        at: &str,
        name: &str,
        args: &[(String, String)],
        actions: &[Action],
        deferred: bool,
    ) -> Option<Run> {
        let names: Vec<&str> = args.iter().map(|(k, _)| k.as_str()).collect();
        match (
            schema::method(name),
            actions.iter().find(|a| a.name == name),
        ) {
            (Some(method), _) => {
                let mut params = Vec::new();
                for (key, value) in args {
                    let Some(param) = param(method, key) else {
                        self.problem(at, format!("`{name}` has no parameter `{key}`"));
                        continue;
                    };
                    if !(deferred && has_placeholder(value)) {
                        self.value(at, param, value);
                    }
                    params.push((param.name, value.clone()));
                }
                self.required(at, method, names.iter().copied());
                Some(Run::Request(Call {
                    method: method.name,
                    params,
                }))
            }
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
                if let Ok(calls) = action.expand(args) {
                    self.bound_values(at, action, &calls, deferred);
                }
                Some(Run::Action {
                    name: name.to_owned(),
                    args: args.to_vec(),
                })
            }
            (None, None) => {
                self.problem(at, format!("unknown action `{name}`"));
                None
            }
        }
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
                        "expected a string, a non-negative integer or a boolean",
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

fn param(method: &'static Method, name: &str) -> Option<&'static Param> {
    method.params.iter().find(|p| p.name == name)
}

fn has_placeholder(text: &str) -> bool {
    placeholders(text).next().is_some()
}

/// A string as itself, a non-negative integer in decimal, a boolean as
/// `true` or `false`.
fn text(value: &Value) -> Option<String> {
    match (value.as_str(), value.as_u64(), value.as_bool()) {
        (Some(text), ..) => Some(text.to_owned()),
        (None, Some(n), _) => Some(n.to_string()),
        (None, None, Some(on)) => Some(on.to_string()),
        (None, None, None) => None,
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
