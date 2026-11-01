//! `agent setup|status|remove`: the rusty-mux skill and a session-start
//! hook for Claude Code, Codex, OpenCode and Pi, for every project (each
//! agent's own config directory) or for one (`--project`, under the current
//! directory). `agent hook` is what the hooks run: inside a block it tells
//! the agent where it is. Paths per agent: [`layout`].
//!
//! Claude Code and Codex take the hook in a JSON settings file, which setup
//! edits, keeping everything else in it. Before its first edit of an
//! existing file it keeps the file's bytes in `FILE.rusty-mux.bak`;
//! `remove` puts them back when nothing else changed since, so removal
//! leaves the file as it was. OpenCode and Pi take the hook as a file of
//! code, which setup writes whole ([`scripts`]). Nothing is written until
//! every file of every chosen agent has been checked, a file setup did not
//! write is never replaced, and `--dry-run` stops after the check. Codex,
//! OpenCode and Pi share one skill file; `remove` keeps it while another of
//! them still has its hook. Placement rules:
//! `docs/projects/mux/AGENT-PLACEMENT.md`.

mod layout;
mod scripts;
mod settings;

use std::fs;
use std::io;
use std::path::{Path, PathBuf};

use rusty_json::{Map, Value};

use crate::cli::{AgentArgs, AgentCommand, AgentKind, AgentScope, Defaults};
use crate::{Env, Exit};
use layout::{Hook, Layout};

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
    /// A file `status` could not read, or a settings file it cannot edit;
    /// setup and remove refuse it with the reason.
    Unreadable,
}

/// What setup or remove did, or with `--dry-run` would do, to one file.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum Done {
    Installed,
    Updated,
    Unchanged,
    Removed,
    Absent,
    /// A foreign file, left alone.
    Kept,
    /// The shared skill, left for another agent whose hook is installed.
    Shared,
}

/// One line of output: an agent, a part, what it is or what was done, and
/// its file.
struct Line {
    agent: AgentKind,
    part: &'static str,
    word: &'static str,
    path: PathBuf,
}

/// Runs an `agent` command, printing one line per file to `out`.
pub(crate) fn run(
    command: &AgentCommand,
    env: &Env,
    json: bool,
    out: &mut dyn io::Write,
) -> Result<Exit, String> {
    let (args, lines, exit) = match command {
        AgentCommand::Hook => {
            let text = hook_text(&env.here, env.ctl.as_deref()).unwrap_or_default();
            return print(out, &text);
        }
        AgentCommand::Setup(args) => (args, setup(&layouts(args, env)?, args.dry_run)?, Exit::Ok),
        AgentCommand::Remove(args) => {
            let others = layouts(&everyone(args), env)?;
            (
                args,
                remove(&layouts(args, env)?, &others, args.dry_run)?,
                Exit::Ok,
            )
        }
        AgentCommand::Status(args) => {
            let lines = status(&layouts(args, env)?);
            let current = lines.iter().all(|l| l.word == state_word(State::Installed));
            (args, lines, if current { Exit::Ok } else { Exit::Failed })
        }
    };
    let mut text: Vec<String> = Vec::new();
    if args.dry_run && !json {
        text.push("dry run: nothing written".into());
    }
    text.extend(lines.iter().map(|l| render(l, json, args.dry_run)));
    print(out, &text.join("\n"))?;
    Ok(exit)
}

/// The chosen agents' layouts.
fn layouts(args: &AgentArgs, env: &Env) -> Result<Vec<Layout>, String> {
    let ctl = ctl_path(env)?;
    args.agents
        .iter()
        .map(|&agent| {
            layout::layout(
                agent,
                &args.scope,
                &env.agent_homes,
                env.here.dir.as_deref(),
                ctl,
            )
        })
        .collect()
}

