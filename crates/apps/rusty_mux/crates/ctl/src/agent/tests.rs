use std::sync::atomic::{AtomicUsize, Ordering};

use rusty_uuid::Uuid;

use super::*;

const CTL: &str = "/opt/rusty mux/bin/rusty-mux-ctl";

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

/// Runs `command` in `dir`, returning the exit and stdout.
fn agent(command: fn(Option<PathBuf>) -> AgentCommand, dir: &Path, ctl: &str) -> (Exit, String) {
    let mut out = Vec::new();
    let exit = run(&command(Some(dir.into())), &env(ctl), false, &mut out).expect("runs");
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
    let after = settings::with_hook(before.clone(), &ours()).expect("adds");
    assert_eq!(after["model"], before["model"]);
    assert_eq!(after["hooks"]["Stop"], before["hooks"]["Stop"]);
    let groups = after["hooks"]["SessionStart"].as_array().expect("array");
    assert_eq!(groups[0], before["hooks"]["SessionStart"][0]);
    assert_eq!(
        groups[1]["hooks"][0]["command"].as_str(),
        Some(ours().as_str())
    );
    assert_eq!(settings::state(&after, &ours()), Ok(State::Installed));
    assert_eq!(
        settings::with_hook(after.clone(), &ours()),
        Ok(after),
        "idempotent"
    );
}

#[test]
fn a_moved_ctl_is_updated_in_place_not_added_twice() {
    let old =
        settings::with_hook(json("{}"), &settings::command("/old/rusty-mux-ctl")).expect("adds");
    assert_eq!(settings::state(&old, &ours()), Ok(State::Outdated));
    let new = settings::with_hook(old, &ours()).expect("updates");
    let groups = new["hooks"]["SessionStart"].as_array().expect("array");
    assert_eq!(groups.len(), 1);
    assert_eq!(settings::state(&new, &ours()), Ok(State::Installed));
}

#[test]
fn removing_drops_only_ours_and_the_containers_it_empties() {
    let mine =
        json(r#"{"hooks":{"SessionStart":[{"hooks":[{"type":"command","command":"echo hi"}]}]}}"#);
    let with = settings::with_hook(mine.clone(), &ours()).expect("adds");
    assert_eq!(settings::without_hook(with, None), Ok(mine.clone()));
    let alone = settings::with_hook(json(r#"{"a":1}"#), &ours()).expect("adds");
    assert_eq!(settings::without_hook(alone, None), Ok(json(r#"{"a":1}"#)));
    // A group shared with someone else's hook keeps theirs.
    let shared = json(&format!(
        r#"{{"hooks":{{"SessionStart":[{{"hooks":[{{"type":"command","command":"echo hi"}},{{"type":"command","command":"{}"}}]}}]}}}}"#,
        ours()
    ));
    assert_eq!(settings::without_hook(shared, None), Ok(mine));
}

#[test]
fn a_shape_it_cannot_edit_is_refused() {
    for bad in [r"[]", r#"{"hooks":[]}"#, r#"{"hooks":{"SessionStart":{}}}"#] {
        assert!(settings::with_hook(json(bad), &ours()).is_err(), "{bad}");
        assert!(settings::without_hook(json(bad), None).is_err(), "{bad}");
    }
}

#[test]
fn setup_then_remove_leaves_an_empty_directory_empty() {
    let dir = scratch();
    let (exit, out) = agent(AgentCommand::Setup, &dir, CTL);
    assert_eq!(exit, Exit::Ok);
    assert!(out.starts_with("skill  installed"), "{out}");
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
        &AgentCommand::Setup(Some(dir.clone())),
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
            &AgentCommand::Setup(Some(dir.clone())),
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
            settings::state(&settings, &ours()),
            Ok(State::Missing),
            "{theirs}"
        );
        let added = settings::with_hook(settings.clone(), &ours()).expect("adds");
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
        let old = settings::with_hook(json("{}"), &settings::command(moved)).expect("adds");
        assert_eq!(
            settings::state(&old, &ours()),
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
/// grants: a settings file with an explicit owner-only DACL, in the
/// broader temp directory, stays so through setup, its backup and remove.
#[cfg(windows)]
#[test]
fn private_settings_stay_private_on_windows() {
    // The access entries icacls lists, without the path it echoes first
    // (which may itself contain words like `Users`).
    let acl = |path: &Path| -> Vec<String> {
        let out = std::process::Command::new("icacls")
            .arg(path)
            .output()
            .expect("icacls");
        assert!(out.status.success(), "{out:?}");
        let shown = path.display().to_string();
        String::from_utf8_lossy(&out.stdout)
            .lines()
            .map(|line| {
                line.trim()
                    .strip_prefix(&shown)
                    .unwrap_or(line)
                    .trim()
                    .to_owned()
            })
            .filter(|entry| entry.contains(":("))
            .collect()
    };
    let user = std::env::var("USERNAME").expect("USERNAME").to_lowercase();
    let owner_only = |path: &Path| {
        let entries = acl(path);
        let only_ours = matches!(
            entries.as_slice(),
            [entry] if entry.to_lowercase().ends_with(&format!("\\{user}:(f)"))
        );
        assert!(only_ours, "{}: {entries:?}", path.display());
    };
    let dir = scratch();
    assert!(
        acl(&dir).len() > 1,
        "the parent is broader: {:?}",
        acl(&dir)
    );
    let settings = dir.join("settings.json");
    fs::write(&settings, "{}").expect("write");
    platform_windows::fs::restrict_to_owner(&settings).expect("restrict");
    owner_only(&settings);

    assert_eq!(agent(AgentCommand::Setup, &dir, CTL).0, Exit::Ok);
    owner_only(&settings);
    owner_only(&dir.join("settings.json.rusty-mux.bak"));
    agent(AgentCommand::Remove, &dir, CTL);
    owner_only(&settings);
    assert_eq!(fs::read_to_string(&settings).expect("read"), "{}");
}
