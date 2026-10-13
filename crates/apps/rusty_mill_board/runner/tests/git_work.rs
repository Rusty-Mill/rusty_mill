//! Real git, real sandbox: a card's worktree, a confined command, a runner-made commit.
//! Skips (loudly) where the kernel cannot confine, rather than passing falsely.

use std::path::{Path, PathBuf};
use std::process::Command;

use rusty_mill_board::domain::Card;
use rusty_mill_runner::config::{Stage, parse};
use rusty_mill_runner::git_work::GitWork;
use rusty_mill_runner::ports::Work;

fn git(dir: &Path, args: &[&str]) -> String {
    let out = Command::new("git")
        .arg("-C")
        .arg(dir)
        .args(args)
        .output()
        .expect("git");
    assert!(
        out.status.success(),
        "git {args:?}: {}",
        String::from_utf8_lossy(&out.stderr)
    );
    String::from_utf8_lossy(&out.stdout).trim().to_owned()
}

struct Fixture {
    root: PathBuf,
    work: GitWork,
    card: Card,
}

fn fixture(name: &str) -> Fixture {
    let root = std::env::temp_dir().join(format!("mill_runner_{name}_{}", std::process::id()));
    let _ = std::fs::remove_dir_all(&root);
    let repo = root.join("repo");
    std::fs::create_dir_all(&repo).expect("repo dir");
    git(&repo, &["init", "-q", "-b", "main"]);
    git(&repo, &["config", "user.email", "t@t"]);
    git(&repo, &["config", "user.name", "t"]);
    std::fs::write(repo.join("README"), "hi\n").expect("write");
    git(&repo, &["add", "."]);
    git(&repo, &["commit", "-q", "-m", "init"]);
    let helper = PathBuf::from(env!("CARGO_BIN_EXE_mill-runner"));
    let work = GitWork::new(&repo, &root.join("work"), "main", helper, vec![]).expect("work");
    let card = Card::new("Add file", "body", vec![], 0).expect("card");
    Fixture { root, work, card }
}

fn stage(json: &str) -> Stage {
    parse(json).expect("stage").remove(0)
}

fn sh(script: &str) -> Vec<String> {
    vec!["/bin/sh".into(), "-c".into(), script.into()]
}

/// `None` when the sandbox cannot be set up here.
fn run(f: &Fixture, s: &Stage, script: &str) -> Option<rusty_mill_runner::ports::Outcome> {
    let dir = f.work.prepare(&f.card).expect("prepare");
    match f.work.run(
        s,
        &sh(script),
        &dir,
        &[("PATH".into(), "/usr/bin:/bin".into())],
    ) {
        Ok(o) => Some(o),
        Err(e)
            if e.to_lowercase().contains("sandbox")
                || e.to_lowercase().contains("landlock")
                || e.contains("not supported") =>
        {
            eprintln!("SKIPPED: sandbox unavailable here: {e}");
            None
        }
        Err(e) => panic!("run: {e}"),
    }
}

const BUILD: &str =
    r#"[{"name":"build","from":["ready"],"role":"builder","argv":["x"],"wall_secs":20}]"#;

#[test]
fn command_edits_the_worktree_and_the_runner_commits_it_on_the_card_branch() {
    let f = fixture("commit");
    let s = stage(BUILD);
    let Some(out) = run(&f, &s, "echo built > out.txt && echo done") else {
        return;
    };
    assert!(out.ok, "{out:?}");
    assert!(out.tail.contains("done"));

    let dir = f.work.prepare(&f.card).expect("dir");
    assert!(
        f.work
            .commit(&dir, "mill: Add file [build]")
            .expect("commit")
    );
    assert!(
        !f.work.commit(&dir, "again").expect("commit"),
        "nothing new to commit"
    );

    let repo = f.root.join("repo");
    let branch = format!("mill/{}", &f.card.id.to_string()[..8]);
    assert_eq!(
        git(&repo, &["log", "-1", "--format=%s", &branch]),
        "mill: Add file [build]"
    );
    assert_eq!(git(&repo, &["show", &format!("{branch}:out.txt")]), "built");
    assert!(
        !git(&repo, &["ls-tree", "-r", "--name-only", "main"]).contains("out.txt"),
        "main is untouched"
    );
    let _ = std::fs::remove_dir_all(&f.root);
}

#[test]
fn command_cannot_write_outside_its_worktree_or_into_git() {
    let f = fixture("escape");
    let s = stage(BUILD);
    let outside = f.root.join("escaped.txt");
    let Some(out) = run(
        &f,
        &s,
        &format!(
            "echo x > {}; echo y > {}/repo/.git/pwned; echo z > ../sibling.txt",
            outside.display(),
            f.root.display()
        ),
    ) else {
        return;
    };
    assert!(!out.ok, "writes outside the sandbox must fail: {out:?}");
    assert!(!outside.exists());
    assert!(!f.root.join("repo/.git/pwned").exists());
    assert!(!f.root.join("work/sibling.txt").exists());
    let _ = std::fs::remove_dir_all(&f.root);
}

#[test]
fn failure_and_timeout_are_outcomes_not_errors() {
    let f = fixture("outcomes");
    let Some(bad) = run(&f, &stage(BUILD), "echo boom >&2; exit 3") else {
        return;
    };
    assert!(
        !bad.ok && bad.summary.starts_with("exit 3") && bad.tail.contains("boom"),
        "{bad:?}"
    );
    let slow =
        stage(r#"[{"name":"build","from":["ready"],"role":"builder","argv":["x"],"wall_secs":1}]"#);
    let Some(timed) = run(&f, &slow, "sleep 30") else {
        return;
    };
    assert!(
        !timed.ok && timed.summary.contains("timed out"),
        "{timed:?}"
    );
    let _ = std::fs::remove_dir_all(&f.root);
}
