use std::sync::atomic::{AtomicUsize, Ordering};

use rusty_uuid::Uuid;

use super::*;
use crate::AgentHomes;

#[cfg(not(windows))]
const CTL: &str = "/opt/rusty mux/bin/rusty-mux-ctl";
/// Codex on Windows refuses a path its shells would misread, such as one
/// with a space.
#[cfg(windows)]
const CTL: &str = r"C:\opt\rusty-mux\bin\rusty-mux-ctl.exe";

fn json(text: &str) -> Value {
    Value::from_json_str(text).expect("JSON")
}

fn ours() -> String {
    settings::command(CTL)
}

/// A fresh, empty agent config directory.
fn scratch() -> PathBuf {
    static NEXT: AtomicUsize = AtomicUsize::new(0);
    let n = NEXT.fetch_add(1, Ordering::Relaxed);
    let dir = std::env::temp_dir().join(format!("rusty-mux-ctl-agent-{}-{n}", std::process::id()));
    let _ = fs::remove_dir_all(&dir);
    fs::create_dir_all(&dir).expect("dir");
    dir
}

fn env(ctl: &str) -> Env {
    Env {
        ctl: Some(ctl.into()),
        ..Env::default()
    }
}

/// Claude Code in `dir`, as `--dir DIR` gives.
fn claude_in(dir: &Path) -> AgentArgs {
    AgentArgs {
        agents: vec![AgentKind::Claude],
        scope: AgentScope::Dir(dir.into()),
        dry_run: false,
    }
}

/// Runs `command` for Claude Code in `dir`, returning the exit and stdout.
fn agent(command: fn(AgentArgs) -> AgentCommand, dir: &Path, ctl: &str) -> (Exit, String) {
    let mut out = Vec::new();
    let exit = run(&command(claude_in(dir)), &env(ctl), false, &mut out).expect("runs");
    (exit, String::from_utf8(out).expect("UTF-8"))
}

fn files_in(dir: &Path) -> Vec<String> {
    let mut names: Vec<String> = fs::read_dir(dir)
        .expect("read dir")
        .map(|e| e.expect("entry").file_name().to_string_lossy().into_owned())
        .collect();
    names.sort();
    names
}

#[test]
fn the_command_quotes_the_path_for_a_posix_shell() {
    assert_eq!(
        settings::command("/a b/rusty-mux-ctl"),
        "'/a b/rusty-mux-ctl' agent hook"
    );
    assert_eq!(
        settings::command("/it's/rusty-mux-ctl"),
        r"'/it'\''s/rusty-mux-ctl' agent hook"
    );
}

#[test]
fn adding_keeps_every_other_setting_and_hook() {
    let before = json(
        r#"{"model":"x","hooks":{"SessionStart":[{"matcher":"startup","hooks":[{"type":"command","command":"echo hi"}]}],"Stop":[]}}"#,
    );
    let after = settings::with_hook(before.clone(), &ours(), None).expect("adds");
    assert_eq!(after["model"], before["model"]);
    assert_eq!(after["hooks"]["Stop"], before["hooks"]["Stop"]);
    let groups = after["hooks"]["SessionStart"].as_array().expect("array");
    assert_eq!(groups[0], before["hooks"]["SessionStart"][0]);
    assert_eq!(
        groups[1]["hooks"][0]["command"].as_str(),
        Some(ours().as_str())
    );
    assert_eq!(settings::state(&after, &ours(), None), Ok(State::Installed));
    assert_eq!(
        settings::with_hook(after.clone(), &ours(), None),
        Ok(after),
        "idempotent"
    );
}

#[test]
fn a_moved_ctl_is_updated_in_place_not_added_twice() {
    let old = settings::with_hook(json("{}"), &settings::command("/old/rusty-mux-ctl"), None)
        .expect("adds");
    assert_eq!(settings::state(&old, &ours(), None), Ok(State::Outdated));
    let new = settings::with_hook(old, &ours(), None).expect("updates");
    let groups = new["hooks"]["SessionStart"].as_array().expect("array");
    assert_eq!(groups.len(), 1);
    assert_eq!(settings::state(&new, &ours(), None), Ok(State::Installed));
}

