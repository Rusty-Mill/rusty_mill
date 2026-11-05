//! The hooks in Claude Code's `settings.json` or Codex's `hooks.json` (one
//! shape), as pure edits of the parsed file: everything else in it is kept
//! as it was.
//!
//! Setup writes one group per [`Entry`]: the session-start hook under
//! `hooks.SessionStart`, and a status hook under each event the agent
//! reports from ([`events`], O2). Each group's single command runs
//! `rusty-mux-ctl agent hook ...`. A hook is ours only in an exact shape
//! [`command`] writes, a quoted path to an executable named `rusty-mux-ctl`
//! then one of the argument lists setup uses, so a ctl that moved is found
//! and updated rather than added twice, and nobody else's hook is ever taken
//! for ours. The entries are installed, updated and removed as one hook.

use rusty_json::{Map, Value};

use super::State;
use super::layout::Ctl;
use crate::cli::AgentKind;

/// The session-start event, run by every agent setup writes settings for.
const SESSION_START: &str = "SessionStart";

/// One group setup writes: `command` (and on Windows, for Codex,
/// `windows`) under `hooks.<event>`.
#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) struct Entry {
    pub event: &'static str,
    pub command: String,
    pub windows: Option<String>,
}

/// An event an agent's status hook runs on, with the words after
/// `agent hook`.
type Event = (&'static str, &'static [&'static str]);

/// Claude Code's status hooks (D87). A permission prompt shows `blocked`
/// until the tool runs, fails or is denied; `PreToolUse` and
/// `PermissionRequest` name the tool from the event's stdin.
const CLAUDE: &[Event] = &[
    (
        "UserPromptSubmit",
        &["--event", "working", "--app", "claude"],
    ),
    (
        "PreToolUse",
        &["--event", "working", "--app", "claude", "--tool-from-stdin"],
    ),
    (
        "PermissionRequest",
        &[
            "--event",
            "blocked",
            "--kind",
            "permission",
            "--app",
            "claude",
            "--tool-from-stdin",
        ],
    ),
    (
        "PermissionDenied",
        &["--event", "working", "--app", "claude"],
    ),
    ("PostToolUse", &["--event", "working", "--app", "claude"]),
    (
        "PostToolUseFailure",
        &["--event", "working", "--app", "claude"],
    ),
    ("Stop", &["--event", "idle", "--app", "claude"]),
    ("StopFailure", &["--event", "error", "--app", "claude"]),
];

/// Codex's status hooks (events from `openai/codex` `codex-rs/hooks`): as
/// Claude Code's, less the failure events, plus `Interrupt`, which ends a
/// turn without `Stop`.
const CODEX: &[Event] = &[
    (
        "UserPromptSubmit",
        &["--event", "working", "--app", "codex"],
    ),
    (
        "PreToolUse",
        &["--event", "working", "--app", "codex", "--tool-from-stdin"],
    ),
    (
        "PermissionRequest",
        &[
            "--event",
            "blocked",
            "--kind",
            "permission",
            "--app",
            "codex",
            "--tool-from-stdin",
        ],
    ),
    ("PostToolUse", &["--event", "working", "--app", "codex"]),
    ("Stop", &["--event", "idle", "--app", "codex"]),
    ("Interrupt", &["--event", "idle", "--app", "codex"]),
];

/// The status hooks of `agent`, or none for an agent without settings.
fn events(agent: AgentKind) -> &'static [Event] {
    match agent {
        AgentKind::Claude => CLAUDE,
        AgentKind::Codex => CODEX,
        AgentKind::OpenCode | AgentKind::Pi => &[],
    }
}

/// Every event setup may write under, once each; they must hold arrays to
/// be edited.
fn known_events() -> Vec<&'static str> {
    let all = std::iter::once(SESSION_START).chain(CLAUDE.iter().chain(CODEX).map(|(e, _)| *e));
    let mut events = Vec::new();
    for event in all {
        if !events.contains(&event) {
            events.push(event);
        }
    }
    events
}

/// The session-start hook's words after the ctl's path, for `ctl`.
const START_ARGS: [&[&str]; 2] = [&["agent", "hook"], &["agent", "hook", "--name-session"]];

/// Every argument list a hook of ours may have, after the ctl's path.
fn ours_args() -> impl Iterator<Item = Vec<&'static str>> {
    let starts = START_ARGS.iter().map(|args| args.to_vec());
    let events = CLAUDE
        .iter()
        .chain(CODEX)
        .map(|(_, args)| status_args(args));
    starts.chain(events)
}

