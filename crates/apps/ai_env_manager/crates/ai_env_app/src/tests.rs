use super::*;
use ai_env_core::Kind;
use std::fs;
use std::sync::atomic::{AtomicU32, Ordering};

/// A scratch folder, removed on drop.
struct Tmp(PathBuf);

impl Tmp {
    fn new() -> Self {
        static N: AtomicU32 = AtomicU32::new(0);
        let dir = env::temp_dir().join(format!(
            "aienv-app-{}-{}",
            std::process::id(),
            N.fetch_add(1, Ordering::Relaxed)
        ));
        fs::create_dir_all(&dir).unwrap();
        Tmp(dir)
    }
}

impl Drop for Tmp {
    fn drop(&mut self) {
        let _ = fs::remove_dir_all(&self.0);
    }
}

fn fixture(t: &Tmp) -> (Entry, Vec<Tool>) {
    let source = t.0.join("src/code-review");
    fs::create_dir_all(source.join("refs")).unwrap();
    fs::write(source.join("SKILL.md"), "one\ntwo\n").unwrap();
    fs::write(source.join("refs/a.md"), "a").unwrap();
    let entry = Entry {
        id: "code-review".parse().unwrap(),
        kind: Kind::Skill,
        source,
    };
    let tool = Tool {
        id: "claude-code".parse().unwrap(),
        skills_dir: t.0.join("home/skills"),
    };
    (entry, vec![tool])
}

fn tool_place() -> Place {
    Place::Tool("claude-code".parse().unwrap())
}

fn enable_by_copy(e: &Entry, tools: &[Tool]) {
    let p = plan_enable(e, &tools[0], Observed::Absent, LinkMethod::Copy);
    apply(&p).unwrap();
}

#[test]
fn lists_source_files_as_editable() {
    let tmp = Tmp::new();
    let (e, tools) = fixture(&tmp);
    let l = list_files(&e, &Place::Source, &tools).unwrap();
    assert_eq!(l.ownership, Ownership::Source);
    let names: Vec<_> = l.files.iter().map(|f| f.path.as_str()).collect();
    assert_eq!(names, ["SKILL.md", "refs/a.md"]);
    assert!(l.files.iter().all(|f| f.editable));
}

#[test]
fn foreign_target_files_are_listed_and_read_but_not_editable() {
    let tmp = Tmp::new();
    let (e, tools) = fixture(&tmp);
    let target = tools[0].target(&e);
    fs::create_dir_all(&target).unwrap();
    fs::write(target.join("SKILL.md"), "theirs").unwrap();
    let l = list_files(&e, &tool_place(), &tools).unwrap();
    assert_eq!(l.ownership, Ownership::Unmanaged);
    assert!(l.files.iter().all(|f| !f.editable));
    let o = read_file(&e, &tool_place(), &tools, "SKILL.md").unwrap();
    let r = edit_apply(
        &e,
        &tool_place(),
        &tools,
        "SKILL.md",
        "mine".into(),
        &o.hash,
    );
    assert!(matches!(r, Err(Error::Edit(EditRefusal::Unmanaged))));
    assert_eq!(
        fs::read_to_string(target.join("SKILL.md")).unwrap(),
        "theirs"
    );
}

#[test]
fn a_tool_without_the_skill_or_an_unknown_tool_is_an_error() {
    let tmp = Tmp::new();
    let (e, tools) = fixture(&tmp);
    assert!(matches!(
        list_files(&e, &tool_place(), &tools),
        Err(Error::NotEnabled(_))
    ));
    let other = Place::Tool("nope".parse().unwrap());
    assert!(matches!(
        list_files(&e, &other, &tools),
        Err(Error::UnknownTool(_))
    ));
}

#[test]
fn preview_shows_the_diff_and_writes_nothing() {
    let tmp = Tmp::new();
    let (e, tools) = fixture(&tmp);
    let o = read_file(&e, &Place::Source, &tools, "SKILL.md").unwrap();
    let p = edit_preview(
        &e,
        &Place::Source,
        &tools,
        "SKILL.md",
        "one\nTWO\n".into(),
        &o.hash,
    )
    .unwrap();
    assert_eq!(
        p.diff,
        [
            DiffLine::Same("one".into()),
            DiffLine::Removed("two".into()),
            DiffLine::Added("TWO".into()),
            DiffLine::Same("".into()),
        ]
    );
    assert!(!p.marks_edited);
    let now = fs::read_to_string(e.source.join("SKILL.md")).unwrap();
    assert_eq!(now, "one\ntwo\n");
}

