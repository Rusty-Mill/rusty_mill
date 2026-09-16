//! The Gemini CLI adapter.
//!
//! Unlike [`crate::claude_code`] and [`crate::codex`], `needs_input`'s
//! patterns and the hook mechanism's existence are **not** independently
//! measured by running a real `gemini` session on this machine -- there
//! is no `GEMINI_API_KEY` (or any other auth method) configured here, and
//! `gemini` hard-refuses to start at all without one, before it ever
//! reaches an interactive screen or fires a single hook (confirmed: a
//! `SessionStart` hook installed in a scratch repo never fired when
//! `gemini` was run there unauthenticated). That is a missing
//! prerequisite, not a shortcut taken.
//!
//! What this adapter is built from instead: `gemini`'s own shipped,
//! unminified-string bundle (`@google/gemini-cli`'s `bundle/*.js` --
//! identifiers are mangled by the bundler, but string literals are not),
//! plus the hook mechanism's own official reference docs
//! (<https://geminicli.com/docs/hooks/reference/>,
//! <https://geminicli.com/docs/hooks/writing-hooks/>). This is arguably
//! *more* precise than eyeballing a live terminal for the screen-text
//! patterns below -- it is the literal string constants the UI renders,
//! not a transcription of what happened to appear on one captured
//! screen -- but it is source inspection, not a live-fired hook, and
//! [`has_verified_hooks`] says so honestly rather than folding this into
//! "verified" the way Claude Code's and Codex's hooks are.
//!
//! **Fork is the exception**: [`fork_args`](GeminiCli::fork_args)'s own
//! discovery step (locating the source session's chat-history file) is
//! live-confirmed against a real `gemini` bundle and real files it wrote
//! on disk -- see that method's own docs and issue #15.

use sessionmgr_core::ports::{AgentAdapterPort, AgentSignal, ForkSource, HookOutcome};

pub struct GeminiCli;

/// Gemini's own busy indicator, checked first and unconditionally, same
/// convention as [`crate::claude_code::BUSY`]. Sourced from
/// `interactiveCli-*.js`'s `cancelAndTimerContent`: rendered as
/// `(esc to cancel, <n>s)` exactly while `streamingState === "responding"`
/// -- i.e. while the model is actively generating. Note this is a
/// *different* phrase from Claude Code's `"esc to interrupt"`; the two
/// CLIs do not share this particular string.
const BUSY: &str = "esc to cancel";

/// Real prompt/dialog/footer text extracted from `gemini-cli`'s own
/// bundled source, each requiring the user to answer before anything
/// continues -- or, for the idle marker, indicating there is nothing
/// left to wait for and the CLI is back at its main prompt:
///
/// - `"Do you trust the files in this folder?"` -- the workspace-trust
///   gate's dialog title (`interactiveCli-*.js`'s `RadioButtonSelect`
///   trust prompt).
/// - `"Do you want to proceed?"` -- the tool-confirmation dialog's
///   `"info"`-type question text.
/// - `"Allow execution of"` -- the tool-confirmation dialog's general and
///   MCP-tool question text (`"Allow execution of <tool>?"` /
///   `"Allow execution of MCP tool \"<tool>\" from server..."` both
///   contain this as a prefix).
/// - `"You must select an auth method to proceed"` -- genuinely a state
///   only the user can resolve, even though sessionmgr cannot help
///   resolve it either.
/// - `"? for shortcuts"` -- the ordinary idle marker, shown once the
///   input buffer is empty and nothing is streaming. Textually identical
///   to Claude Code's own idle marker -- not assumed to be a
///   coincidence, but not investigated further; both are real, extracted
///   strings either way.
const NEEDS_INPUT_MARKERS: &[&str] = &[
    "Do you trust the files in this folder?",
    "Do you want to proceed?",
    "Allow execution of",
    "You must select an auth method to proceed",
    "? for shortcuts",
];

