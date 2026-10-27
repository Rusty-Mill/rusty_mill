//! `agent setup|status|remove`: the rusty-mux skill and a session-start
//! hook for Claude Code, in its config directory (`$CLAUDE_CONFIG_DIR`, else
//! `~/.claude`, or `--dir`, such as a project's `.claude`). `agent hook` is
//! what the hook runs: inside a block it tells the agent where it is.
//!
//! Setup writes `skills/rusty-mux/SKILL.md` and adds one hook to
//! `settings.json`, keeping everything else in it. Before its first edit of
//! an existing `settings.json` it keeps the file's bytes in
//! `settings.json.rusty-mux.bak`; `remove` puts them back when nothing else
//! changed since, so removal leaves the file as it was. Nothing is written
//! until every file has been checked, and a skill or settings file setup
//! did not write is never replaced. Placement rules:
//! `docs/projects/mux/AGENT-PLACEMENT.md`.

mod settings;

use std::fs;
use std::io;
use std::path::{Path, PathBuf};

use rusty_json::{Map, Value};

use crate::cli::{AgentCommand, Defaults};
use crate::{Env, Exit};

/// The skill, our own text.
const SKILL: &str = include_str!("agent/SKILL.md");
/// The line that marks a skill file as setup's.
const MARK: &str = "<!-- Installed by `rusty-mux-ctl agent setup`";

/// Where a skill or the hook stands.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum State {
    /// Present and as setup would write it.
    Installed,
    /// Ours, but from another version or for a ctl elsewhere.
    Outdated,
    Missing,
    /// A file by that name that setup did not write.
    Foreign,
}

/// What setup or remove did to one file.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum Done {
    Installed,
    Updated,
    Unchanged,
    Removed,
    Absent,
    /// A foreign skill, left alone.
    Kept,
}

/// One line of output: a part, what it is or what was done, and its file.
struct Line {
    part: &'static str,
    word: &'static str,
    path: PathBuf,
}

/// The files setup manages in an agent config directory.
struct Files {
    skill: PathBuf,
    settings: PathBuf,
    backup: PathBuf,
}

impl Files {
    fn in_dir(dir: &Path) -> Self {
        Self {
            skill: dir.join("skills").join("rusty-mux").join("SKILL.md"),
            settings: dir.join("settings.json"),
            backup: dir.join("settings.json.rusty-mux.bak"),
        }
    }
}

/// Runs an `agent` command, printing one line per file to `out`.
pub(crate) fn run(
    command: &AgentCommand,
    env: &Env,
    json: bool,
    out: &mut dyn io::Write,
) -> Result<Exit, String> {
    let (lines, exit) = match command {
        AgentCommand::Hook => {
            let text = hook_text(&env.here, env.ctl.as_deref()).unwrap_or_default();
            return print(out, &text);
        }
        AgentCommand::Setup(dir) => (setup(&files(dir, env)?, &hook_command(env)?)?, Exit::Ok),
        AgentCommand::Remove(dir) => (remove(&files(dir, env)?)?, Exit::Ok),
        AgentCommand::Status(dir) => {
            let files = files(dir, env)?;
            let (skill, hook) = status(&files, &hook_command(env)?)?;
            let exit = match (skill, hook) {
                (State::Installed, State::Installed) => Exit::Ok,
                _ => Exit::Failed,
            };
            let lines = vec![
                line("skill", state_word(skill), &files.skill),
                line("hook", state_word(hook), &files.settings),
            ];
            (lines, exit)
        }
    };
    let text: Vec<String> = lines.iter().map(|l| render(l, json)).collect();
    print(out, &text.join("\n"))?;
    Ok(exit)
}

/// The files in `--dir`, else in the agent's usual config directory.
fn files(dir: &Option<PathBuf>, env: &Env) -> Result<Files, String> {
    dir.as_deref()
        .or(env.agent_dir.as_deref())
        .map(Files::in_dir)
        .ok_or_else(|| {
            "no agent config directory: pass --dir, or set CLAUDE_CONFIG_DIR or HOME".into()
        })
}

fn print(out: &mut dyn io::Write, text: &str) -> Result<Exit, String> {
    if text.is_empty() {
        return Ok(Exit::Ok);
    }
    writeln!(out, "{text}").map_err(|e| format!("writing output: {e}"))?;
    Ok(Exit::Ok)
}

/// What the hook tells an agent that starts in a block; nothing elsewhere.
fn hook_text(here: &Defaults, ctl: Option<&Path>) -> Option<String> {
    let block = here.block?;
    let mut text = format!("This shell runs in rusty_mux block {block}");
    if let (Some(window), Some(session)) = (here.window, here.session) {
        text.push_str(&format!(" (window {window}, session {session})"));
    }
    text.push_str(
        ". Use the rusty-mux skill to run, watch and read programs with rusty-mux-ctl; \
         put new blocks in this window unless the user says otherwise.",
    );
    if let Some(ctl) = ctl {
        text.push_str(&format!(" rusty-mux-ctl is {}.", ctl.display()));
    }
    Some(text)
}

