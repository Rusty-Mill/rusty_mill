#![allow(clippy::unwrap_used, clippy::expect_used)]
//! Against the real git CLI.

use rusty_worktree::{Author, Repo};
use std::path::{Path, PathBuf};
use std::process::Command;

fn git(dir: &Path, args: &[&str]) -> String {
    let out = Command::new("git")
        .arg("-C")
        .arg(dir)
        .args(args)
        .output()
        .unwrap();
    assert!(
        out.status.success(),
        "git {args:?}: {}",
        String::from_utf8_lossy(&out.stderr)
    );
    String::from_utf8_lossy(&out.stdout).trim().to_owned()
}

fn repo(tag: &str) -> (PathBuf, Repo) {
    let root = std::env::temp_dir().join(format!("rusty_worktree_{}_{tag}", std::process::id()));
    let _ = std::fs::remove_dir_all(&root);
    let repo = root.join("repo");
    std::fs::create_dir_all(&repo).unwrap();
    git(&repo, &["init", "-q", "-b", "main"]);
    git(&repo, &["config", "user.name", "t"]);
    git(&repo, &["config", "user.email", "t@example.invalid"]);
    std::fs::write(repo.join("flag.txt"), "no\n").unwrap();
    git(&repo, &["add", "."]);
    git(&repo, &["commit", "-qm", "base"]);
    // A hook that would mark every checkout: it must never run.
    let hook = repo.join(".git/hooks/post-checkout");
    std::fs::write(
        &hook,
        format!("#!/bin/sh\ntouch {}/hooked\n", root.display()),
    )
    .unwrap();
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        std::fs::set_permissions(&hook, std::fs::Permissions::from_mode(0o755)).unwrap();
    }
    (root, Repo::open(&repo).unwrap())
}

#[test]
fn worktrees_share_one_object_store_and_keep_their_own_changes() {
    let (root, repo) = repo("share");
    let a = repo.add_detached(&root.join("a"), "HEAD").unwrap();
    let b = repo.add_branch(&root.join("b"), "card/b", "main").unwrap();
    let common = |dir: &Path| {
        let out = git(
            dir,
            &["rev-parse", "--path-format=absolute", "--git-common-dir"],
        );
        PathBuf::from(out).canonicalize().unwrap()
    };
    assert_eq!(common(&a), repo.git_dir());
    assert_eq!(common(&b), repo.git_dir());
    assert!(!root.join("hooked").exists(), "hooks are off");

    std::fs::write(a.join("flag.txt"), "ok\n").unwrap();
    std::fs::write(a.join("new.txt"), "x\n").unwrap();
    assert_eq!(repo.changed(&a).unwrap(), ["flag.txt", "new.txt"]);
    assert!(repo.changed(&b).unwrap().is_empty());

    let author = Author {
        name: "runner",
        email: "runner@localhost",
    };
    std::fs::write(b.join("flag.txt"), "ok\n").unwrap();
    assert!(repo.commit(&b, "flag ok", author).unwrap());
    assert!(!repo.commit(&b, "nothing", author).unwrap());
    assert_eq!(
        git(repo.root(), &["log", "-1", "--format=%an %s", "card/b"]),
        "runner flag ok"
    );

    repo.discard(&a).unwrap();
    assert!(repo.changed(&a).unwrap().is_empty());
    // An existing branch worktree is reopened, not recreated.
    assert_eq!(
        repo.add_branch(&root.join("b"), "card/b", "main").unwrap(),
        b
    );

    repo.remove(&a).unwrap();
    assert!(!a.exists());
    assert!(repo.admin_dir(&a).is_err());
    let _ = std::fs::remove_dir_all(&root);
}

#[test]
fn a_rewritten_dot_git_file_cannot_redirect_the_caller() {
    let (root, repo) = repo("redirect");
    let a = repo.add_detached(&root.join("a"), "HEAD").unwrap();
    // A decoy repository the worktree's `.git` file now points at.
    let decoy = root.join("decoy");
    std::fs::create_dir_all(&decoy).unwrap();
    git(&decoy, &["init", "-q"]);
    std::fs::write(
        a.join(".git"),
        format!("gitdir: {}/.git\n", decoy.display()),
    )
    .unwrap();
    std::fs::write(a.join("flag.txt"), "ok\n").unwrap();
    let changed = repo.changed(&a).unwrap();
    assert!(changed.contains(&"flag.txt".to_owned()), "{changed:?}");
    let _ = std::fs::remove_dir_all(&root);
}

#[test]
fn a_detached_worktree_replaces_a_leftover() {
    let (root, repo) = repo("leftover");
    let dir = root.join("a");
    std::fs::create_dir_all(dir.join("junk")).unwrap();
    let a = repo.add_detached(&dir, "HEAD").unwrap();
    assert!(!a.join("junk").exists());
    assert_eq!(std::fs::read_to_string(a.join("flag.txt")).unwrap(), "no\n");
    let _ = std::fs::remove_dir_all(&root);
}