/// Events this adapter installs a hook for, and what each means.
///
/// All three are real, documented Gemini CLI events
/// (<https://geminicli.com/docs/hooks/reference/>), not guessed. There is
/// no Gemini analog to Claude Code's/Codex's `SubagentStop` -- Gemini
/// CLI's own hook event list has no sub-agent-finished event at all, so
/// this adapter has nothing to map to [`HookOutcome::Notify`].
///
/// - `SessionStart`: fires on a fresh interactive start (`source:
///   "startup"`, matched exactly -- see `hook_config`'s own comment on
///   why the config narrows to it). Maps to [`HookOutcome::Ignore`], same
///   as the other two adapters: a session starting is not a
///   needs-input signal.
/// - `Notification`: fires for a system alert, documented today as
///   firing for `notification_type: "ToolPermission"` -- the closest
///   Gemini analog to Claude Code's own `Notification` event, and
///   mapped the same way.
/// - `AfterAgent`: fires once per turn after the model's final response
///   -- the closest Gemini analog to Claude Code's/Codex's `Stop`: the
///   agent loop has finished generating and control is back with the
///   user.
const HOOK_EVENTS: &[&str] = &["SessionStart", "Notification", "AfterAgent"];

impl AgentAdapterPort for GeminiCli {
    fn launch_args(
        &self,
        extra: &[String],
        hooks_enabled: bool,
        _native_id: Option<&str>,
    ) -> Vec<String> {
        // `native_id` is ignored here even though `gemini --help` does
        // have a `--session-id <uuid>` flag that could pin one: nothing
        // in the daemon calls this with `Some` today, since `fork_args`
        // below returns `None` -- see its own docs for why -- and wiring
        // the pin now, for a capability nothing yet uses, would be
        // exactly the speculative generality this project's own
        // conventions ask not to build ahead of.
        let mut args = vec!["gemini".to_owned()];
        if hooks_enabled {
            // `--skip-trust` bypasses the interactive workspace-trust
            // gate (`gemini --help`'s own description: "Trust the
            // current workspace for this session"). Unlike Codex,
            // source inspection found no *separate* hook-trust-review
            // gate or sandbox flag gating hook execution specifically --
            // hooks in a project the user has not trusted are simply
            // blocked by the same trust gate everything else is, per
            // the hooks docs' own "Project-level hooks are particularly
            // risky when opening untrusted projects" -- so one flag is
            // the whole story here, matching Claude Code's simpler
            // model rather than Codex's two-flag one. Not independently
            // confirmed live (see this module's own top-level docs);
            // flagged here rather than silently assumed identical to
            // either sibling adapter.
            args.push("--skip-trust".to_owned());
        }
        args.extend(extra.iter().cloned());
        args
    }

    fn needs_input(&self, screen_text: &str) -> AgentSignal {
        if screen_text.contains(BUSY) {
            return AgentSignal::Running;
        }
        if NEEDS_INPUT_MARKERS.iter().any(|m| screen_text.contains(m)) {
            AgentSignal::NeedsInput
        } else {
            AgentSignal::Running
        }
    }

    fn has_verified_hooks(&self) -> bool {
        // Honestly `false`: the hook mechanism and its config format are
        // real and well-documented (this module's own top-level docs),
        // but no hook installed by this adapter has actually been fired
        // and observed on a running `gemini` process -- blocked by a
        // missing `GEMINI_API_KEY`/auth method on this machine, which
        // `gemini` requires before it will even reach a hook-firing
        // state. A real, stated missing prerequisite, not a corner cut.
        false
    }

    fn hook_config(
        &self,
        hook_fire_exe: &std::path::Path,
        session_id: &sessionmgr_core::SessionId,
    ) -> (std::path::PathBuf, String) {
        let mut hooks = serde_json::Map::new();
        for event in HOOK_EVENTS {
            let command = format!(
                "{} __hook-fire --session-id {session_id} --event {event}",
                hook_fire_exe.display()
            );
            let mut group = serde_json::json!({
                "hooks": [{ "type": "command", "command": command }]
            });
            if *event == "SessionStart" {
                // Lifecycle events match on an exact string
                // (geminicli.com/docs/hooks/: "Lifecycle events:
                // Matchers are Exact Strings"). `"startup"` is a fresh
                // interactive start, the only way sessionmgr ever
                // launches this CLI -- narrowing to it, rather than
                // leaving the matcher off, means this hook does not
                // also fire on `--resume`/`/clear`, neither of which
                // sessionmgr uses today.
                group["matcher"] = serde_json::Value::String("startup".to_owned());
            }
            hooks.insert((*event).to_owned(), serde_json::json!([group]));
        }
        let content = serde_json::to_string_pretty(&serde_json::json!({ "hooks": hooks }))
            .expect("a Map<String, Value> of our own construction always serializes");
        (
            std::path::PathBuf::from(".gemini").join("settings.json"),
            content,
        )
    }