#[test]
fn apply_saves_and_a_second_apply_with_the_old_hash_is_refused() {
    let tmp = Tmp::new();
    let (e, tools) = fixture(&tmp);
    let o = read_file(&e, &Place::Source, &tools, "SKILL.md").unwrap();
    edit_apply(
        &e,
        &Place::Source,
        &tools,
        "SKILL.md",
        "new".into(),
        &o.hash,
    )
    .unwrap();
    assert_eq!(
        fs::read_to_string(e.source.join("SKILL.md")).unwrap(),
        "new"
    );
    let again = edit_apply(
        &e,
        &Place::Source,
        &tools,
        "SKILL.md",
        "newer".into(),
        &o.hash,
    );
    assert!(matches!(
        again,
        Err(Error::Fs(ai_env_fs::Error::Changed(_)))
    ));
    assert_eq!(
        fs::read_to_string(e.source.join("SKILL.md")).unwrap(),
        "new"
    );
}

#[test]
fn each_refusal_reaches_the_caller_and_changes_nothing() {
    let tmp = Tmp::new();
    let (e, tools) = fixture(&tmp);
    let o = read_file(&e, &Place::Source, &tools, "SKILL.md").unwrap();
    let go =
        |path: &str, text: &str| edit_apply(&e, &Place::Source, &tools, path, text.into(), &o.hash);
    assert!(matches!(
        go("../x", "t"),
        Err(Error::Edit(EditRefusal::BadPath(_)))
    ));
    assert!(matches!(
        go("/etc/passwd", "t"),
        Err(Error::Edit(EditRefusal::BadPath(_)))
    ));
    assert!(matches!(
        go("SKILL.md", "one\ntwo\n"),
        Err(Error::Edit(EditRefusal::NoChange))
    ));
    assert!(matches!(
        go("SKILL.md", "a\0b"),
        Err(Error::Edit(EditRefusal::NotText))
    ));
    assert!(matches!(
        go("missing.md", "t"),
        Err(Error::Fs(ai_env_fs::Error::Io(..)))
    ));
    assert!(matches!(
        go("refs", "t"),
        Err(Error::Fs(ai_env_fs::Error::NotFile(_)))
    ));
    assert_eq!(
        fs::read_to_string(e.source.join("SKILL.md")).unwrap(),
        "one\ntwo\n"
    );
}

#[test]
fn the_marker_file_is_never_editable() {
    let tmp = Tmp::new();
    let (e, tools) = fixture(&tmp);
    enable_by_copy(&e, &tools);
    let r = edit_apply(&e, &tool_place(), &tools, ".aienv-managed", "x".into(), "h");
    assert!(matches!(
        r,
        Err(Error::Fs(_) | Error::Edit(EditRefusal::MarkerFile))
    ));
    let listed = list_files(&e, &tool_place(), &tools).unwrap();
    assert!(listed.files.iter().all(|f| !f.path.is_marker()));
}

#[test]
fn editing_a_marked_copy_previews_the_edited_warning_and_blocks_disable() {
    let tmp = Tmp::new();
    let (e, tools) = fixture(&tmp);
    enable_by_copy(&e, &tools);
    let place = tool_place();
    let o = read_file(&e, &place, &tools, "SKILL.md").unwrap();
    assert_eq!(o.ownership, Ownership::MarkedCopy);
    let p = edit_preview(&e, &place, &tools, "SKILL.md", "x".into(), &o.hash).unwrap();
    assert!(p.marks_edited);
    edit_apply(&e, &place, &tools, "SKILL.md", "x".into(), &o.hash).unwrap();
    assert_eq!(observe(&e, &tools[0]).unwrap(), Observed::ManagedModified);
    assert!(plan(&e, &tools[0], Op::Disable)
        .unwrap()
        .actions()
        .iter()
        .all(|a| !matches!(a, ai_env_core::Action::Unlink { .. })));
    // The source is untouched.
    assert_eq!(
        fs::read_to_string(e.source.join("SKILL.md")).unwrap(),
        "one\ntwo\n"
    );
}

#[cfg(unix)]
#[test]
fn a_linked_tool_edits_the_one_source_file() {
    let tmp = Tmp::new();
    let (e, tools) = fixture(&tmp);
    let p = plan_enable(&e, &tools[0], Observed::Absent, LinkMethod::Symlink);
    apply(&p).unwrap();
    let o = read_file(&e, &tool_place(), &tools, "SKILL.md").unwrap();
    assert_eq!(o.ownership, Ownership::Source);
    edit_apply(
        &e,
        &tool_place(),
        &tools,
        "SKILL.md",
        "via link".into(),
        &o.hash,
    )
    .unwrap();
    assert_eq!(
        fs::read_to_string(e.source.join("SKILL.md")).unwrap(),
        "via link"
    );
}
