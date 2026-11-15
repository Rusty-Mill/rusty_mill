//! The commands' logic on real temp folders (a fake home and a skills folder),
//! without opening a window.

use ai_env_gui_lib::view::{Outcome, Verb};
use ai_env_gui_lib::{parse_scope, Ctx};
use std::ffi::OsString;
use std::fs;
use std::path::PathBuf;
use std::sync::atomic::{AtomicU32, Ordering};

struct Tmp(PathBuf);

impl Tmp {
    fn new() -> Self {
        static N: AtomicU32 = AtomicU32::new(0);
        let dir = std::env::temp_dir().join(format!(
            "aienv-gui-{}-{}",
            std::process::id(),
            N.fetch_add(1, Ordering::Relaxed)
        ));
        fs::create_dir_all(&dir).unwrap();
        Tmp(dir)
    }

    fn ctx(&self, names: &[&str]) -> Ctx {
        let skills = self.0.join("skills");
        for n in names {
            fs::create_dir_all(skills.join(n)).unwrap();
            fs::write(skills.join(n).join("SKILL.md"), n).unwrap();
        }
        let scope = parse_scope([OsString::from("--skills"), skills.into()]).unwrap();
        Ctx::new(scope, self.0.join("home"))
    }
}

impl Drop for Tmp {
    fn drop(&mut self) {
        let _ = fs::remove_dir_all(&self.0);
    }
}

#[test]
fn args_need_skills_and_reject_the_unknown() {
    let args = |a: &[&str]| a.iter().map(OsString::from).collect::<Vec<_>>();
    assert!(parse_scope(args(&[])).is_err());
    assert!(parse_scope(args(&["--skills"])).is_err());
    assert!(parse_scope(args(&["--skills", "d", "--open", "x"])).is_err());
    assert!(parse_scope(args(&["--skills", "d", "--tool", "Bad Id"])).is_err());
    let s = parse_scope(args(&["--skills", "a", "--skills", "b", "--tool", "codex"])).unwrap();
    assert_eq!((s.skills.len(), s.tools.len()), (2, 1));
}

#[test]
fn list_shows_every_skill_against_every_tool() {
    let t = Tmp::new();
    let v = t.ctx(&["one", "two"]).list().unwrap();
    assert_eq!(v.tools, ["claude-code", "codex"]);
    assert_eq!(v.rows.len(), 2);
    assert!(v.rows[0].cells.iter().all(|c| c.state == "absent"));
}

#[test]
fn plan_writes_nothing_then_apply_enables_and_disables() {
    let t = Tmp::new();
    let ctx = t.ctx(&["one"]);

    let plans = ctx.plan("one", Verb::Enable).unwrap();
    assert!(plans.iter().all(|p| !p.noop && !p.refused));
    assert!(ctx.list().unwrap().rows[0]
        .cells
        .iter()
        .all(|c| c.state == "absent"));

    let done = ctx.apply("one", Verb::Enable).unwrap();
    assert!(done.iter().all(|r| r.outcome == Outcome::Applied));
    assert!(ctx.list().unwrap().rows[0]
        .cells
        .iter()
        .all(|c| c.state == "enabled"));
    assert!(ctx
        .apply("one", Verb::Enable)
        .unwrap()
        .iter()
        .all(|r| r.outcome == Outcome::Nothing));

    ctx.apply("one", Verb::Disable).unwrap();
    assert!(ctx.list().unwrap().rows[0]
        .cells
        .iter()
        .all(|c| c.state == "absent"));
}

#[test]
fn a_foreign_folder_is_refused_and_left_alone_while_other_tools_apply() {
    let t = Tmp::new();
    let ctx = t.ctx(&["one"]);
    let foreign = t.0.join("home/.agents/skills/one");
    fs::create_dir_all(&foreign).unwrap();
    fs::write(foreign.join("mine.txt"), "x").unwrap();

    let plans = ctx.plan("one", Verb::Enable).unwrap();
    assert_eq!(plans.iter().filter(|p| p.refused).count(), 1);
    let done = ctx.apply("one", Verb::Enable).unwrap();
    let outcomes: Vec<_> = done.iter().map(|r| (r.tool.as_str(), r.outcome)).collect();
    assert_eq!(
        outcomes,
        [
            ("claude-code", Outcome::Applied),
            ("codex", Outcome::Refused)
        ]
    );
    assert!(foreign.join("mine.txt").exists());
}

#[test]
fn unknown_and_invalid_ids_are_errors_not_panics() {
    let t = Tmp::new();
    let ctx = t.ctx(&["one"]);
    assert!(ctx.plan("nope", Verb::Enable).is_err());
    assert!(ctx.apply("../escape", Verb::Enable).is_err());
}