    fn hook_signal(&self, event: &str) -> HookOutcome {
        match event {
            "Notification" | "AfterAgent" => HookOutcome::Status(AgentSignal::NeedsInput),
            _ => HookOutcome::Ignore,
        }
    }

    /// `--session-file <path>` is real, and is in fact the most explicit
    /// of the three CLIs' mechanisms
    /// (`docs/decisions/0003-resume-fork-spike.md`): unlike Claude Code's
    /// and Codex's id-based resume/fork, it takes an arbitrary *file
    /// path* to a prior conversation, no native id required at all --
    /// which is why this adapter ignores `new_native_id` entirely: the
    /// installed bundle's own `resolveSessionId` mints a brand-new
    /// session id internally whenever `--session-file` is given, rather
    /// than honoring a caller-supplied `--session-id` alongside it (the
    /// same reason `launch_args`'s own `native_id` parameter is ignored
    /// here).
    ///
    /// The blocking piece issue #15 originally named -- locating the
    /// source session's own current chat-history file -- is live-confirmed
    /// solved, and simpler than the hash this adapter was once worried
    /// about needing to reverse-engineer: `locate_current_chat_file`'s
    /// own docs have the full mechanism.
    ///
    /// Returns `None` when no matching chat file can be found for
    /// `source.workspace_cwd` -- a session-specific "nothing to fork from
    /// right now" outcome (no `projects.json` entry yet, or no `"kind":
    /// "main"` chat file written yet), not a statement that this
    /// adapter's Fork mechanism does not work; see [`ForkSource`]'s own
    /// docs and [`Self::supports_fork`]'s for why the two are different
    /// questions for this adapter specifically.
    fn fork_args(
        &self,
        source: ForkSource<'_>,
        _new_native_id: &str,
        extra: &[String],
    ) -> Option<Vec<String>> {
        let gemini_home = gemini_home_dir()?;
        let chat_file = locate_current_chat_file(&gemini_home, source.workspace_cwd)?;
        let mut args = vec![
            "gemini".to_owned(),
            "--session-file".to_owned(),
            chat_file.to_string_lossy().into_owned(),
        ];
        args.extend(extra.iter().cloned());
        Some(args)
    }

    fn supports_fork(&self) -> bool {
        // The *mechanism* works (see `fork_args`'s own docs) -- whether
        // any single call succeeds depends on whether a matching chat
        // file exists yet for that call's own workspace, which is not
        // this method's question. See `AgentAdapterPort::supports_fork`'s
        // own docs for why these are deliberately different questions for
        // this adapter.
        true
    }
}

/// `gemini`'s own global config directory. Live-confirmed from the
/// installed `@google/gemini-cli` bundle's own `homedir()`
/// (`chunk-32XQ54AJ.js`): `GEMINI_CLI_HOME`, if set, otherwise the OS
/// home directory joined with `.gemini` -- no XDG or other override
/// exists in the bundle for this specific path.
fn gemini_home_dir() -> Option<std::path::PathBuf> {
    if let Some(dir) = std::env::var_os("GEMINI_CLI_HOME") {
        return Some(std::path::PathBuf::from(dir));
    }
    let home = if cfg!(windows) {
        std::env::var_os("USERPROFILE")
    } else {
        std::env::var_os("HOME")
    }?;
    Some(std::path::PathBuf::from(home).join(".gemini"))
}

