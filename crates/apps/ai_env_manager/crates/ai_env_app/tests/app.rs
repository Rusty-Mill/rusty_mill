//! The shared steps on real temp folders: a fake home, a skills folder, the
//! built-in catalog (claude-code and codex).

use ai_env_app::{apply, find_entry, load_entries, observe, plan, select_tools, Error, Op, Scope};
use ai_env_core::{Observed, ToolId};
use std::fs;
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicU32, Ordering};

struct Tmp(PathBuf);

impl Tmp {
    fn new() -> Self {
        static N: AtomicU32 = AtomicU32::new(0);
        let dir = std::env::temp_dir().join(format!(
            "aienv-app-{}-{}",
            std::process::id(),
            N.fetch_add(1, Ordering::Relaxed)
        ));
        fs::create_dir_all(&dir).unwrap();
        Tmp(dir)
    }

    /// A skills folder holding the named skills.
    fn skills(&self, folder: &str, names: &[&str]) -> PathBuf {
        let dir = self.0.join(folder);
        for name in names {
            fs::create_dir_all(dir.join(name)).unwrap();
            fs::write(dir.join(name).join("SKILL.md"), name).unwrap();
        }
        dir
    }

    fn home(&self) -> PathBuf {
        self.0.join("home")
    }
}

impl Drop for Tmp {
    fn drop(&mut self) {
        let _ = fs::remove_dir_all(&self.0);
    }
}

fn scope(skills: Vec<PathBuf>, tools: &[&str]) -> Scope {
    Scope {
        skills,
        tools: tools
            .iter()
            .map(|t| t.parse::<ToolId>().expect("valid tool id"))
            .collect(),
    }
}

fn first_tool(home: &Path) -> ai_env_core::Tool {
    let tools = select_tools(&scope(vec![], &["claude-code"]), home).unwrap();
    tools.into_iter().next().unwrap()
}

#[test]
fn empty_tool_scope_selects_the_whole_catalog() {
    let t = Tmp::new();
    let all = select_tools(&scope(vec![], &[]), &t.home()).unwrap();
    assert!(all.len() >= 2);
}

#[test]
fn unknown_tool_is_an_error() {
    let t = Tmp::new();
    let r = select_tools(&scope(vec![], &["no-such-tool"]), &t.home());
    assert!(matches!(r, Err(Error::UnknownTool(_))));
}

#[test]
fn entries_are_sorted_and_unknown_ones_are_errors() {
    let t = Tmp::new();
    let dir = t.skills("s", &["zeta", "alpha"]);
    let loaded = load_entries(&scope(vec![dir], &[])).unwrap();
    let ids: Vec<_> = loaded.entries.iter().map(|e| e.id.to_string()).collect();
    assert_eq!(ids, ["alpha", "zeta"]);
    let missing = "nope".parse::<ai_env_core::EntryId>().unwrap();
    assert!(matches!(
        find_entry(&loaded.entries, &missing),
        Err(Error::UnknownEntry(_))
    ));
}

#[test]
fn an_id_in_two_folders_is_an_error() {
    let t = Tmp::new();
    let (a, b) = (t.skills("a", &["same"]), t.skills("b", &["same"]));
    let r = load_entries(&scope(vec![a, b], &[]));
    assert!(matches!(r, Err(Error::DuplicateEntry(_))));
}

#[test]
fn an_unreadable_folder_is_an_error_with_its_path() {
    let t = Tmp::new();
    let missing = t.0.join("missing");
    let r = load_entries(&scope(vec![missing.clone()], &[]));
    assert!(matches!(r, Err(Error::Folder(p, _)) if p == missing));
}

#[test]
fn an_invalid_folder_name_is_reported_not_dropped() {
    let t = Tmp::new();
    let dir = t.skills("s", &["good", "Bad Name"]);
    let loaded = load_entries(&scope(vec![dir], &[])).unwrap();
    assert_eq!(loaded.entries.len(), 1);
    assert_eq!(loaded.skipped.len(), 1);
}

#[test]
fn plan_writes_nothing_and_apply_round_trips() {
    let t = Tmp::new();
    let dir = t.skills("s", &["one"]);
    let loaded = load_entries(&scope(vec![dir], &[])).unwrap();
    let entry = &loaded.entries[0];
    let tool = first_tool(&t.home());

    let enable = plan(entry, &tool, Op::Enable).unwrap();
    assert_eq!(observe(entry, &tool).unwrap(), Observed::Absent);
    apply(&enable).unwrap();
    assert_eq!(observe(entry, &tool).unwrap(), Observed::ManagedCurrent);

    assert!(plan(entry, &tool, Op::Enable).unwrap().is_noop());
    apply(&plan(entry, &tool, Op::Disable).unwrap()).unwrap();
    assert_eq!(observe(entry, &tool).unwrap(), Observed::Absent);
}

#[test]
fn a_foreign_folder_is_refused_and_apply_rejects_the_plan() {
    let t = Tmp::new();
    let dir = t.skills("s", &["one"]);
    let loaded = load_entries(&scope(vec![dir], &[])).unwrap();
    let entry = &loaded.entries[0];
    let tool = first_tool(&t.home());
    let target = tool.target(entry);
    fs::create_dir_all(&target).unwrap();
    fs::write(target.join("mine.txt"), "x").unwrap();

    let p = plan(entry, &tool, Op::Enable).unwrap();
    assert!(p.has_refusals());
    assert!(apply(&p).is_err());
    assert!(target.join("mine.txt").exists());
}
