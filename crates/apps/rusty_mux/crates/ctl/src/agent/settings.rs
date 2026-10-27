//! The session-start hook in Claude Code's `settings.json`, as pure edits of
//! the parsed file: everything else in it is kept as it was.
//!
//! The hook is one group in `hooks.SessionStart` whose single command runs
//! `rusty-mux-ctl agent hook`. A hook is ours only in the exact shape
//! [`command`] writes, a quoted path to an executable named `rusty-mux-ctl`
//! then ` agent hook`, so a ctl that moved is found and updated rather than
//! added twice, and nobody else's hook is ever taken for ours.

use rusty_json::{Map, Value};

use super::State;

const EVENT: &str = "SessionStart";
const SUFFIX: &str = " agent hook";

/// The hook command that runs the ctl at `ctl`, quoted for the POSIX shell
/// Claude Code runs hooks in (Git Bash on Windows).
pub(crate) fn command(ctl: &str) -> String {
    format!("'{}'{SUFFIX}", ctl.replace('\'', r"'\''"))
}

fn is_ours(hook: &Value) -> bool {
    hook.get("type").and_then(Value::as_str) == Some("command")
        && hook
            .get("command")
            .and_then(Value::as_str)
            .is_some_and(is_ours_command)
}

/// Whether [`command`] wrote `command`, for a ctl at any path.
fn is_ours_command(command: &str) -> bool {
    let Some(path) = command.strip_suffix(SUFFIX).and_then(unquote) else {
        return false;
    };
    let name = path.rsplit(['/', '\\']).next().unwrap_or_default();
    matches!(name, "rusty-mux-ctl" | "rusty-mux-ctl.exe") && name.len() < path.len()
}

/// The word that [`command`]'s quoting turns into `word`, if it is one.
fn unquote(word: &str) -> Option<String> {
    let inner = word.strip_prefix('\'')?.strip_suffix('\'')?;
    let parts: Vec<&str> = inner.split(r"'\''").collect();
    match parts.iter().any(|part| part.contains('\'')) {
        true => None,
        false => Some(parts.join("'")),
    }
}

/// The `SessionStart` groups, if any; fails on a file whose shape setup
/// cannot edit without losing something.
fn groups(settings: &Value) -> Result<&[Value], String> {
    let root = settings
        .as_object()
        .ok_or("the settings are not a JSON object")?;
    let Some(hooks) = root.get("hooks") else {
        return Ok(&[]);
    };
    let hooks = hooks.as_object().ok_or("`hooks` is not an object")?;
    match hooks.get(EVENT) {
        None => Ok(&[]),
        Some(groups) => groups
            .as_array()
            .map(Vec::as_slice)
            .ok_or_else(|| format!("`hooks.{EVENT}` is not an array")),
    }
}

/// The hooks of a group, skipping a group of a shape this does not know.
fn hooks_of(group: &Value) -> &[Value] {
    group
        .get("hooks")
        .and_then(Value::as_array)
        .map_or(&[], Vec::as_slice)
}

/// Whether `settings` hold a hook of ours, whatever its command.
pub(crate) fn has_hook(settings: &Value) -> Result<bool, String> {
    Ok(groups(settings)?.iter().flat_map(hooks_of).any(is_ours))
}

/// Whether `settings` hold the hook that runs `command`.
pub(crate) fn state(settings: &Value, command: &str) -> Result<State, String> {
    let ours: Vec<&Value> = groups(settings)?
        .iter()
        .flat_map(hooks_of)
        .filter(|h| is_ours(h))
        .collect();
    let current = |h: &&Value| h.get("command").and_then(Value::as_str) == Some(command);
    Ok(match ours.as_slice() {
        [] => State::Missing,
        hooks if hooks.iter().all(current) => State::Installed,
        _ => State::Outdated,
    })
}

/// `settings` with the hook running `command`: an outdated one is pointed
/// at `command`, a missing one appended as a group of its own.
pub(crate) fn with_hook(mut settings: Value, command: &str) -> Result<Value, String> {
    let had = state(&settings, command)?;
    let groups = groups_mut(&mut settings)?;
    match had {
        State::Installed | State::Foreign => {}
        State::Outdated => {
            for hook in groups.iter_mut().flat_map(hooks_of_mut) {
                if is_ours(hook) {
                    hook["command"] = command.into();
                }
            }
        }
        State::Missing => groups.push(group(command)),
    }
    Ok(settings)
}

/// `settings` without the hook, dropping the group, and `SessionStart` and
/// `hooks` where it leaves them empty unless `before` (the settings as they
/// were before setup) had them.
pub(crate) fn without_hook(mut settings: Value, before: Option<&Value>) -> Result<Value, String> {
    let had_hooks = before.and_then(|b| b.get("hooks"));
    let had_event = had_hooks.and_then(|h| h.get(EVENT)).is_some();
    let had_hooks = had_hooks.is_some();
    groups(&settings)?;
    let root = settings
        .as_object_mut()
        .ok_or("the settings are not a JSON object")?;
    let Some(hooks) = root.get_mut("hooks").and_then(Value::as_object_mut) else {
        return Ok(settings);
    };
    let Some(groups) = hooks.get_mut(EVENT).and_then(Value::as_array_mut) else {
        return Ok(settings);
    };
    groups.retain_mut(|group| {
        let Some(hooks) = group.get_mut("hooks").and_then(Value::as_array_mut) else {
            return true;
        };
        let before = hooks.len();
        hooks.retain(|h| !is_ours(h));
        hooks.len() == before || !hooks.is_empty()
    });
    if groups.is_empty() && !had_event {
        hooks.remove(EVENT);
    }
    if hooks.is_empty() && !had_hooks {
        root.remove("hooks");
    }
    Ok(settings)
}

/// The `SessionStart` array, created if absent, after [`groups`] passed.
fn groups_mut(settings: &mut Value) -> Result<&mut Vec<Value>, String> {
    groups(settings)?;
    settings
        .as_object_mut()
        .and_then(|root| {
            root.entry("hooks".into())
                .or_insert_with(|| Value::Object(Map::new()))
                .as_object_mut()
        })
        .and_then(|hooks| {
            hooks
                .entry(EVENT.into())
                .or_insert_with(|| Value::Array(Vec::new()))
                .as_array_mut()
        })
        .ok_or_else(|| format!("cannot add to `hooks.{EVENT}`"))
}

fn hooks_of_mut(group: &mut Value) -> impl Iterator<Item = &mut Value> {
    group
        .get_mut("hooks")
        .and_then(Value::as_array_mut)
        .into_iter()
        .flatten()
}

/// A group with no matcher, so it runs on every kind of session start.
fn group(command: &str) -> Value {
    let mut hook = Map::new();
    hook.insert("type".into(), "command".into());
    hook.insert("command".into(), command.into());
    let mut group = Map::new();
    group.insert("hooks".into(), Value::Array(vec![Value::Object(hook)]));
    Value::Object(group)
}