/// Every agent the scope allows, to find who else uses a shared skill.
fn everyone(args: &AgentArgs) -> AgentArgs {
    let agents = match args.scope {
        AgentScope::Dir(_) => vec![AgentKind::Claude],
        AgentScope::User | AgentScope::Project => AgentKind::ALL.to_vec(),
    };
    AgentArgs {
        agents,
        ..args.clone()
    }
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

fn ctl_path(env: &Env) -> Result<&str, String> {
    let ctl = env
        .ctl
        .as_deref()
        .ok_or("cannot tell where rusty-mux-ctl is installed")?;
    ctl.to_str()
        .ok_or_else(|| format!("{}: the path is not UTF-8", ctl.display()))
}

/// Each item's state; one that cannot be read is `Unreadable`, not an error.
fn status(layouts: &[Layout]) -> Vec<Line> {
    let mut lines = Vec::new();
    for layout in layouts {
        let skill = skill_state(&layout.skill).unwrap_or(State::Unreadable);
        let hook = hook_state(&layout.hook).unwrap_or(State::Unreadable);
        lines.push(line(
            layout.agent,
            "skill",
            state_word(skill),
            &layout.skill,
        ));
        lines.push(line(
            layout.agent,
            "hook",
            state_word(hook),
            layout.hook.path(),
        ));
    }
    lines
}

/// The state of the hook `hook` describes.
fn hook_state(hook: &Hook) -> Result<State, String> {
    match hook {
        Hook::Settings { file, command, .. } => match read(file)? {
            None => Ok(State::Missing),
            Some((_, value)) => settings::state(&value, command).map_err(|e| at(file, e)),
        },
        Hook::Script { file, text } => marked_state(file, text, |t| t.starts_with(scripts::MARK)),
    }
}

/// Whether a hook of ours is there, current or not.
fn has_hook(hook: &Hook) -> Result<bool, String> {
    Ok(matches!(
        hook_state(hook)?,
        State::Installed | State::Outdated
    ))
}

/// Checks every agent's files, then writes what is missing or outdated.
fn setup(layouts: &[Layout], dry_run: bool) -> Result<Vec<Line>, String> {
    let mut plan = Vec::new();
    for layout in layouts {
        let skill = skill_state(&layout.skill)?;
        if skill == State::Foreign {
            return Err(foreign(&layout.skill));
        }
        let hook = hook_state(&layout.hook)?;
        if hook == State::Foreign {
            return Err(foreign(layout.hook.path()));
        }
        plan.push((layout, skill, hook));
    }
    let mut lines = Vec::new();
    for (layout, skill, hook) in plan {
        // A skill shared with an agent earlier in the plan is written once.
        if skill != State::Installed && !dry_run && skill_state(&layout.skill)? != State::Installed
        {
            write_new(&layout.skill, SKILL)?;
        }
        if hook != State::Installed && !dry_run {
            install_hook(&layout.hook, hook)?;
        }
        lines.push(line(
            layout.agent,
            "skill",
            done_word(set_done(skill)),
            &layout.skill,
        ));
        lines.push(line(
            layout.agent,
            "hook",
            done_word(set_done(hook)),
            layout.hook.path(),
        ));
    }
    Ok(lines)
}

fn foreign(path: &Path) -> String {
    format!(
        "{}: not written by agent setup; move it away first",
        path.display()
    )
}

fn install_hook(hook: &Hook, state: State) -> Result<(), String> {
    match hook {
        Hook::Script { file, text } => write_new(file, text),
        Hook::Settings {
            file,
            backup,
            command,
        } => {
            let value = match read(file)? {
                Some((text, value)) => {
                    if state == State::Missing && !exists(backup)? {
                        let protection = permissions(file)?;
                        create(backup, &text, protection.as_ref()).map_err(|e| at(backup, e))?;
                    }
                    value
                }
                None => Value::Object(Map::new()),
            };
            let value = settings::with_hook(value, command).map_err(|e| at(file, e))?;
            write_new(file, &pretty(&value))
        }
    }
}

/// Writes `text` to `path`, making its directory first.
fn write_new(path: &Path, text: &str) -> Result<(), String> {
    if let Some(parent) = path.parent() {
        fs::create_dir_all(parent).map_err(|e| at(parent, e))?;
    }
    write(path, text)
}

fn set_done(state: State) -> Done {
    match state {
        State::Installed => Done::Unchanged,
        State::Outdated => Done::Updated,
        State::Missing | State::Foreign | State::Unreadable => Done::Installed,
    }
}

/// Deletes our skills (and their directories once empty) and our hooks,
/// restoring backed-up settings when nothing else changed since. A skill
/// another agent of `everyone` still uses is kept.
fn remove(layouts: &[Layout], everyone: &[Layout], dry_run: bool) -> Result<Vec<Line>, String> {
    let mut plan = Vec::new();
    for layout in layouts {
        let skill = skill_state(&layout.skill)?;
        let hook = has_hook(&layout.hook)?;
        let mut shared = false;
        for other in everyone.iter().filter(|o| layout.shares_skill(o)) {
            let leaving = layouts.iter().any(|l| l.agent == other.agent);
            shared |= !leaving && has_hook(&other.hook)?;
        }
        plan.push((layout, skill, hook, shared));
    }
    let mut lines = Vec::new();
    for (layout, skill, hook, shared) in plan {
        let skill = match skill {
            State::Missing => Done::Absent,
            State::Foreign | State::Unreadable => Done::Kept,
            State::Installed | State::Outdated if shared => Done::Shared,
            State::Installed | State::Outdated => {
                if !dry_run {
                    remove_skill(&layout.skill)?;
                }
                Done::Removed
            }
        };
        let hook = match (hook, &layout.hook) {
            (false, _) => Done::Absent,
            (true, Hook::Script { file, .. }) => {
                if !dry_run {
                    fs::remove_file(file).map_err(|e| at(file, e))?;
                    remove_empty_dir(file.parent())?;
                }
                Done::Removed
            }
            (true, Hook::Settings { file, backup, .. }) if !dry_run => remove_hook(file, backup)?,
            (true, Hook::Settings { .. }) => Done::Removed,
        };
        lines.push(line(layout.agent, "skill", done_word(skill), &layout.skill));
        lines.push(line(
            layout.agent,
            "hook",
            done_word(hook),
            layout.hook.path(),
        ));
    }
    Ok(lines)
}

/// Deletes the skill, then its directory and `skills` if that leaves them
/// empty. A skill an earlier agent of the same command removed is gone.
fn remove_skill(path: &Path) -> Result<(), String> {
    match fs::remove_file(path) {
        Err(e) if e.kind() == io::ErrorKind::NotFound => return Ok(()),
        other => other.map_err(|e| at(path, e))?,
    }
    let dir = path.parent();
    remove_empty_dir(dir)?;
    remove_empty_dir(dir.and_then(Path::parent))
}

fn remove_hook(file: &Path, backup_path: &Path) -> Result<Done, String> {
    let backup = read(backup_path)?;
    let done = match read(file)? {
        None => Done::Absent,
        Some((_, value)) => {
            let ours = settings::has_hook(&value).map_err(|e| at(file, e))?;
            if !ours {
                Done::Absent
            } else {
                let before = backup.as_ref().map(|(_, before)| before);
                let value = settings::without_hook(value, before).map_err(|e| at(file, e))?;
                match &backup {
                    Some((text, before)) if *before == value => write(file, text)?,
                    None if value == Value::Object(Map::new()) => {
                        fs::remove_file(file).map_err(|e| at(file, e))?
                    }
                    _ => write(file, &pretty(&value))?,
                }
                Done::Removed
            }
        }
    };
    if backup.is_some() {
        fs::remove_file(backup_path).map_err(|e| at(backup_path, e))?;
    }
    Ok(done)
}

fn skill_state(path: &Path) -> Result<State, String> {
    marked_state(path, SKILL, |text| text.contains(MARK))
}

/// `Installed` when the file at `path` holds `want`, `Outdated` when it is
/// ours by `is_ours` but differs, `Foreign` otherwise.
fn marked_state(path: &Path, want: &str, is_ours: fn(&str) -> bool) -> Result<State, String> {
    match fs::read_to_string(path) {
        Ok(text) if text == want => Ok(State::Installed),
        Ok(text) if is_ours(&text) => Ok(State::Outdated),
        Ok(_) => Ok(State::Foreign),
        Err(e) if e.kind() == io::ErrorKind::NotFound => Ok(State::Missing),
        Err(e) if e.kind() == io::ErrorKind::InvalidData => Ok(State::Foreign),
        Err(e) => Err(at(path, e)),
    }
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

fn line(agent: AgentKind, part: &'static str, word: &'static str, path: &Path) -> Line {
    Line {
        agent,
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
        State::Unreadable => "unreadable",
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
        Done::Shared => "shared",
    }
}

fn render(line: &Line, json: bool, dry_run: bool) -> String {
    let path = line.path.display().to_string();
    let agent = line.agent.name();
    if !json {
        return format!("{agent:<9}{:<6} {:<10} {path}", line.part, line.word);
    }
    let mut map = Map::new();
    map.insert("agent".into(), agent.into());
    map.insert("part".into(), line.part.into());
    map.insert("state".into(), line.word.into());
    map.insert("path".into(), path.into());
    if dry_run {
        map.insert("dry_run".into(), true.into());
    }
    Value::Object(map).to_json_string()
}

#[cfg(test)]
mod tests;