/// Locates the source session's own current chat-history file --
/// `fork_args`'s `--session-file` target -- given `gemini_home` (see
/// [`gemini_home_dir`]) and the source session's own `workspace_cwd`.
///
/// A plain synchronous filesystem lookup, deliberately: unlike Codex's
/// own discovery problem (issue #14), this needs no post-spawn watch --
/// by the time anything is forked, the source session has already been
/// running and its own chat file already exists.
///
/// Two live-confirmed facts make this possible without any hash:
///
/// 1. `<gemini_home>/projects.json` is a plain JSON registry mapping
///    each project's own absolute working directory to a short,
///    human-readable directory name (confirmed live: e.g.
///    `{"projects": {"/home/user/rusty_yirp": "rusty-yirp"}}`). Read
///    directly from the installed bundle's own `ProjectRegistry.normalizePath`
///    (`chunk-32XQ54AJ.js`): a plain `path.resolve`, lowercased only on
///    `win32` -- no realpath/symlink resolution -- so `workspace_cwd`
///    (already absolute; sessionmgr builds it from `git rev-parse
///    --show-toplevel`) is exactly this key on every platform, modulo
///    that one case-folding rule.
/// 2. `<gemini_home>/tmp/<name>/chats/` holds each project's own chat
///    files, in two shapes confirmed by reading real files this bundle
///    wrote: a flat `session-<timestamp><shortid>.jsonl`, whose first
///    line reads `{"sessionId":...,"kind":"main",...}` -- the actual
///    top-level conversation -- and a nested
///    `<parent-session-id>/<subagent-session-id>.jsonl`, first line
///    `{"kind":"subagent",...}`, a sub-agent/tool-driven conversation
///    that is *not* the one to fork. Only the flat, `"kind":"main"`
///    shape is considered here; the newest such file (by mtime) is the
///    source's own current conversation.
///
/// `None` on any failure along the way (no home directory resolvable, no
/// registry entry for this workspace, no matching chat file, an I/O or
/// parse error) -- deliberately not distinguished further, since every
/// case means the same thing to a caller: nothing to fork from right
/// now.
fn locate_current_chat_file(
    gemini_home: &std::path::Path,
    workspace_cwd: &std::path::Path,
) -> Option<std::path::PathBuf> {
    let registry_text = std::fs::read_to_string(gemini_home.join("projects.json")).ok()?;
    let registry: serde_json::Value = serde_json::from_str(&registry_text).ok()?;
    let key = if cfg!(windows) {
        workspace_cwd.to_string_lossy().to_lowercase()
    } else {
        workspace_cwd.to_string_lossy().into_owned()
    };
    let project_name = registry.get("projects")?.get(key.as_str())?.as_str()?;

    let chats_dir = gemini_home.join("tmp").join(project_name).join("chats");
    let mut newest: Option<(std::time::SystemTime, std::path::PathBuf)> = None;
    for entry in std::fs::read_dir(&chats_dir).ok()?.flatten() {
        let path = entry.path();
        let is_candidate = entry.file_type().is_ok_and(|t| t.is_file())
            && path
                .file_name()
                .and_then(|n| n.to_str())
                .is_some_and(|n| n.starts_with("session-") && n.ends_with(".jsonl"));
        if !is_candidate || !first_line_is_main_session(&path) {
            continue;
        }
        let Ok(modified) = entry.metadata().and_then(|m| m.modified()) else {
            continue;
        };
        if newest.as_ref().is_none_or(|(t, _)| modified > *t) {
            newest = Some((modified, path));
        }
    }
    newest.map(|(_, path)| path)
}

