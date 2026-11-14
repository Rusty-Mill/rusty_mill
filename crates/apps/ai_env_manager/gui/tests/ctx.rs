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