#[test]
fn removing_drops_only_ours_and_the_containers_it_empties() {
    let mine =
        json(r#"{"hooks":{"SessionStart":[{"hooks":[{"type":"command","command":"echo hi"}]}]}}"#);
    let with = settings::with_hook(mine.clone(), &ours(), None).expect("adds");
    assert_eq!(settings::without_hook(with, None), Ok(mine.clone()));
    let alone = settings::with_hook(json(r#"{"a":1}"#), &ours(), None).expect("adds");
    assert_eq!(settings::without_hook(alone, None), Ok(json(r#"{"a":1}"#)));
    // A group shared with someone else's hook keeps theirs.
    let shared = json(&format!(
        r#"{{"hooks":{{"SessionStart":[{{"hooks":[{{"type":"command","command":"echo hi"}},{{"type":"command","command":{}}}]}}]}}}}"#,
        Value::from(ours()).to_json_string()
    ));
    assert_eq!(settings::without_hook(shared, None), Ok(mine));
}

#[test]
fn a_shape_it_cannot_edit_is_refused() {
    for bad in [r"[]", r#"{"hooks":[]}"#, r#"{"hooks":{"SessionStart":{}}}"#] {
        assert!(
            settings::with_hook(json(bad), &ours(), None).is_err(),
            "{bad}"
        );
        assert!(settings::without_hook(json(bad), None).is_err(), "{bad}");
    }
}

#[test]
fn setup_then_remove_leaves_an_empty_directory_empty() {
    let dir = scratch();
    let (exit, out) = agent(AgentCommand::Setup, &dir, CTL);
    assert_eq!(exit, Exit::Ok);
    assert!(out.starts_with("claude   skill  installed"), "{out}");
    assert_eq!(
        fs::read_to_string(dir.join("skills/rusty-mux/SKILL.md")).expect("skill"),
        SKILL
    );
    assert_eq!(agent(AgentCommand::Status, &dir, CTL).0, Exit::Ok);
    let (_, again) = agent(AgentCommand::Setup, &dir, CTL);
    assert!(
        again.contains("skill  unchanged") && again.contains("hook   unchanged"),
        "{again}"
    );

    let (exit, out) = agent(AgentCommand::Remove, &dir, CTL);
    assert_eq!(exit, Exit::Ok);
    assert!(
        out.contains("skill  removed") && out.contains("hook   removed"),
        "{out}"
    );
    assert!(files_in(&dir).is_empty(), "{:?}", files_in(&dir));
    assert_eq!(agent(AgentCommand::Status, &dir, CTL).0, Exit::Failed);
}

#[test]
fn remove_restores_existing_settings_byte_for_byte() {
    let dir = scratch();
    let original = "{\n    \"zeta\": true,\n    \"alpha\": [1, 2]\n}\n";
    fs::write(dir.join("settings.json"), original).expect("write");
    agent(AgentCommand::Setup, &dir, CTL);
    let installed = fs::read_to_string(dir.join("settings.json")).expect("read");
    assert_eq!(json(&installed)["zeta"], Value::Bool(true));
    assert!(dir.join("settings.json.rusty-mux.bak").exists());

    agent(AgentCommand::Remove, &dir, CTL);
    assert_eq!(
        fs::read_to_string(dir.join("settings.json")).expect("read"),
        original
    );
    assert!(!dir.join("settings.json.rusty-mux.bak").exists());
}

#[test]
fn remove_keeps_edits_made_after_setup() {
    let dir = scratch();
    fs::write(dir.join("settings.json"), r#"{"a":1}"#).expect("write");
    agent(AgentCommand::Setup, &dir, CTL);
    let mut edited = json(&fs::read_to_string(dir.join("settings.json")).expect("read"));
    edited["b"] = 2u64.into();
    fs::write(dir.join("settings.json"), edited.to_json_string()).expect("write");

    agent(AgentCommand::Remove, &dir, CTL);
    let left = json(&fs::read_to_string(dir.join("settings.json")).expect("read"));
    assert_eq!(left, json(r#"{"a":1,"b":2}"#));
    assert!(!dir.join("settings.json.rusty-mux.bak").exists());
}

#[test]
fn foreign_files_are_never_replaced() {
    let dir = scratch();
    let skill = dir.join("skills/rusty-mux/SKILL.md");
    fs::create_dir_all(skill.parent().expect("parent")).expect("dir");
    fs::write(&skill, "my own notes").expect("write");
    let mut out = Vec::new();
    let refused = run(
        &AgentCommand::Setup(claude_in(&dir)),
        &env(CTL),
        false,
        &mut out,
    );
    assert!(refused.is_err());
    assert!(!dir.join("settings.json").exists(), "nothing is written");
    let (_, removed) = agent(AgentCommand::Remove, &dir, CTL);
    assert!(removed.contains("skill  kept"), "{removed}");
    assert_eq!(fs::read_to_string(&skill).expect("read"), "my own notes");

    fs::write(dir.join("settings.json"), "{ not json").expect("write");
    fs::remove_file(&skill).expect("rm");
    assert!(
        run(
            &AgentCommand::Setup(claude_in(&dir)),
            &env(CTL),
            false,
            &mut out
        )
        .is_err()
    );
    assert!(
        !skill.exists(),
        "a bad settings file stops setup before any write"
    );
    assert_eq!(
        fs::read_to_string(dir.join("settings.json")).expect("read"),
        "{ not json"
    );
}

#[test]
fn status_reports_an_outdated_skill_and_a_moved_ctl() {
    let dir = scratch();
    agent(AgentCommand::Setup, &dir, "/old/rusty-mux-ctl");
    let skill = dir.join("skills/rusty-mux/SKILL.md");
    fs::write(&skill, SKILL.replace("# rusty_mux", "# older")).expect("write");
    let (exit, out) = agent(AgentCommand::Status, &dir, CTL);
    assert_eq!(exit, Exit::Failed);
    assert!(
        out.contains("skill  outdated") && out.contains("hook   outdated"),
        "{out}"
    );
    let (_, out) = agent(AgentCommand::Setup, &dir, CTL);
    assert!(
        out.contains("skill  updated") && out.contains("hook   updated"),
        "{out}"
    );
    assert_eq!(agent(AgentCommand::Status, &dir, CTL).0, Exit::Ok);
}

fn id(n: u8) -> Uuid {
    let mut bytes = [0; 16];
    bytes[15] = n;
    Uuid::from_bytes(bytes)
}

#[test]
fn the_hook_speaks_only_inside_a_block() {
    assert_eq!(hook_text(&Defaults::default(), Some(Path::new(CTL))), None);
    let block = id(3);
    let here = Defaults {
        session: Some(id(1)),
        window: Some(id(2)),
        block: Some(block),
        dir: None,
    };
    let text = hook_text(&here, Some(Path::new(CTL))).expect("text");
    assert!(
        text.contains(&block.to_string()) && text.contains(CTL),
        "{text}"
    );
}

#[test]
fn the_skill_carries_its_mark_after_the_front_matter() {
    assert!(SKILL.starts_with("---\nname: rusty-mux\ndescription: "));
    assert!(SKILL.contains(MARK));
}

#[test]
fn only_the_exact_generated_command_is_ours() {
    for theirs in [
        "echo rusty-mux-ctl agent hook",
        "rusty-mux-ctl agent hook",
        "'rusty-mux-ctl' agent hook",
        "'/usr/bin/my-rusty-mux-ctl' agent hook",
        "'/usr/bin/rusty-mux-ctl-wrapper' agent hook",
        "'/usr/bin/rusty-mux-ctl' agent hook --verbose",
        "'/usr/bin/rusty-mux-ctl' agent hook; echo hi",
        "'/a'b/rusty-mux-ctl' agent hook",
        "/usr/bin/wrap '/usr/bin/rusty-mux-ctl' agent hook",
    ] {
        let settings = json(&format!(
            r#"{{"hooks":{{"SessionStart":[{{"hooks":[{{"type":"command","command":{}}}]}}]}}}}"#,
            Value::from(theirs).to_json_string()
        ));
        assert_eq!(
            settings::state(&settings, &ours(), None),
            Ok(State::Missing),
            "{theirs}"
        );
        let added = settings::with_hook(settings.clone(), &ours(), None).expect("adds");
        assert_eq!(
            added["hooks"]["SessionStart"][0], settings["hooks"]["SessionStart"][0],
            "{theirs}"
        );
        assert_eq!(
            settings::without_hook(added, Some(&settings)),
            Ok(settings.clone()),
            "{theirs}"
        );
    }
    for moved in ["/it's/rusty-mux-ctl", r"C:\Program Files\rusty-mux-ctl.exe"] {
        let old = settings::with_hook(json("{}"), &settings::command(moved), None).expect("adds");
        assert_eq!(
            settings::state(&old, &ours(), None),
            Ok(State::Outdated),
            "{moved}"
        );
    }
}

#[test]
fn empty_hook_containers_round_trip_byte_for_byte() {
    for original in [r#"{"hooks":{}}"#, "{\"hooks\": {\"SessionStart\": []}}\n"] {
        let dir = scratch();
        fs::write(dir.join("settings.json"), original).expect("write");
        agent(AgentCommand::Setup, &dir, CTL);
        agent(AgentCommand::Remove, &dir, CTL);
        assert_eq!(
            fs::read_to_string(dir.join("settings.json")).expect("read"),
            original
        );

        // An edit after setup is kept, and so are the containers.
        agent(AgentCommand::Setup, &dir, CTL);
        let mut edited = json(&fs::read_to_string(dir.join("settings.json")).expect("read"));
        edited["model"] = "x".into();
        fs::write(dir.join("settings.json"), edited.to_json_string()).expect("write");
        agent(AgentCommand::Remove, &dir, CTL);
        let mut expected = json(original);
        expected["model"] = "x".into();
        let left = json(&fs::read_to_string(dir.join("settings.json")).expect("read"));
        assert_eq!(left, expected, "{original}");
    }
}

#[test]
fn occupied_temporary_names_are_skipped_not_truncated() {
    let dir = scratch();
    fs::write(dir.join("settings.json"), "{}").expect("write");
    let target = fs::canonicalize(dir.join("settings.json")).expect("canonical");
    let temp = |n: u32| {
        let mut name = target.clone().into_os_string();
        name.push(format!(".rusty-mux-{}-{n}.tmp", std::process::id()));
        PathBuf::from(name)
    };
    fs::write(temp(0), "someone else's").expect("write");
    #[cfg(unix)]
    {
        let victim = dir.join("victim");
        fs::write(&victim, "keep me").expect("write");
        std::os::unix::fs::symlink(&victim, temp(1)).expect("symlink");
    }
    assert_eq!(agent(AgentCommand::Setup, &dir, CTL).0, Exit::Ok);
    assert_eq!(fs::read_to_string(temp(0)).expect("read"), "someone else's");
    #[cfg(unix)]
    {
        assert!(
            fs::symlink_metadata(temp(1))
                .expect("link")
                .file_type()
                .is_symlink()
        );
        assert_eq!(
            fs::read_to_string(dir.join("victim")).expect("read"),
            "keep me"
        );
    }
    assert_eq!(agent(AgentCommand::Status, &dir, CTL).0, Exit::Ok);
}

#[cfg(unix)]
#[test]
fn private_settings_stay_private() {
    use std::os::unix::fs::PermissionsExt;
    let mode = |path: &Path| fs::metadata(path).expect("meta").permissions().mode() & 0o777;
    let dir = scratch();
    let settings = dir.join("settings.json");
    fs::write(&settings, "{}").expect("write");
    fs::set_permissions(&settings, fs::Permissions::from_mode(0o600)).expect("chmod");
    agent(AgentCommand::Setup, &dir, CTL);
    assert_eq!(mode(&settings), 0o600);
    assert_eq!(mode(&dir.join("settings.json.rusty-mux.bak")), 0o600);
    agent(AgentCommand::Remove, &dir, CTL);
    assert_eq!(mode(&settings), 0o600);

    // A file setup creates is private too, whatever the umask.
    let fresh = scratch();
    agent(AgentCommand::Setup, &fresh, CTL);
    assert_eq!(mode(&fresh.join("settings.json")), 0o600);
}

/// On Windows every copy is the owner's alone, whatever the directory
/// grants: a settings file with an explicit owner-only DACL, in a broader
/// directory whose path has words like `Users` in it, stays so through
/// setup, its backup, an update and remove. Checked on the security
/// descriptor's access rules (`Get-Acl`), by SID, not by account names.
#[cfg(windows)]
#[test]
fn private_settings_stay_private_on_windows() {
    // Whether `path`'s DACL is protected, then one line per entry: its
    // SID (as a SID, never an account name or SDDL alias such as `LA`),
    // allow or deny, rights, and whether it is inherited.
    const DACL: &str = "$acl = Get-Acl -LiteralPath $env:ACL_PATH; \
        $acl.AreAccessRulesProtected; \
        $acl.GetAccessRules($true, $true, [Security.Principal.SecurityIdentifier]) | \
        ForEach-Object { '{0} {1} {2} {3}' -f $_.IdentityReference.Value, \
        $_.AccessControlType, [int]$_.FileSystemRights, $_.IsInherited }";
    let dacl = |path: &Path| -> (bool, Vec<String>) {
        let out = std::process::Command::new("powershell")
            .args(["-NoProfile", "-NonInteractive", "-Command", DACL])
            .env("ACL_PATH", path)
            .output()
            .expect("powershell");
        assert!(out.status.success(), "{out:?}");
        let text = String::from_utf8_lossy(&out.stdout).into_owned();
        let mut lines = text.lines().map(str::trim).filter(|l| !l.is_empty());
        let protected = lines.next() == Some("True");
        (protected, lines.map(str::to_owned).collect())
    };
    let me = {
        let out = std::process::Command::new("powershell")
            .args([
                "-NoProfile",
                "-NonInteractive",
                "-Command",
                "[Security.Principal.WindowsIdentity]::GetCurrent().User.Value",
            ])
            .output()
            .expect("powershell");
        String::from_utf8_lossy(&out.stdout).trim().to_owned()
    };
    assert!(me.starts_with("S-1-"), "the current user's SID: {me}");
    // Protected, with one entry: full control (0x1F01FF) allowed to the
    // current user, not inherited.
    let owner_only = |path: &Path| {
        let (protected, entries) = dacl(path);
        let ok = protected && entries == [format!("{me} Allow 2032127 False")];
        assert!(ok, "{}: protected={protected} {entries:?}", path.display());
    };
    let dir = scratch().join("Users Everyone SYSTEM");
    fs::create_dir_all(&dir).expect("dir");
    let (_, parent) = dacl(&dir);
    assert!(parent.len() > 1, "the parent is broader: {parent:?}");
    let settings = dir.join("settings.json");
    fs::write(&settings, "{}").expect("write");
    platform_windows::fs::restrict_to_owner(&settings).expect("restrict");
    owner_only(&settings);

    assert_eq!(agent(AgentCommand::Setup, &dir, CTL).0, Exit::Ok);
    owner_only(&settings);
    owner_only(&dir.join("settings.json.rusty-mux.bak"));
    owner_only(&dir.join("skills").join("rusty-mux").join("SKILL.md"));
    // An update replaces the file again.
    assert_eq!(
        agent(AgentCommand::Setup, &dir, r"C:\moved\rusty-mux-ctl.exe").0,
        Exit::Ok
    );
    owner_only(&settings);
    assert_eq!(agent(AgentCommand::Remove, &dir, CTL).0, Exit::Ok);
    owner_only(&settings);
    assert_eq!(fs::read_to_string(&settings).expect("read"), "{}");
    assert!(!dir.join("settings.json.rusty-mux.bak").exists());
}

/// Runs `command` for `agents` with `home` as the home directory and the
/// project directory, returning the exit and stdout.
fn in_home(
    command: fn(AgentArgs) -> AgentCommand,
    agents: &[AgentKind],
    scope: AgentScope,
    dry_run: bool,
    home: &Path,
) -> Result<(Exit, String), String> {
    let mut env = env(CTL);
    env.agent_homes.home = Some(home.into());
    env.here.dir = Some(home.join("project"));
    let args = AgentArgs {
        agents: agents.to_vec(),
        scope,
        dry_run,
    };
    let mut out = Vec::new();
    let exit = run(&command(args), &env, false, &mut out)?;
    Ok((exit, String::from_utf8(out).expect("UTF-8")))
}

/// Every file under `dir`, relative and sorted.
fn tree(dir: &Path) -> Vec<String> {
    fn walk(root: &Path, dir: &Path, into: &mut Vec<String>) {
        for entry in fs::read_dir(dir).expect("read dir") {
            let path = entry.expect("entry").path();
            if path.is_dir() {
                walk(root, &path, into);
            } else {
                let rel = path.strip_prefix(root).expect("under root");
                into.push(rel.to_string_lossy().replace('\\', "/"));
            }
        }
    }
    let mut files = Vec::new();
    walk(dir, dir, &mut files);
    files.sort();
    files
}

#[test]
fn each_agent_has_its_own_hook_and_a_shared_skill() {
    let homes = AgentHomes {
        home: Some("/h".into()),
        ..AgentHomes::default()
    };
    let at = |agent, scope: &AgentScope| {
        let layout =
            layout::layout(agent, scope, &homes, Some(Path::new("/p")), CTL).expect("layout");
        (layout.skill, layout.hook.path().to_path_buf())
    };
    let paths = |skill: &str, hook: &str| (PathBuf::from(skill), PathBuf::from(hook));
    let user = AgentScope::User;
    assert_eq!(
        at(AgentKind::Claude, &user),
        paths(
            "/h/.claude/skills/rusty-mux/SKILL.md",
            "/h/.claude/settings.json"
        )
    );
    assert_eq!(
        at(AgentKind::Codex, &user),
        paths(
            "/h/.agents/skills/rusty-mux/SKILL.md",
            "/h/.codex/hooks.json"
        )
    );
    assert_eq!(
        at(AgentKind::OpenCode, &user),
        paths(
            "/h/.agents/skills/rusty-mux/SKILL.md",
            "/h/.config/opencode/plugins/rusty-mux.js"
        )
    );
    assert_eq!(
        at(AgentKind::Pi, &user),
        paths(
            "/h/.agents/skills/rusty-mux/SKILL.md",
            "/h/.pi/agent/extensions/rusty-mux.ts"
        )
    );
    let project = AgentScope::Project;
    assert_eq!(
        at(AgentKind::Claude, &project),
        paths(
            "/p/.claude/skills/rusty-mux/SKILL.md",
            "/p/.claude/settings.json"
        )
    );
    assert_eq!(
        at(AgentKind::Codex, &project),
        paths(
            "/p/.agents/skills/rusty-mux/SKILL.md",
            "/p/.codex/hooks.json"
        )
    );
    assert_eq!(
        at(AgentKind::OpenCode, &project).1,
        PathBuf::from("/p/.opencode/plugins/rusty-mux.js")
    );
    assert_eq!(
        at(AgentKind::Pi, &project).1,
        PathBuf::from("/p/.pi/extensions/rusty-mux.ts")
    );
}

#[test]
fn each_agents_variable_moves_its_user_directory() {
    let mut homes = AgentHomes {
        home: Some("/h".into()),
        claude: Some("/c".into()),
        codex: Some("/x".into()),
        config: Some("/xdg".into()),
        pi: Some("/pi".into()),
    };
    let hook = |agent| {
        layout::layout(agent, &AgentScope::User, &homes, None, CTL)
            .map(|l| l.hook.path().to_path_buf())
    };
    assert_eq!(hook(AgentKind::Claude), Ok("/c/settings.json".into()));
    assert_eq!(hook(AgentKind::Codex), Ok("/x/hooks.json".into()));
    assert_eq!(
        hook(AgentKind::OpenCode),
        Ok("/xdg/opencode/plugins/rusty-mux.js".into())
    );
    assert_eq!(
        hook(AgentKind::Pi),
        Ok("/pi/extensions/rusty-mux.ts".into())
    );
    // The shared skill lives under the home directory, which may be unknown.
    homes.home = None;
    let err = layout::layout(AgentKind::Codex, &AgentScope::User, &homes, None, CTL)
        .err()
        .expect("no home");
    assert!(err.contains("HOME"), "{err}");
    // Project scope needs the current directory.
    assert!(layout::layout(AgentKind::Pi, &AgentScope::Project, &homes, None, CTL).is_err());
}

#[test]
fn codex_takes_the_claude_hook_shape_in_hooks_json() {
    let home = scratch();
    let codex = home.join(".codex");
    fs::create_dir_all(&codex).expect("dir");
    let original = "{\"hooks\": {\"Stop\": []}, \"x\": 1}\n";
    fs::write(codex.join("hooks.json"), original).expect("write");
    let setup = in_home(
        AgentCommand::Setup,
        &[AgentKind::Codex],
        AgentScope::User,
        false,
        &home,
    );
    let (exit, out) = setup.expect("setup");
    assert_eq!(exit, Exit::Ok);
    assert!(out.contains("codex    hook   installed"), "{out}");
    let hooks = json(&fs::read_to_string(codex.join("hooks.json")).expect("read"));
    assert_eq!(
        hooks["hooks"]["SessionStart"][0]["hooks"][0]["command"].as_str(),
        Some(ours().as_str())
    );
    assert_eq!(hooks["x"], Value::from(1u64));
    assert_eq!(
        fs::read_to_string(home.join(".agents/skills/rusty-mux/SKILL.md")).expect("skill"),
        SKILL
    );

    let removed = in_home(
        AgentCommand::Remove,
        &[AgentKind::Codex],
        AgentScope::User,
        false,
        &home,
    );
    assert_eq!(removed.expect("remove").0, Exit::Ok);
    assert_eq!(
        fs::read_to_string(codex.join("hooks.json")).expect("read"),
        original,
        "restored byte for byte"
    );
    assert_eq!(tree(&home), [".codex/hooks.json"]);
}

#[test]
fn opencode_and_pi_hooks_are_whole_files_naming_this_ctl() {
    let home = scratch();
    let agents = [AgentKind::OpenCode, AgentKind::Pi];
    let (exit, out) =
        in_home(AgentCommand::Setup, &agents, AgentScope::User, false, &home).expect("setup");
    assert_eq!(exit, Exit::Ok, "{out}");
    let plugin = home.join(".config/opencode/plugins/rusty-mux.js");
    let extension = home.join(".pi/agent/extensions/rusty-mux.ts");
    assert_eq!(
        fs::read_to_string(&plugin).expect("plugin"),
        scripts::opencode(CTL)
    );
    assert_eq!(
        fs::read_to_string(&extension).expect("extension"),
        scripts::pi(CTL)
    );
    for text in [scripts::opencode(CTL), scripts::pi(CTL)] {
        assert!(text.starts_with(scripts::MARK));
        let line = format!("const CTL = {};", Value::from(CTL).to_json_string());
        assert!(text.contains(&line), "{text}");
    }
    let status = in_home(
        AgentCommand::Status,
        &agents,
        AgentScope::User,
        false,
        &home,
    );
    assert_eq!(status.expect("status").0, Exit::Ok);

    // A moved ctl makes both outdated, and setup rewrites them.
    let moved = |command: fn(AgentArgs) -> AgentCommand| {
        let mut env = env("/new/rusty-mux-ctl");
        env.agent_homes.home = Some(home.clone());
        let args = AgentArgs {
            agents: agents.to_vec(),
            scope: AgentScope::User,
            dry_run: false,
        };
        let mut out = Vec::new();
        let exit = run(&command(args), &env, false, &mut out).expect("runs");
        (exit, String::from_utf8(out).expect("UTF-8"))
    };
    let (exit, out) = moved(AgentCommand::Status);
    assert_eq!(exit, Exit::Failed);
    assert_eq!(out.matches("hook   outdated").count(), 2, "{out}");
    let (_, out) = moved(AgentCommand::Setup);
    assert_eq!(out.matches("hook   updated").count(), 2, "{out}");
    assert_eq!(
        fs::read_to_string(&plugin).expect("plugin"),
        scripts::opencode("/new/rusty-mux-ctl")
    );

    let removed = in_home(
        AgentCommand::Remove,
        &agents,
        AgentScope::User,
        false,
        &home,
    );
    assert!(
        removed
            .expect("remove")
            .1
            .contains("opencode skill  removed")
    );
    assert!(tree(&home).is_empty(), "{:?}", tree(&home));
}

#[test]
fn a_path_cannot_end_the_scripts_string_early() {
    let ctl = "/a\"b\\c\n';`${x}`/rusty-mux-ctl";
    for text in [scripts::opencode(ctl), scripts::pi(ctl)] {
        let line = text
            .lines()
            .find_map(|l| l.strip_prefix("const CTL = "))
            .expect("the CTL line");
        let literal = line.strip_suffix(';').expect("one statement");
        assert_eq!(json(literal).as_str(), Some(ctl), "{line}");
    }
}

#[test]
fn a_foreign_hook_file_is_never_replaced() {
    let home = scratch();
    let plugin = home.join(".config/opencode/plugins/rusty-mux.js");
    fs::create_dir_all(plugin.parent().expect("parent")).expect("dir");
    fs::write(&plugin, "export const Mine = async () => ({});\n").expect("write");
    let refused = in_home(
        AgentCommand::Setup,
        &[AgentKind::Codex, AgentKind::OpenCode],
        AgentScope::User,
        false,
        &home,
    );
    assert!(refused.expect_err("refused").contains("rusty-mux.js"));
    assert_eq!(
        tree(&home),
        [".config/opencode/plugins/rusty-mux.js"],
        "nothing is written for any agent"
    );
    let (_, out) = in_home(
        AgentCommand::Remove,
        &[AgentKind::OpenCode],
        AgentScope::User,
        false,
        &home,
    )
    .expect("remove");
    assert!(out.contains("opencode hook   absent"), "{out}");
    assert!(plugin.exists());
}

#[test]
fn the_shared_skill_stays_while_another_agent_uses_it() {
    let home = scratch();
    let all = [AgentKind::Codex, AgentKind::OpenCode, AgentKind::Pi];
    let (_, out) =
        in_home(AgentCommand::Setup, &all, AgentScope::User, false, &home).expect("setup");
    assert_eq!(out.matches("skill  installed").count(), 3, "{out}");
    let skill = home.join(".agents/skills/rusty-mux/SKILL.md");

    let (_, out) = in_home(
        AgentCommand::Remove,
        &[AgentKind::Codex],
        AgentScope::User,
        false,
        &home,
    )
    .expect("remove codex");
    assert!(out.contains("codex    skill  shared"), "{out}");
    assert!(skill.exists());
    let (_, out) = in_home(
        AgentCommand::Remove,
        &[AgentKind::OpenCode, AgentKind::Pi],
        AgentScope::User,
        false,
        &home,
    )
    .expect("remove the rest");
    assert!(out.contains("opencode skill  removed"), "{out}");
    assert!(tree(&home).is_empty(), "{:?}", tree(&home));
}

#[test]
fn dry_run_reports_and_writes_nothing() {
    let home = scratch();
    let all = AgentKind::ALL;
    let (exit, out) =
        in_home(AgentCommand::Setup, &all, AgentScope::Project, true, &home).expect("dry setup");
    assert_eq!(exit, Exit::Ok);
    assert!(out.starts_with("dry run: nothing written\n"), "{out}");
    assert_eq!(out.matches(" installed ").count(), 8, "{out}");
    assert!(tree(&home).is_empty(), "{:?}", tree(&home));

    in_home(AgentCommand::Setup, &all, AgentScope::Project, false, &home).expect("setup");
    let before = tree(&home);
    assert_eq!(before.len(), 6, "{before:?}");
    let (_, out) =
        in_home(AgentCommand::Remove, &all, AgentScope::Project, true, &home).expect("dry remove");
    assert_eq!(out.matches(" removed ").count(), 8, "{out}");
    assert_eq!(tree(&home), before, "nothing removed");

    let mut env = env(CTL);
    env.here.dir = Some(home.join("project"));
    let args = AgentArgs {
        agents: vec![AgentKind::Pi],
        scope: AgentScope::Project,
        dry_run: true,
    };
    let mut out = Vec::new();
    run(&AgentCommand::Setup(args), &env, true, &mut out).expect("json");
    let first = String::from_utf8(out).expect("UTF-8");
    let first = json(first.lines().next().expect("a line"));
    assert_eq!(first["agent"].as_str(), Some("pi"));
    assert_eq!(first["dry_run"], Value::Bool(true));
}

#[test]
fn status_names_an_unreadable_item_instead_of_failing() {
    let home = scratch();
    let codex = home.join(".codex");
    fs::create_dir_all(&codex).expect("dir");
    fs::write(codex.join("hooks.json"), "{ not json").expect("write");
    let (exit, out) = in_home(
        AgentCommand::Status,
        &[AgentKind::Codex],
        AgentScope::User,
        false,
        &home,
    )
    .expect("status reports");
    assert_eq!(exit, Exit::Failed);
    assert!(out.contains("codex    hook   unreadable"), "{out}");
    assert!(out.contains("codex    skill  missing"), "{out}");
    let setup = in_home(
        AgentCommand::Setup,
        &[AgentKind::Codex],
        AgentScope::User,
        false,
        &home,
    );
    assert!(setup.is_err(), "setup refuses it with the reason");
}

/// Review on C39: removing Claude Code alone needs no home directory, as
/// in C34, though other agents' skills live under it.
#[test]
fn removing_claude_alone_needs_no_home() {
    let dir = scratch();
    let mut env = env(CTL);
    env.agent_homes.claude = Some(dir.clone());
    let args = AgentArgs {
        agents: vec![AgentKind::Claude],
        scope: AgentScope::User,
        dry_run: false,
    };
    let mut out = Vec::new();
    run(&AgentCommand::Setup(args.clone()), &env, false, &mut out).expect("setup");
    run(&AgentCommand::Remove(args), &env, false, &mut out).expect("remove");
    assert!(files_in(&dir).is_empty(), "{:?}", files_in(&dir));
}

/// Review on C39: run from the home directory, project scope's shared skill
/// is user scope's, so removing one agent there keeps it for another agent
/// set up for the user.
#[test]
fn the_shared_skill_is_kept_for_another_scope() {
    let home = scratch();
    let mut env = env(CTL);
    env.agent_homes.home = Some(home.clone());
    env.here.dir = Some(home.clone());
    let args = |agents: &[AgentKind], scope| AgentArgs {
        agents: agents.to_vec(),
        scope,
        dry_run: false,
    };
    let mut out = Vec::new();
    let both = [AgentKind::Codex, AgentKind::OpenCode];
    run(
        &AgentCommand::Setup(args(&both, AgentScope::User)),
        &env,
        false,
        &mut out,
    )
    .expect("setup");
    let mut out = Vec::new();
    let remove = AgentCommand::Remove(args(&[AgentKind::Codex], AgentScope::Project));
    run(&remove, &env, false, &mut out).expect("remove");
    let out = String::from_utf8(out).expect("UTF-8");
    assert!(out.contains("codex    skill  shared"), "{out}");
    assert!(home.join(".agents/skills/rusty-mux/SKILL.md").exists());
}

/// Review on C39: another agent's file that cannot be read stops remove
/// with a reason naming why it was read.
#[test]
fn a_broken_neighbour_is_named_as_the_reason() {
    let home = scratch();
    in_home(
        AgentCommand::Setup,
        &[AgentKind::OpenCode],
        AgentScope::User,
        false,
        &home,
    )
    .expect("setup");
    fs::create_dir_all(home.join(".codex")).expect("dir");
    fs::write(home.join(".codex/hooks.json"), "{ broken").expect("write");
    let err = in_home(
        AgentCommand::Remove,
        &[AgentKind::OpenCode],
        AgentScope::User,
        false,
        &home,
    )
    .expect_err("refused");
    assert!(
        err.contains("still used") && err.contains("hooks.json"),
        "{err}"
    );
}

/// Review on C39, round 2: the other direction. Run from the home
/// directory, a user-scope remove keeps the skill a project there uses.
#[test]
fn the_shared_skill_is_kept_for_a_project_in_the_home_directory() {
    let home = scratch();
    let mut env = env(CTL);
    env.agent_homes.home = Some(home.clone());
    env.here.dir = Some(home.clone());
    let args = |agent, scope| AgentArgs {
        agents: vec![agent],
        scope,
        dry_run: false,
    };
    let mut out = Vec::new();
    let project = AgentCommand::Setup(args(AgentKind::OpenCode, AgentScope::Project));
    run(&project, &env, false, &mut out).expect("project setup");
    let user = AgentCommand::Setup(args(AgentKind::Codex, AgentScope::User));
    run(&user, &env, false, &mut out).expect("user setup");
    let mut out = Vec::new();
    let remove = AgentCommand::Remove(args(AgentKind::Codex, AgentScope::User));
    run(&remove, &env, false, &mut out).expect("remove");
    let out = String::from_utf8(out).expect("UTF-8");
    assert!(out.contains("codex    skill  shared"), "{out}");
    assert!(home.join(".agents/skills/rusty-mux/SKILL.md").exists());
}

/// Review on #1372: a project in the home directory keeps its skill when
/// user scope is removed from anywhere else.
#[test]
fn the_shared_skill_is_kept_for_the_home_project_from_any_directory() {
    let home = scratch();
    let elsewhere = home.join("work");
    fs::create_dir_all(&elsewhere).expect("dir");
    let mut env = env(CTL);
    env.agent_homes.home = Some(home.clone());
    let args = |agent, scope| AgentArgs {
        agents: vec![agent],
        scope,
        dry_run: false,
    };
    let mut out = Vec::new();
    env.here.dir = Some(home.clone());
    let project = AgentCommand::Setup(args(AgentKind::OpenCode, AgentScope::Project));
    run(&project, &env, false, &mut out).expect("project setup");
    env.here.dir = Some(elsewhere);
    let user = AgentCommand::Setup(args(AgentKind::Codex, AgentScope::User));
    run(&user, &env, false, &mut out).expect("user setup");
    let mut out = Vec::new();
    let remove = AgentCommand::Remove(args(AgentKind::Codex, AgentScope::User));
    run(&remove, &env, false, &mut out).expect("remove");
    let out = String::from_utf8(out).expect("UTF-8");
    assert!(out.contains("codex    skill  shared"), "{out}");
    assert!(home.join(".agents/skills/rusty-mux/SKILL.md").exists());
}

/// Review on #1372: a backup remove cannot read stops it, dry run or not,
/// before anything is deleted.
#[test]
fn a_bad_backup_stops_remove_before_any_change() {
    let home = scratch();
    let codex = home.join(".codex");
    fs::create_dir_all(&codex).expect("dir");
    fs::write(codex.join("hooks.json"), "{}").expect("write");
    let agents = [AgentKind::Codex];
    in_home(AgentCommand::Setup, &agents, AgentScope::User, false, &home).expect("setup");
    fs::write(codex.join("hooks.json.rusty-mux.bak"), "{ broken").expect("write");
    let before = tree(&home);
    for dry_run in [true, false] {
        let removed = in_home(
            AgentCommand::Remove,
            &agents,
            AgentScope::User,
            dry_run,
            &home,
        );
        let err = removed.expect_err("refused");
        assert!(err.contains("hooks.json.rusty-mux.bak"), "{err}");
        assert_eq!(tree(&home), before, "dry run {dry_run}");
    }
}

/// Review on #1372: a hook someone already took out still leaves remove to
/// drop its backup, so a later setup backs up the settings as they are.
#[test]
fn remove_drops_the_backup_when_the_hook_is_already_gone() {
    let dir = scratch();
    fs::write(dir.join("settings.json"), r#"{"a":1}"#).expect("write");
    agent(AgentCommand::Setup, &dir, CTL);
    fs::write(dir.join("settings.json"), r#"{"a":2}"#).expect("write");
    let (exit, out) = agent(AgentCommand::Remove, &dir, CTL);
    assert_eq!(exit, Exit::Ok);
    assert!(out.contains("hook   absent"), "{out}");
    assert_eq!(files_in(&dir), ["settings.json"]);
}

/// Review on #1372: Claude Code's skill can be the shared one, when its
/// directory is `~/.agents`; neither agent's remove takes it from the other.
#[test]
fn claude_can_share_the_skill_too() {
    let home = scratch();
    let mut env = env(CTL);
    env.agent_homes.home = Some(home.clone());
    env.agent_homes.claude = Some(home.join(".agents"));
    let args = |agent| AgentArgs {
        agents: vec![agent],
        scope: AgentScope::User,
        dry_run: false,
    };
    let mut out = Vec::new();
    for agent in [AgentKind::Claude, AgentKind::Codex] {
        run(&AgentCommand::Setup(args(agent)), &env, false, &mut out).expect("setup");
    }
    let skill = home.join(".agents/skills/rusty-mux/SKILL.md");
    let mut out = Vec::new();
    run(
        &AgentCommand::Remove(args(AgentKind::Claude)),
        &env,
        false,
        &mut out,
    )
    .expect("remove");
    let out = String::from_utf8(out).expect("UTF-8");
    assert!(out.contains("claude   skill  shared"), "{out}");
    assert!(skill.exists());
    let mut out = Vec::new();
    run(
        &AgentCommand::Remove(args(AgentKind::Codex)),
        &env,
        false,
        &mut out,
    )
    .expect("remove");
    assert!(!skill.exists());
}

/// Review on #1372: Codex on Windows runs a hook in the session's shell
/// (PowerShell, `cmd.exe` or Git Bash), so its Windows command is a plain
/// path all three run, and a path one of them would misread is refused.
#[test]
fn codex_gets_a_windows_command_every_windows_shell_runs() {
    assert_eq!(
        settings::windows_command(r"C:\Users\me\.cargo\bin\rusty-mux-ctl.exe"),
        Ok("C:/Users/me/.cargo/bin/rusty-mux-ctl.exe agent hook".to_owned())
    );
    for bad in [
        r"C:\Program Files\rusty-mux-ctl.exe",
        r"C:\a&b\rusty-mux-ctl.exe",
        r"C:\it's\rusty-mux-ctl.exe",
        r"C:\$x\rusty-mux-ctl.exe",
        r"C:\a(b)\rusty-mux-ctl.exe",
        r"C:\a%b%\rusty-mux-ctl.exe",
        "",
    ] {
        assert!(settings::windows_command(bad).is_err(), "{bad}");
    }
    // Ours with the Windows command, and updated when it changes.
    let windows = "C:/new/rusty-mux-ctl.exe agent hook";
    let added = settings::with_hook(json("{}"), &ours(), Some(windows)).expect("adds");
    let hook = &added["hooks"]["SessionStart"][0]["hooks"][0];
    assert_eq!(hook["commandWindows"].as_str(), Some(windows));
    assert_eq!(
        settings::state(&added, &ours(), Some(windows)),
        Ok(State::Installed)
    );
    assert_eq!(settings::state(&added, &ours(), None), Ok(State::Outdated));
    let other = "C:/old/rusty-mux-ctl.exe agent hook";
    assert_eq!(
        settings::state(&added, &ours(), Some(other)),
        Ok(State::Outdated)
    );
    let moved = settings::with_hook(added, &ours(), Some(other)).expect("updates");
    assert_eq!(
        settings::state(&moved, &ours(), Some(other)),
        Ok(State::Installed)
    );
    let back = settings::with_hook(moved, &ours(), None).expect("updates");
    let hook = &back["hooks"]["SessionStart"][0]["hooks"][0];
    assert!(hook.get("commandWindows").is_none(), "{back:?}");
    assert_eq!(settings::without_hook(back, None), Ok(json("{}")));
}

/// Review on #1372: on Windows, a ctl path Codex's shells would misread
/// makes Codex's hook `unsupported`: setup refuses it before writing, and
/// the other agents and remove are unaffected.
#[test]
fn codex_on_windows_refuses_a_path_its_shells_misread() {
    let homes = AgentHomes {
        home: Some("/h".into()),
        ..AgentHomes::default()
    };
    let on_windows = |agent, ctl| {
        layout::layout_on(agent, &AgentScope::User, &homes, None, ctl, true).expect("layout")
    };
    let spaced = r"C:\Program Files\rusty-mux\rusty-mux-ctl.exe";
    let codex = on_windows(AgentKind::Codex, spaced);
    assert!(codex.unsupported.is_some());
    assert!(on_windows(AgentKind::Claude, spaced).unsupported.is_none());
    let plain = on_windows(AgentKind::Codex, r"C:\bin\rusty-mux-ctl.exe");
    assert!(plain.unsupported.is_none());
    let Hook::Settings { windows, .. } = &plain.hook else {
        panic!("Codex takes a settings hook");
    };
    assert_eq!(
        windows.as_deref(),
        Some("C:/bin/rusty-mux-ctl.exe agent hook")
    );

    let lines = status(std::slice::from_ref(&codex));
    assert_eq!(lines[1].word, "unsupported");
    let err = setup(&[codex], false).err().expect("refused");
    assert!(err.contains("Program Files"), "{err}");
}
