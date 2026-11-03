//! `do` and `config`: actions and key bindings from the config file (see
//! [`rusty_mux_script`]). The file is read by each command, so an edit
//! takes effect at once here; only the server's `server` section waits
//! for `reload-config`.

use std::path::{Path, PathBuf};

use rusty_json::{Map, Value};
use rusty_mux_script::{Action, Call, Config, Rule, Run};

/// The config file, as [`rusty_mux_script::default_path`] finds it.
pub(crate) fn default_path() -> Option<PathBuf> {
    rusty_mux_script::default_path(|name| std::env::var_os(name))
}

/// The checked config at `path`; the error names the file and lists every
/// problem, one per line.
pub(crate) fn load(path: Option<&Path>) -> Result<Config, String> {
    let path = path.ok_or("no config file: set RUSTY_MUX_CONFIG")?;
    let text = std::fs::read_to_string(path).map_err(|e| format!("{}: {e}", path.display()))?;
    rusty_mux_script::parse(&text).map_err(|problems| {
        let lines: Vec<String> = problems
            .iter()
            .map(|p| format!("{}: {p}", path.display()))
            .collect();
        lines.join("\n")
    })
}

/// The command line that runs `calls` in order on one connection.
pub(crate) fn chain(calls: &[Call]) -> Vec<String> {
    let requests: Vec<Vec<String>> = calls
        .iter()
        .map(|call| {
            std::iter::once(call.method.to_owned())
                .chain(
                    call.params
                        .iter()
                        .map(|(name, value)| format!("--{name}={value}")),
                )
                .collect()
        })
        .collect();
    requests.join(&"+".to_owned())
}

/// What `config check` prints for a valid config.
pub(crate) fn summary(config: &Config) -> String {
    format!(
        "ok: {} actions, {} bindings, {} rules",
        config.actions.len(),
        config.binds.len(),
        config.rules.len()
    )
}

/// One line per action, then per binding, then per rule, or a JSON object
/// of each.
pub(crate) fn list(config: &Config, json: bool) -> String {
    let actions = config.actions.iter().map(|a| action_line(a, json));
    let binds = config.binds.iter().map(|b| {
        let args: Vec<String> = b.args.iter().map(|(k, v)| format!("--{k}={v}")).collect();
        match json {
            true => object([
                ("bind", b.keys.as_str().into()),
                ("action", b.action.as_str().into()),
                ("args", pairs(&b.args)),
            ]),
            false => format!("bind   {:<16} {} {}", b.keys, b.action, args.join(" "))
                .trim_end()
                .to_owned(),
        }
    });
    let rules = config.rules.iter().map(|r| rule_line(r, json));
    actions
        .chain(binds)
        .chain(rules)
        .collect::<Vec<_>>()
        .join("\n")
}

fn rule_line(rule: &Rule, json: bool) -> String {
    let (name, args): (&str, Vec<(String, String)>) = match &rule.run {
        Run::Action { name, args } => (name, args.clone()),
        Run::Request(call) => (
            call.method,
            call.params
                .iter()
                .map(|(k, v)| ((*k).to_owned(), v.clone()))
                .collect(),
        ),
    };
    if json {
        return object([
            ("on", rule.event.into()),
            ("action", name.into()),
            ("args", pairs(&args)),
        ]);
    }
    let flags: Vec<String> = args.iter().map(|(k, v)| format!("--{k}={v}")).collect();
    format!("on     {:<16} {name} {}", rule.event, flags.join(" "))
        .trim_end()
        .to_owned()
}

fn action_line(action: &Action, json: bool) -> String {
    let title = action.title.as_deref().unwrap_or("");
    if json {
        let args = action.args.iter().map(|a| a.as_str().into()).collect();
        return object([
            ("action", action.name.as_str().into()),
            ("title", title.into()),
            ("args", Value::Array(args)),
        ]);
    }
    let args: Vec<String> = action.args.iter().map(|a| format!("--{a}")).collect();
    format!("action {:<16} {} {}", action.name, args.join(" "), title)
        .trim_end()
        .to_owned()
}

fn pairs(args: &[(String, String)]) -> Value {
    let mut map = Map::new();
    for (key, value) in args {
        map.insert(key.clone(), value.as_str().into());
    }
    Value::Object(map)
}

fn object<'a>(fields: impl IntoIterator<Item = (&'a str, Value)>) -> String {
    let mut map = Map::new();
    for (key, value) in fields {
        map.insert(key.into(), value);
    }
    Value::Object(map).to_json_string()
}

#[cfg(test)]
mod tests {
    use super::*;

    const CONFIG: &str = r#"{
      "actions": [{"name": "scratch", "title": "Scratch", "args": ["name"], "run": [
        {"call": "create-window", "args": {"name": "${name}"}},
        {"call": "input", "args": {"data": "a + b=c"}}
      ]}],
      "binds": [{"keys": "ctrl+b>c", "action": "scratch", "args": {"name": "x"}}],
      "on": [{"event": "pwd-changed", "action": "scratch", "args": {"name": "${pwd}"}}]
    }"#;

    #[test]
    fn a_chain_joins_requests_with_plus() {
        let config = rusty_mux_script::parse(CONFIG).expect("valid");
        let calls = config.actions[0]
            .expand(&[("name".into(), "logs".into())])
            .expect("expands");
        assert_eq!(
            chain(&calls),
            [
                "create-window",
                "--name=logs",
                "+",
                "input",
                "--data=a + b=c"
            ]
        );
    }

    #[test]
    fn listing_shows_actions_then_bindings() {
        let config = rusty_mux_script::parse(CONFIG).expect("valid");
        assert_eq!(
            list(&config, false),
            "action scratch          --name Scratch\n\
             bind   ctrl+b>c         scratch --name=x\n\
             on     pwd-changed      scratch --name=${pwd}"
        );
        let json: Vec<Value> = list(&config, true)
            .lines()
            .map(|l| Value::from_json_str(l).expect("JSON"))
            .collect();
        assert_eq!(
            json[0].get("action").and_then(Value::as_str),
            Some("scratch")
        );
        assert_eq!(
            json[1]
                .get("args")
                .and_then(|a| a.get("name"))
                .and_then(Value::as_str),
            Some("x")
        );
        assert_eq!(
            json[2].get("on").and_then(Value::as_str),
            Some("pwd-changed")
        );
        assert_eq!(summary(&config), "ok: 1 actions, 1 bindings, 1 rules");
    }

    #[test]
    fn loading_names_the_file_in_every_problem() {
        let dir = std::env::temp_dir().join(format!("rusty-mux-ctl-config-{}", std::process::id()));
        std::fs::create_dir_all(&dir).expect("dir");
        let path = dir.join("config.json");
        std::fs::write(&path, r#"{"actions": [{"name": "x"}], "extra": 1}"#).expect("write");
        let problems = load(Some(&path)).expect_err("invalid");
        assert_eq!(problems.lines().count(), 2, "{problems}");
        assert!(
            problems
                .lines()
                .all(|l| l.starts_with(&path.display().to_string()))
        );
        assert!(load(Some(&dir.join("absent.json"))).is_err());
        assert_eq!(
            load(None),
            Err("no config file: set RUSTY_MUX_CONFIG".into())
        );
        std::fs::remove_dir_all(&dir).expect("cleanup");
    }
}