fn hook_command(env: &Env) -> Result<String, String> {
    let ctl = env
        .ctl
        .as_deref()
        .ok_or("cannot tell where rusty-mux-ctl is installed")?;
    let ctl = ctl
        .to_str()
        .ok_or_else(|| format!("{}: the path is not UTF-8", ctl.display()))?;
    Ok(settings::command(ctl))
}

fn status(files: &Files, command: &str) -> Result<(State, State), String> {
    let skill = skill_state(&files.skill)?;
    let hook = match read(&files.settings)? {
        None => State::Missing,
        Some((_, value)) => settings::state(&value, command).map_err(|e| at(&files.settings, e))?,
    };
    Ok((skill, hook))
}

/// Checks both files, then writes what is missing or outdated.
fn setup(files: &Files, command: &str) -> Result<Vec<Line>, String> {
    let skill = skill_state(&files.skill)?;
    if skill == State::Foreign {
        return Err(format!(
            "{}: not written by agent setup; move it away first",
            files.skill.display()
        ));
    }
    let existing = read(&files.settings)?;
    let hook = match &existing {
        None => State::Missing,
        Some((_, value)) => settings::state(value, command).map_err(|e| at(&files.settings, e))?,
    };

    if skill != State::Installed {
        if let Some(parent) = files.skill.parent() {
            fs::create_dir_all(parent).map_err(|e| at(parent, e))?;
        }
        write(&files.skill, SKILL)?;
    }
    if hook != State::Installed {
        let value = match existing {
            Some((text, value)) => {
                if hook == State::Missing && !exists(&files.backup)? {
                    let protection = permissions(&files.settings)?;
                    create(&files.backup, &text, protection.as_ref())
                        .map_err(|e| at(&files.backup, e))?;
                }
                value
            }
            None => Value::Object(Map::new()),
        };
        let value = settings::with_hook(value, command).map_err(|e| at(&files.settings, e))?;
        write(&files.settings, &pretty(&value))?;
    }
    Ok(vec![
        line("skill", done_word(set_done(skill)), &files.skill),
        line("hook", done_word(set_done(hook)), &files.settings),
    ])
}

fn set_done(state: State) -> Done {
    match state {
        State::Installed => Done::Unchanged,
        State::Outdated => Done::Updated,
        State::Missing | State::Foreign => Done::Installed,
    }
}

/// Deletes our skill (and its directory once empty) and our hook,
/// restoring the backed-up settings when nothing else changed since.
fn remove(files: &Files) -> Result<Vec<Line>, String> {
    let skill = match skill_state(&files.skill)? {
        State::Missing => Done::Absent,
        State::Foreign => Done::Kept,
        State::Installed | State::Outdated => {
            fs::remove_file(&files.skill).map_err(|e| at(&files.skill, e))?;
            // The skill's directory, then `skills` if that leaves it empty.
            let dir = files.skill.parent();
            remove_empty_dir(dir)?;
            remove_empty_dir(dir.and_then(Path::parent))?;
            Done::Removed
        }
    };
    let hook = remove_hook(files)?;
    Ok(vec![
        line("skill", done_word(skill), &files.skill),
        line("hook", done_word(hook), &files.settings),
    ])
}

fn remove_hook(files: &Files) -> Result<Done, String> {
    let backup = read(&files.backup)?;
    let done = match read(&files.settings)? {
        None => Done::Absent,
        Some((_, value)) => {
            let ours = settings::has_hook(&value).map_err(|e| at(&files.settings, e))?;
            if !ours {
                Done::Absent
            } else {
                let before = backup.as_ref().map(|(_, before)| before);
                let value =
                    settings::without_hook(value, before).map_err(|e| at(&files.settings, e))?;
                match &backup {
                    Some((text, before)) if *before == value => write(&files.settings, text)?,
                    None if value == Value::Object(Map::new()) => {
                        fs::remove_file(&files.settings).map_err(|e| at(&files.settings, e))?
                    }
                    _ => write(&files.settings, &pretty(&value))?,
                }
                Done::Removed
            }
        }
    };
    if backup.is_some() {
        fs::remove_file(&files.backup).map_err(|e| at(&files.backup, e))?;
    }
    Ok(done)
}

fn remove_empty_dir(dir: Option<&Path>) -> Result<(), String> {
    let Some(dir) = dir else { return Ok(()) };
    match fs::remove_dir(dir) {
        Err(e)
            if matches!(
                e.kind(),
                io::ErrorKind::DirectoryNotEmpty | io::ErrorKind::NotFound
            ) =>
        {
            Ok(())
        }
        other => other.map_err(|e| at(dir, e)),
    }
}

fn skill_state(path: &Path) -> Result<State, String> {
    match fs::read_to_string(path) {
        Ok(text) if text == SKILL => Ok(State::Installed),
        Ok(text) if text.contains(MARK) => Ok(State::Outdated),
        Ok(_) => Ok(State::Foreign),
        Err(e) if e.kind() == io::ErrorKind::NotFound => Ok(State::Missing),
        Err(e) if e.kind() == io::ErrorKind::InvalidData => Ok(State::Foreign),
        Err(e) => Err(at(path, e)),
    }
}

