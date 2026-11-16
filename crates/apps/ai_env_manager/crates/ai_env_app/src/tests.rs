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

/// A marked copy whose SKILL.md was edited to `text`.
fn edited_copy(e: &Entry, tools: &[Tool], text: &str) -> PathBuf {
    enable_by_copy(e, tools);
    let copy = tools[0].target(e);
    fs::write(copy.join("SKILL.md"), text).unwrap();
    copy
}

fn tool_id() -> ToolId {
    "claude-code".parse().unwrap()
}

#[test]
fn push_preview_diffs_source_to_copy_and_writes_nothing() {
    let tmp = Tmp::new();
    let (e, tools) = fixture(&tmp);
    edited_copy(&e, &tools, "one\nTWO\n");
    let p = push_preview(&e, &tool_id(), &tools, None).unwrap();
    assert_eq!(p.files.len(), 1);
    assert_eq!(p.files[0].base.path, "SKILL.md");
    assert!(p.files[0].diff.contains(&DiffLine::Added("TWO".into())));
    assert!(p.left.is_empty());
    assert_eq!(
        fs::read_to_string(e.source.join("SKILL.md")).unwrap(),
        "one\ntwo\n"
    );
}

#[test]
fn push_apply_writes_the_source_and_the_copy_reads_enabled_again() {
    let tmp = Tmp::new();
    let (e, tools) = fixture(&tmp);
    edited_copy(&e, &tools, "one\nTWO\n");
    assert_eq!(observe(&e, &tools[0]).unwrap(), Observed::ManagedModified);
    let p = push_preview(&e, &tool_id(), &tools, None).unwrap();
    let bases: Vec<_> = p.files.iter().map(|f| f.base.clone()).collect();
    let done = push_apply(&e, &tool_id(), &tools, &bases).unwrap();
    assert!(done.remarked && done.left.is_empty());
    assert_eq!(done.written, ["SKILL.md"]);
    assert_eq!(
        fs::read_to_string(e.source.join("SKILL.md")).unwrap(),
        "one\nTWO\n"
    );
    assert_eq!(observe(&e, &tools[0]).unwrap(), Observed::ManagedCurrent);
}

#[test]
fn a_partial_push_leaves_the_copy_edited() {
    let tmp = Tmp::new();
    let (e, tools) = fixture(&tmp);
    let copy = edited_copy(&e, &tools, "one\nTWO\n");
    fs::write(copy.join("refs/a.md"), "A").unwrap();
    let p = push_preview(&e, &tool_id(), &tools, Some("SKILL.md")).unwrap();
    assert_eq!(p.files.len(), 1);
    assert_eq!(p.left[0].path, "refs/a.md");
    let bases: Vec<_> = p.files.iter().map(|f| f.base.clone()).collect();
    let done = push_apply(&e, &tool_id(), &tools, &bases).unwrap();
    assert!(!done.remarked);
    assert_eq!(done.left.len(), 1);
    assert_eq!(observe(&e, &tools[0]).unwrap(), Observed::ManagedModified);
    assert_eq!(fs::read_to_string(e.source.join("refs/a.md")).unwrap(), "a");
}

#[test]
fn files_on_one_side_only_are_listed_and_never_pushed() {
    let tmp = Tmp::new();
    let (e, tools) = fixture(&tmp);
    let copy = edited_copy(&e, &tools, "one\nTWO\n");
    fs::write(copy.join("new.md"), "n").unwrap();
    let p = push_preview(&e, &tool_id(), &tools, None).unwrap();
    assert_eq!(p.left.len(), 1);
    assert_eq!(p.left[0].delta, Delta::OnlyInCopy);
    let bases: Vec<_> = p.files.iter().map(|f| f.base.clone()).collect();
    let done = push_apply(&e, &tool_id(), &tools, &bases).unwrap();
    assert!(!done.remarked);
    assert!(!e.source.join("new.md").exists());
}

#[test]
fn push_is_refused_unless_the_copy_is_edited() {
    let tmp = Tmp::new();
    let (e, tools) = fixture(&tmp);
    assert!(matches!(
        push_preview(&e, &tool_id(), &tools, None),
        Err(Error::NotEnabled(_))
    ));
    enable_by_copy(&e, &tools);
    assert!(matches!(
        push_preview(&e, &tool_id(), &tools, None),
        Err(Error::NotEdited(_))
    ));
    // A foreign folder is never read as ours.
    fs::remove_dir_all(tools[0].target(&e)).unwrap();
    fs::create_dir_all(tools[0].target(&e)).unwrap();
    fs::write(tools[0].target(&e).join("SKILL.md"), "theirs").unwrap();
    assert!(matches!(
        push_preview(&e, &tool_id(), &tools, None),
        Err(Error::NotEdited(_))
    ));
}

#[test]
fn push_apply_refuses_when_either_side_changed_since_the_preview() {
    let tmp = Tmp::new();
    let (e, tools) = fixture(&tmp);
    let copy = edited_copy(&e, &tools, "one\nTWO\n");
    let p = push_preview(&e, &tool_id(), &tools, None).unwrap();
    let bases: Vec<_> = p.files.iter().map(|f| f.base.clone()).collect();
    fs::write(e.source.join("SKILL.md"), "someone else\n").unwrap();
    assert!(matches!(
        push_apply(&e, &tool_id(), &tools, &bases),
        Err(Error::Fs(ai_env_fs::Error::Changed(_)))
    ));
    assert_eq!(
        fs::read_to_string(e.source.join("SKILL.md")).unwrap(),
        "someone else\n"
    );
    fs::write(e.source.join("SKILL.md"), "one\ntwo\n").unwrap();
    fs::write(copy.join("SKILL.md"), "one\nTHREE\n").unwrap();
    assert!(push_apply(&e, &tool_id(), &tools, &bases).is_err());
}

#[test]
fn pushing_a_file_that_does_not_differ_or_escapes_is_refused() {
    let tmp = Tmp::new();
    let (e, tools) = fixture(&tmp);
    edited_copy(&e, &tools, "one\nTWO\n");
    assert!(matches!(
        push_preview(&e, &tool_id(), &tools, Some("refs/a.md")),
        Err(Error::NothingToPush(_))
    ));
    assert!(matches!(
        push_preview(&e, &tool_id(), &tools, Some("../x")),
        Err(Error::Edit(EditRefusal::BadPath(_)))
    ));
}