fn status_args(args: &[&'static str]) -> Vec<&'static str> {
    ["agent", "hook"]
        .into_iter()
        .chain(args.iter().copied())
        .collect()
}

/// The entries setup writes for `agent` running `ctl`; `windows` asks for
/// Codex's Windows commands, failing on a path they cannot carry.
pub(crate) fn entries(agent: AgentKind, ctl: Ctl<'_>, windows: bool) -> Result<Vec<Entry>, String> {
    let start = (SESSION_START, ctl.args().to_vec());
    let status = events(agent)
        .iter()
        .map(|(event, args)| (*event, status_args(args)));
    std::iter::once(start)
        .chain(status)
        .map(|(event, args)| {
            let windows = match windows && agent == AgentKind::Codex {
                true => Some(windows_line(ctl.path, &args)?),
                false => None,
            };
            Ok(Entry {
                event,
                command: posix_line(ctl.path, &args),
                windows,
            })
        })
        .collect()
}

/// The session-start hook command that runs `ctl`, quoted for the POSIX
/// shell Claude Code runs hooks in (Git Bash on Windows).
#[cfg(test)]
pub(crate) fn command(ctl: Ctl<'_>) -> String {
    posix_line(ctl.path, ctl.args())
}

fn posix_line(path: &str, args: &[&str]) -> String {
    format!("'{}' {}", path.replace('\'', r"'\''"), args.join(" "))
}

/// The key of the Windows command Codex runs instead of `command` there.
const WINDOWS: &str = "commandWindows";

/// The session-start hook's Windows command for Codex; see [`windows_line`].
#[cfg(test)]
pub(crate) fn windows_command(ctl: Ctl<'_>) -> Result<String, String> {
    windows_line(ctl.path, ctl.args())
}

/// The Windows command for Codex, which runs a hook's command with the
/// session's shell: PowerShell, `cmd.exe` or Git Bash. No quoting works in
/// all three, so the path goes unquoted, with `/` for `\` (which all three
/// take in an absolute path), and a path with any character one of them
/// would read specially is refused.
fn windows_line(path: &str, args: &[&str]) -> Result<String, String> {
    let plain_path = path.replace('\\', "/");
    let plain = |c: char| c.is_ascii_alphanumeric() || matches!(c, '/' | ':' | '.' | '-' | '_');
    if plain_path.is_empty() || !plain_path.chars().all(plain) {
        return Err(format!(
            "{path}: Codex on Windows runs hooks in the session's shell, and this path \
             needs quoting there; put rusty-mux-ctl in a directory whose path has only \
             letters, digits, `-`, `_` and `.`"
        ));
    }
    Ok(format!("{plain_path} {}", args.join(" ")))
}

fn is_ours(hook: &Value) -> bool {
    hook.get("type").and_then(Value::as_str) == Some("command")
        && hook
            .get("command")
            .and_then(Value::as_str)
            .is_some_and(is_ours_command)
}

/// Whether [`posix_line`] wrote `command` for one of [`ours_args`], for a
/// ctl at any path.
fn is_ours_command(command: &str) -> bool {
    let Some(path) = ours_args()
        .find_map(|args| command.strip_suffix(&format!(" {}", args.join(" "))))
        .and_then(unquote)
    else {
        return false;
    };
    let name = path.rsplit(['/', '\\']).next().unwrap_or_default();
    matches!(name, "rusty-mux-ctl" | "rusty-mux-ctl.exe") && name.len() < path.len()
}

/// The word that [`posix_line`]'s quoting turns into `word`, if it is one.
fn unquote(word: &str) -> Option<String> {
    let inner = word.strip_prefix('\'')?.strip_suffix('\'')?;
    let parts: Vec<&str> = inner.split(r"'\''").collect();
    match parts.iter().any(|part| part.contains('\'')) {
        true => None,
        false => Some(parts.join("'")),
    }
}

/// The `hooks` object, if any; fails on a file whose shape setup cannot
/// edit without losing something: a root or `hooks` that is not an object,
/// or an event setup writes under that is not an array.
fn hooks(settings: &Value) -> Result<Option<&Map>, String> {
    let root = settings
        .as_object()
        .ok_or("the settings are not a JSON object")?;
    let Some(hooks) = root.get("hooks") else {
        return Ok(None);
    };
    let hooks = hooks.as_object().ok_or("`hooks` is not an object")?;
    for event in known_events() {
        if hooks
            .get(event)
            .is_some_and(|groups| groups.as_array().is_none())
        {
            return Err(format!("`hooks.{event}` is not an array"));
        }
    }
    Ok(Some(hooks))
}

/// Our hooks, each with its event, in the events setup writes under.
fn ours(settings: &Value) -> Result<Vec<(&'static str, &Value)>, String> {
    let Some(hooks) = hooks(settings)? else {
        return Ok(Vec::new());
    };
    let mut found = Vec::new();
    for event in known_events() {
        let groups = hooks.get(event).and_then(Value::as_array);
        for hook in groups.into_iter().flatten().flat_map(hooks_of) {
            if is_ours(hook) {
                found.push((event, hook));
            }
        }
    }
    Ok(found)
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
    Ok(!ours(settings)?.is_empty())
}

/// Whether `settings` hold exactly `entries` among our hooks: installed
/// when each is there as written and nothing else of ours is, outdated when
/// some of ours differ, are missing or are extra.
pub(crate) fn state(settings: &Value, entries: &[Entry]) -> Result<State, String> {
    let ours = ours(settings)?;
    if ours.is_empty() {
        return Ok(State::Missing);
    }
    let matches = |entry: &Entry, (event, hook): &(&str, &Value)| {
        *event == entry.event
            && hook.get("command").and_then(Value::as_str) == Some(entry.command.as_str())
            && hook.get(WINDOWS).and_then(Value::as_str) == entry.windows.as_deref()
    };
    let exact = ours.len() == entries.len()
        && entries
            .iter()
            .all(|entry| ours.iter().filter(|hook| matches(entry, hook)).count() == 1);
    Ok(match exact {
        true => State::Installed,
        false => State::Outdated,
    })
}

/// `settings` with exactly `entries` of ours: unchanged when already so,
/// otherwise every hook of ours is taken out and each entry appended as a
/// group of its own.
pub(crate) fn with_hook(settings: Value, entries: &[Entry]) -> Result<Value, String> {
    if state(&settings, entries)? == State::Installed {
        return Ok(settings);
    }
    let before = settings.clone();
    let mut settings = without_hook(settings, Some(&before))?;
    for entry in entries {
        groups_mut(&mut settings, entry.event)?.push(group(entry));
    }
    Ok(settings)
}

/// `settings` without our hooks, dropping their groups, and an event or
/// `hooks` that this leaves empty unless `before` (the settings as they were
/// before setup) had it.
pub(crate) fn without_hook(mut settings: Value, before: Option<&Value>) -> Result<Value, String> {
    hooks(&settings)?;
    let had_hooks = before.and_then(|b| b.get("hooks"));
    let had_event = |event: &str| had_hooks.and_then(|h| h.get(event)).is_some();
    let root = settings
        .as_object_mut()
        .ok_or("the settings are not a JSON object")?;
    let Some(hooks) = root.get_mut("hooks").and_then(Value::as_object_mut) else {
        return Ok(settings);
    };
    for event in known_events() {
        let Some(groups) = hooks.get_mut(event).and_then(Value::as_array_mut) else {
            continue;
        };
        groups.retain_mut(|group| {
            let Some(hooks) = group.get_mut("hooks").and_then(Value::as_array_mut) else {
                return true;
            };
            let count = hooks.len();
            hooks.retain(|h| !is_ours(h));
            hooks.len() == count || !hooks.is_empty()
        });
        if groups.is_empty() && !had_event(event) {
            hooks.remove(event);
        }
    }
    if hooks.is_empty() && had_hooks.is_none() {
        root.remove("hooks");
    }
    Ok(settings)
}

/// The array of `event`, created if absent, after [`hooks`] passed.
fn groups_mut<'a>(settings: &'a mut Value, event: &str) -> Result<&'a mut Vec<Value>, String> {
    hooks(settings)?;
    settings
        .as_object_mut()
        .and_then(|root| {
            root.entry("hooks".into())
                .or_insert_with(|| Value::Object(Map::new()))
                .as_object_mut()
        })
        .and_then(|hooks| {
            hooks
                .entry(event.into())
                .or_insert_with(|| Value::Array(Vec::new()))
                .as_array_mut()
        })
        .ok_or_else(|| format!("cannot add to `hooks.{event}`"))
}

/// A group with no matcher, so it runs on every occurrence of its event.
fn group(entry: &Entry) -> Value {
    let mut hook = Map::new();
    hook.insert("type".into(), "command".into());
    hook.insert("command".into(), entry.command.as_str().into());
    if let Some(windows) = &entry.windows {
        hook.insert(WINDOWS.into(), windows.as_str().into());
    }
    let mut group = Map::new();
    group.insert("hooks".into(), Value::Array(vec![Value::Object(hook)]));
    Value::Object(group)
}