/// A JSON file's text and value, or `None` if there is none.
fn read(path: &Path) -> Result<Option<(String, Value)>, String> {
    let text = match fs::read_to_string(path) {
        Ok(text) => text,
        Err(e) if e.kind() == io::ErrorKind::NotFound => return Ok(None),
        Err(e) => return Err(at(path, e)),
    };
    let value = Value::from_json_str(&text).map_err(|e| at(path, e))?;
    Ok(Some((text, value)))
}

fn exists(path: &Path) -> Result<bool, String> {
    path.try_exists().map_err(|e| at(path, e))
}

/// Writes `text` to a new temporary file beside `path`'s target and renames
/// it over, so a crash leaves the old file or the new one. The new file is
/// made by [`create`], so private to the owner, then given the old one's
/// permissions (on Windows only the read-only flag; the DACL stays
/// owner-only). A symlink, such as a dotfile manager's, is followed rather
/// than replaced.
fn write(path: &Path, text: &str) -> Result<(), String> {
    let target = match fs::canonicalize(path) {
        Ok(target) => target,
        Err(e) if e.kind() == io::ErrorKind::NotFound => path.to_path_buf(),
        Err(e) => return Err(at(path, e)),
    };
    let protection = permissions(&target)?;
    let temp = (0..100)
        .find_map(|n| {
            let mut temp = target.clone().into_os_string();
            temp.push(format!(".rusty-mux-{}-{n}.tmp", std::process::id()));
            let temp = PathBuf::from(temp);
            match create(&temp, text, protection.as_ref()) {
                Err(e) if e.kind() == io::ErrorKind::AlreadyExists => None,
                done => Some(done.map(|()| temp.clone()).map_err(|e| at(&temp, e))),
            }
        })
        .unwrap_or_else(|| Err(at(&target, "no free temporary file name beside it")))?;
    fs::rename(&temp, &target).map_err(|e| {
        // The rename's error is the one to report; the temporary file is ours.
        let _ = fs::remove_file(&temp);
        at(&target, e)
    })
}

/// The permissions of the file at `path`, if there is one.
fn permissions(path: &Path) -> Result<Option<fs::Permissions>, String> {
    match fs::metadata(path) {
        Ok(meta) => Ok(Some(meta.permissions())),
        Err(e) if e.kind() == io::ErrorKind::NotFound => Ok(None),
        Err(e) => Err(at(path, e)),
    }
}

/// Creates `path`, failing if anything is there (a symlink included), with
/// `text` and `protection`. It is private to the owner from the start: mode
/// 0600 on Unix (until `protection` applies), and on Windows an owner-only
/// DACL set before any bytes go in, while the open handle shares no access,
/// so no other account can read a copy (failing closed if it cannot be set).
fn create(path: &Path, text: &str, protection: Option<&fs::Permissions>) -> io::Result<()> {
    let mut options = fs::OpenOptions::new();
    options.write(true).create_new(true);
    #[cfg(unix)]
    std::os::unix::fs::OpenOptionsExt::mode(&mut options, 0o600);
    #[cfg(windows)]
    std::os::windows::fs::OpenOptionsExt::share_mode(&mut options, 0);
    let mut file = options.open(path)?;
    #[cfg(windows)]
    if let Err(e) = platform_windows::fs::restrict_to_owner(path) {
        drop(file);
        // The DACL's error is the one to report; the empty file is ours.
        let _ = fs::remove_file(path);
        return Err(io::Error::other(format!("restricting access: {e}")));
    }
    let written = io::Write::write_all(&mut file, text.as_bytes())
        .and_then(|()| match protection {
            Some(protection) => file.set_permissions(protection.clone()),
            None => Ok(()),
        })
        .and_then(|()| file.sync_all());
    if written.is_err() {
        // The write's error is the one to report; the file is ours.
        let _ = fs::remove_file(path);
    }
    written
}

fn pretty(value: &Value) -> String {
    format!("{}\n", value.to_json_string_pretty())
}

fn at(path: &Path, error: impl std::fmt::Display) -> String {
    format!("{}: {error}", path.display())
}

fn line(part: &'static str, word: &'static str, path: &Path) -> Line {
    Line {
        part,
        word,
        path: path.to_path_buf(),
    }
}

const fn state_word(state: State) -> &'static str {
    match state {
        State::Installed => "installed",
        State::Outdated => "outdated",
        State::Missing => "missing",
        State::Foreign => "foreign",
    }
}

const fn done_word(done: Done) -> &'static str {
    match done {
        Done::Installed => "installed",
        Done::Updated => "updated",
        Done::Unchanged => "unchanged",
        Done::Removed => "removed",
        Done::Absent => "absent",
        Done::Kept => "kept",
    }
}

fn render(line: &Line, json: bool) -> String {
    let path = line.path.display().to_string();
    if !json {
        return format!("{:<6} {:<10} {path}", line.part, line.word);
    }
    let mut map = Map::new();
    map.insert("part".into(), line.part.into());
    map.insert("state".into(), line.word.into());
    map.insert("path".into(), path.into());
    Value::Object(map).to_json_string()
}

#[cfg(test)]
mod tests;