#[test]
fn plan_serializes_with_stable_names() {
    let t = Tmp::new();
    let plans = t.ctx(&["one"]).plan("one", Verb::Enable).unwrap();
    let json = serde_json::to_value(&plans).unwrap();
    assert_eq!(json[0]["actions"][0]["kind"], "link");
    assert_eq!(json[0]["tool"], "claude-code");
}

#[test]
fn verbs_and_outcomes_keep_their_wire_names() {
    assert_eq!(
        serde_json::from_str::<Verb>("\"enable\"").unwrap(),
        Verb::Enable
    );
    assert!(serde_json::from_str::<Verb>("true").is_err());
    assert_eq!(
        serde_json::to_string(&Outcome::Nothing).unwrap(),
        "\"nothing\""
    );
}

#[test]
fn editor_lists_reads_previews_and_saves_a_source_file() {
    let t = Tmp::new();
    let ctx = t.ctx(&["one"]);

    let listing = ctx.files("one", None).unwrap();
    assert_eq!(listing.ownership, "source");
    assert_eq!(listing.files.len(), 1);
    assert_eq!(listing.files[0].path, "SKILL.md");
    assert!(listing.files[0].editable);

    let read = ctx.file("one", None, "SKILL.md").unwrap();
    assert_eq!(read.text, "one");

    let p = ctx
        .edit_preview("one", None, "SKILL.md", "two".into(), &read.hash)
        .unwrap();
    assert!(!p.marks_edited);
    let kinds: Vec<_> = p.diff.iter().map(|d| d.kind).collect();
    assert_eq!(kinds, ["removed", "added"]);
    // Previewing wrote nothing.
    assert_eq!(ctx.file("one", None, "SKILL.md").unwrap().text, "one");

    ctx.edit_apply("one", None, "SKILL.md", "two".into(), &read.hash)
        .unwrap();
    assert_eq!(ctx.file("one", None, "SKILL.md").unwrap().text, "two");
}

#[test]
fn editor_refuses_with_the_backends_reason_and_changes_nothing() {
    let t = Tmp::new();
    let ctx = t.ctx(&["one"]);
    let hash = ctx.file("one", None, "SKILL.md").unwrap().hash;
    let go = |path: &str, tool: Option<&str>| {
        ctx.edit_apply("one", tool, path, "x".into(), &hash)
            .unwrap_err()
    };
    assert!(go("../escape", None).contains("plain relative path"));
    assert!(go("/etc/passwd", None).contains("plain relative path"));
    assert!(go("SKILL.md", Some("Bad Id")).contains("Bad Id"));
    assert!(go("SKILL.md", Some("codex")).contains("not enabled"));
    assert!(ctx.files("nope", None).is_err());
    assert!(ctx
        .edit_apply("one", None, "SKILL.md", "x".into(), "stale-hash")
        .unwrap_err()
        .contains("changed since it was read"));
    assert_eq!(ctx.file("one", None, "SKILL.md").unwrap().text, "one");
}

#[test]
fn a_foreign_target_is_read_only_and_a_linked_tool_edits_the_source() {
    let t = Tmp::new();
    let ctx = t.ctx(&["one"]);
    let foreign = t.0.join("home/.agents/skills/one");
    fs::create_dir_all(&foreign).unwrap();
    fs::write(foreign.join("SKILL.md"), "theirs").unwrap();

    let listing = ctx.files("one", Some("codex")).unwrap();
    assert_eq!(listing.ownership, "foreign");
    assert!(listing.files.iter().all(|f| !f.editable));
    let read = ctx.file("one", Some("codex"), "SKILL.md").unwrap();
    let err = ctx
        .edit_apply("one", Some("codex"), "SKILL.md", "mine".into(), &read.hash)
        .unwrap_err();
    assert!(err.contains("not one this app placed"), "{err}");
    assert_eq!(
        fs::read_to_string(foreign.join("SKILL.md")).unwrap(),
        "theirs"
    );

    // claude-code is linked (or copied) by enable; on Unix that is a symlink
    // to the one source file, so an edit through it changes the source.
    #[cfg(unix)]
    {
        ctx.apply("one", Verb::Enable).unwrap();
        let via = ctx.file("one", Some("claude-code"), "SKILL.md").unwrap();
        assert_eq!(via.ownership, "source");
        ctx.edit_apply(
            "one",
            Some("claude-code"),
            "SKILL.md",
            "via".into(),
            &via.hash,
        )
        .unwrap();
        let src = t.0.join("skills/one/SKILL.md");
        assert_eq!(fs::read_to_string(src).unwrap(), "via");
    }
}

#[test]
fn editor_wire_names_are_stable() {
    use ai_env_core::Ownership;
    use ai_env_gui_lib::view;
    assert_eq!(view::ownership(Ownership::Source), "source");
    assert_eq!(view::ownership(Ownership::MarkedCopy), "copy");
    assert_eq!(view::ownership(Ownership::Unmanaged), "foreign");
}