/// Does `path`'s first line look like gemini's own top-level (not
/// subagent) conversation record? A substring check on the raw line
/// rather than full JSON parsing -- the first line is small, stable
/// metadata written once at session start, and this only needs to tell
/// `"kind":"main"` apart from `"kind":"subagent"`, not validate the
/// record's full shape.
fn first_line_is_main_session(path: &std::path::Path) -> bool {
    let Ok(file) = std::fs::File::open(path) else {
        return false;
    };
    let mut first_line = String::new();
    let mut reader = std::io::BufReader::new(file);
    if std::io::BufRead::read_line(&mut reader, &mut first_line).is_err() {
        return false;
    }
    first_line.contains("\"kind\":\"main\"")
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn launch_args_bare_is_just_the_program() {
        assert_eq!(
            GeminiCli.launch_args(&[], false, None),
            vec!["gemini".to_owned()]
        );
    }

    #[test]
    fn launch_args_passes_through_an_initial_prompt() {
        assert_eq!(
            GeminiCli.launch_args(&["fix the failing test".to_owned()], false, None),
            vec!["gemini".to_owned(), "fix the failing test".to_owned()]
        );
    }

    #[test]
    fn launch_args_adds_skip_trust_flag_when_hooks_enabled() {
        assert_eq!(
            GeminiCli.launch_args(&[], true, None),
            vec!["gemini".to_owned(), "--skip-trust".to_owned()]
        );
    }

    #[test]
    fn launch_args_ignores_a_native_id_it_cannot_yet_use() {
        assert_eq!(
            GeminiCli.launch_args(&[], false, Some("some-id")),
            GeminiCli.launch_args(&[], false, None)
        );
    }

    #[test]
    fn supports_fork_is_true() {
        // The mechanism is real (see `fork_args`'s own docs) even though
        // any single call can still come back `None` for a call-specific
        // reason -- see `locate_current_chat_file`'s own tests below.
        assert!(GeminiCli.supports_fork());
    }

    /// A scratch stand-in for `gemini_home_dir()`'s own directory,
    /// removed on drop. Not a process-wide env var override (which would
    /// race every other test in this binary running concurrently) --
    /// `locate_current_chat_file` takes its `gemini_home` as a plain
    /// argument, so these tests just build one and pass it directly.
    struct ScratchGeminiHome(std::path::PathBuf);

    impl ScratchGeminiHome {
        fn new(label: &str) -> Self {
            use std::hash::{Hash, Hasher};
            let mut hasher = std::collections::hash_map::DefaultHasher::new();
            (label, std::process::id(), line!()).hash(&mut hasher);
            let dir = std::env::temp_dir().join(format!("smgh{:x}", hasher.finish()));
            std::fs::create_dir_all(&dir).expect("create the scratch gemini home");
            ScratchGeminiHome(dir)
        }

        fn path(&self) -> &std::path::Path {
            &self.0
        }

        fn write_projects_json(&self, entries: &[(&std::path::Path, &str)]) {
            let mut projects = serde_json::Map::new();
            for (cwd, name) in entries {
                projects.insert(
                    cwd.to_string_lossy().into_owned(),
                    serde_json::Value::String((*name).to_owned()),
                );
            }
            let content =
                serde_json::to_string(&serde_json::json!({ "projects": projects })).unwrap();
            std::fs::write(self.0.join("projects.json"), content).expect("write projects.json");
        }

        /// Writes a flat, top-level chat file directly under
        /// `tmp/<project>/chats/`, with `modified_secs_ago` controlling
        /// its mtime explicitly (`File::set_modified`, stable since Rust
        /// 1.75) -- deterministic "which is newest" tests would otherwise
        /// depend on filesystem timestamp resolution, which is not fine
        /// enough to trust for files written microseconds apart.
        fn write_chat_file(
            &self,
            project: &str,
            filename: &str,
            kind: &str,
            modified_secs_ago: u64,
        ) {
            let dir = self.0.join("tmp").join(project).join("chats");
            std::fs::create_dir_all(&dir).expect("create chats dir");
            let path = dir.join(filename);
            std::fs::write(&path, format!(r#"{{"sessionId":"x","kind":"{kind}"}}"#))
                .expect("write chat file");
            let modified =
                std::time::SystemTime::now() - std::time::Duration::from_secs(modified_secs_ago);
            let file = std::fs::OpenOptions::new()
                .write(true)
                .open(&path)
                .expect("reopen chat file");
            file.set_modified(modified).expect("set mtime");
        }

        /// Writes a nested subagent-shaped file, one directory deeper
        /// than the flat shape -- confirmed live not to be a candidate
        /// (see `locate_current_chat_file`'s own docs).
        fn write_nested_subagent_file(&self, project: &str, parent_session: &str, filename: &str) {
            let dir = self
                .0
                .join("tmp")
                .join(project)
                .join("chats")
                .join(parent_session);
            std::fs::create_dir_all(&dir).expect("create nested chats dir");
            std::fs::write(dir.join(filename), r#"{"kind":"subagent"}"#)
                .expect("write subagent file");
        }
    }

    impl Drop for ScratchGeminiHome {
        fn drop(&mut self) {
            let _ = std::fs::remove_dir_all(&self.0);
        }
    }

    #[test]
    fn locate_current_chat_file_returns_none_without_a_projects_json() {
        let home = ScratchGeminiHome::new("no-registry");
        let workspace = std::path::Path::new("/some/project");
        assert_eq!(locate_current_chat_file(home.path(), workspace), None);
    }

    #[test]
    fn locate_current_chat_file_returns_none_without_a_matching_registry_entry() {
        let home = ScratchGeminiHome::new("no-entry");
        let workspace = std::path::Path::new("/some/project");
        home.write_projects_json(&[(std::path::Path::new("/some/other/project"), "other")]);
        assert_eq!(locate_current_chat_file(home.path(), workspace), None);
    }

    #[test]
    fn locate_current_chat_file_returns_none_with_no_chat_files_yet() {
        let home = ScratchGeminiHome::new("no-chats-yet");
        let workspace = std::path::Path::new("/some/project");
        home.write_projects_json(&[(workspace, "myproj")]);
        std::fs::create_dir_all(home.path().join("tmp/myproj/chats")).unwrap();
        assert_eq!(locate_current_chat_file(home.path(), workspace), None);
    }

    #[test]
    fn locate_current_chat_file_finds_the_newest_main_session_file() {
        let home = ScratchGeminiHome::new("newest-wins");
        let workspace = std::path::Path::new("/some/project");
        home.write_projects_json(&[(workspace, "myproj")]);
        home.write_chat_file(
            "myproj",
            "session-2026-01-01T00-00-aaaa1111.jsonl",
            "main",
            120,
        );
        home.write_chat_file(
            "myproj",
            "session-2026-01-02T00-00-bbbb2222.jsonl",
            "main",
            10,
        );
        let found = locate_current_chat_file(home.path(), workspace);
        assert_eq!(
            found,
            Some(
                home.path()
                    .join("tmp/myproj/chats/session-2026-01-02T00-00-bbbb2222.jsonl")
            )
        );
    }

    #[test]
    fn locate_current_chat_file_ignores_nested_subagent_files_even_if_newer() {
        let home = ScratchGeminiHome::new("ignore-subagent");
        let workspace = std::path::Path::new("/some/project");
        home.write_projects_json(&[(workspace, "myproj")]);
        home.write_chat_file(
            "myproj",
            "session-2026-01-01T00-00-aaaa1111.jsonl",
            "main",
            120,
        );
        // Nested and newer -- and, per its own `Drop`-free write, has no
        // controlled mtime at all, meaning it is effectively "now",
        // strictly newer than the flat file above. Still must not win.
        home.write_nested_subagent_file("myproj", "aaaa1111", "subagent.jsonl");
        let found = locate_current_chat_file(home.path(), workspace);
        assert_eq!(
            found,
            Some(
                home.path()
                    .join("tmp/myproj/chats/session-2026-01-01T00-00-aaaa1111.jsonl")
            )
        );
    }

    #[test]
    fn locate_current_chat_file_ignores_a_flat_file_that_is_not_kind_main() {
        let home = ScratchGeminiHome::new("ignore-non-main");
        let workspace = std::path::Path::new("/some/project");
        home.write_projects_json(&[(workspace, "myproj")]);
        // Shaped like a flat, top-level file, but not actually
        // `"kind":"main"` -- should not happen in practice, but this
        // guards the filter's own logic rather than assuming it.
        home.write_chat_file(
            "myproj",
            "session-2026-01-01T00-00-aaaa1111.jsonl",
            "subagent",
            10,
        );
        assert_eq!(locate_current_chat_file(home.path(), workspace), None);
    }

    #[test]
    fn busy_screen_is_running_even_if_it_also_contains_a_marker_substring() {
        // Mirrors claude_code's/codex's own busy-wins-first test: the
        // busy check must be unconditional, not "checked only when no
        // marker matched".
        assert_eq!(
            GeminiCli.needs_input("Thinking... (esc to cancel, 3s)"),
            AgentSignal::Running
        );
    }

    #[test]
    fn idle_screen_needs_input() {
        assert_eq!(
            GeminiCli.needs_input("  Type your message or @path/to/file\n? for shortcuts"),
            AgentSignal::NeedsInput
        );
    }

    #[test]
    fn trust_gate_needs_input() {
        assert_eq!(
            GeminiCli.needs_input("Do you trust the files in this folder?"),
            AgentSignal::NeedsInput
        );
    }

    #[test]
    fn tool_confirmation_needs_input() {
        assert_eq!(
            GeminiCli.needs_input("Allow execution of \"run_shell_command\"?"),
            AgentSignal::NeedsInput
        );
        assert_eq!(
            GeminiCli.needs_input("Do you want to proceed?"),
            AgentSignal::NeedsInput
        );
    }

    #[test]
    fn plain_output_is_running() {
        assert_eq!(GeminiCli.needs_input("hello world"), AgentSignal::Running);
    }

    #[test]
    fn has_verified_hooks_is_honestly_false() {
        assert!(!GeminiCli.has_verified_hooks());
    }

    #[test]
    fn hook_config_writes_gemini_settings_json_with_every_event() {
        let id = sessionmgr_core::SessionId::new(1_700_000_000_000, 1);
        let (path, content) =
            GeminiCli.hook_config(std::path::Path::new("C:/x/sessionmgr.exe"), &id);
        assert_eq!(
            path,
            std::path::PathBuf::from(".gemini").join("settings.json")
        );
        let parsed: serde_json::Value = serde_json::from_str(&content).expect("valid JSON");
        for event in HOOK_EVENTS {
            let command = parsed["hooks"][event][0]["hooks"][0]["command"]
                .as_str()
                .unwrap_or_else(|| panic!("missing command for {event}"));
            assert!(command.contains("C:/x/sessionmgr.exe"));
            assert!(command.contains(&format!("--session-id {id}")));
            assert!(command.contains(&format!("--event {event}")));
        }
    }

    #[test]
    fn hook_config_matches_session_start_to_a_real_startup_only() {
        let id = sessionmgr_core::SessionId::new(1_700_000_000_000, 1);
        let (_, content) = GeminiCli.hook_config(std::path::Path::new("C:/x/sessionmgr.exe"), &id);
        let parsed: serde_json::Value = serde_json::from_str(&content).expect("valid JSON");
        assert_eq!(parsed["hooks"]["SessionStart"][0]["matcher"], "startup");
        assert!(parsed["hooks"]["Notification"][0]["matcher"].is_null());
        assert!(parsed["hooks"]["AfterAgent"][0]["matcher"].is_null());
    }

    #[test]
    fn hook_signal_maps_needs_input_events() {
        assert_eq!(
            GeminiCli.hook_signal("Notification"),
            HookOutcome::Status(AgentSignal::NeedsInput)
        );
        assert_eq!(
            GeminiCli.hook_signal("AfterAgent"),
            HookOutcome::Status(AgentSignal::NeedsInput)
        );
    }

    #[test]
    fn hook_signal_ignores_session_start_and_unrecognized_events() {
        assert_eq!(GeminiCli.hook_signal("SessionStart"), HookOutcome::Ignore);
        assert_eq!(GeminiCli.hook_signal("BeforeTool"), HookOutcome::Ignore);
    }

    #[test]
    fn hook_signal_has_no_notify_mapping() {
        // Documented difference from Claude Code/Codex: Gemini CLI's own
        // hook event list has no sub-agent-finished event, so nothing
        // here should ever produce HookOutcome::Notify.
        for event in HOOK_EVENTS {
            assert_ne!(GeminiCli.hook_signal(event), HookOutcome::Notify);
        }
    }
}
