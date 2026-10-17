#![allow(clippy::expect_used, clippy::unwrap_used)]
//! `build.rs` provenance, against real cargo: the commit a binary reports must
//! be the tree it was built from. A throwaway git repository holds an app and a
//! path dependency that use this crate's own `build.rs`; each step edits the
//! tree, rebuilds with cargo and runs the binary, which prints its
//! `BBP_BUILD_SHA`. A binary built from a dirty tree must never report a clean
//! commit, whatever cargo decides to rebuild.

use std::path::{Path, PathBuf};
use std::process::Command;

const BUILD_RS: &str = include_str!("../build.rs");

fn cargo() -> String {
    std::env::var("CARGO").unwrap_or_else(|_| "cargo".into())
}

fn run(dir: &Path, program: &str, args: &[&str]) -> String {
    let out = Command::new(program)
        .current_dir(dir)
        .args(args)
        .output()
        .unwrap_or_else(|e| panic!("{program}: {e}"));
    assert!(
        out.status.success(),
        "{program} {args:?}: {}",
        String::from_utf8_lossy(&out.stderr)
    );
    String::from_utf8_lossy(&out.stdout).trim().to_owned()
}

struct Repo {
    root: PathBuf,
    target: PathBuf,
}

impl Repo {
    fn new(tag: &str) -> Repo {
        let base = std::env::temp_dir().join(format!("bbp_build_sha_{}_{tag}", std::process::id()));
        let _ = std::fs::remove_dir_all(&base);
        let root = base.join("repo");
        let target = base.join("target");
        for d in ["app/src", "dep/src"] {
            std::fs::create_dir_all(root.join(d)).unwrap();
        }
        let w = |p: &str, text: &str| std::fs::write(root.join(p), text).unwrap();
        w(
            "app/Cargo.toml",
            "[package]\nname = \"app\"\nversion = \"0.0.0\"\nedition = \"2021\"\nbuild = \"build.rs\"\n\n[dependencies]\ndep = { path = \"../dep\" }\n\n[workspace]\n",
        );
        w("app/build.rs", BUILD_RS);
        w(
            "app/src/main.rs",
            "fn main() {\n    print!(\"{}\", env!(\"BBP_BUILD_SHA\"));\n    let _ = dep::answer();\n}\n",
        );
        w(
            "dep/Cargo.toml",
            "[package]\nname = \"dep\"\nversion = \"0.0.0\"\nedition = \"2021\"\n\n[workspace]\n",
        );
        w("dep/src/lib.rs", "pub fn answer() -> u32 {\n    1\n}\n");
        // The lockfile is part of the tree: cargo would otherwise write it during
        // the first build, and that build would see an untracked file.
        run(
            &root.join("app"),
            &cargo(),
            &["generate-lockfile", "--offline", "-q"],
        );
        let r = Repo { root, target };
        r.git(&["init", "-q"]);
        r.commit("one");
        r
    }

    fn git(&self, args: &[&str]) -> String {
        let mut a = vec!["-c", "user.name=t", "-c", "user.email=t@t"];
        a.extend_from_slice(args);
        run(&self.root, "git", &a)
    }

    fn commit(&self, msg: &str) -> String {
        self.git(&["add", "-A"]);
        self.git(&["commit", "-q", "-m", msg]);
        self.head()
    }

    fn head(&self) -> String {
        self.git(&["rev-parse", "HEAD"])
    }

    fn edit(&self, file: &str, text: &str) {
        let p = self.root.join(file);
        let old = std::fs::read_to_string(&p).unwrap();
        std::fs::write(p, format!("{old}{text}")).unwrap();
    }

    /// Build the app with cargo and run it: the sha it was built with.
    fn built_sha(&self) -> String {
        let app = self.root.join("app");
        let target = self.target.display().to_string();
        let out = Command::new(cargo())
            .current_dir(&app)
            .args(["build", "--offline", "-q"])
            .env("CARGO_TARGET_DIR", &target)
            .output()
            .unwrap();
        assert!(
            out.status.success(),
            "{}",
            String::from_utf8_lossy(&out.stderr)
        );
        run(&app, &format!("{target}/debug/app"), &[])
    }
}

impl Drop for Repo {
    fn drop(&mut self) {
        let _ = std::fs::remove_dir_all(self.root.parent().unwrap());
    }
}

#[test]
fn a_clean_build_reports_the_head_it_was_built_from() {
    let r = Repo::new("clean");
    assert_eq!(r.built_sha(), r.head());
}

#[test]
fn an_unstaged_edit_makes_the_rebuilt_binary_dirty_and_a_revert_makes_it_clean_again() {
    let r = Repo::new("edit");
    let head = r.head();
    assert_eq!(r.built_sha(), head);
    r.edit("app/src/main.rs", "// edited\n");
    assert_eq!(r.built_sha(), format!("{head}-dirty"));
    r.git(&["checkout", "--", "app/src/main.rs"]);
    assert_eq!(
        r.built_sha(),
        head,
        "reverted: clean again, not stale-dirty"
    );
}

#[test]
fn an_edit_to_a_dependency_makes_the_binary_dirty() {
    let r = Repo::new("dep");
    let head = r.head();
    assert_eq!(r.built_sha(), head);
    r.edit("dep/src/lib.rs", "// edited\n");
    assert_eq!(r.built_sha(), format!("{head}-dirty"));
    r.git(&["checkout", "--", "dep/src/lib.rs"]);
    assert_eq!(r.built_sha(), head);
}

#[test]
fn a_new_untracked_file_makes_the_binary_dirty() {
    let r = Repo::new("untracked");
    let head = r.head();
    assert_eq!(r.built_sha(), head);
    std::fs::write(r.root.join("dep/src/extra.rs"), "// new\n").unwrap();
    assert_eq!(r.built_sha(), format!("{head}-dirty"));
}

#[test]
fn a_new_commit_on_the_same_branch_and_a_moved_ref_are_followed() {
    let r = Repo::new("refs");
    let first = r.head();
    assert_eq!(r.built_sha(), first);
    // A commit on the checked-out branch changes the branch ref, not HEAD itself.
    r.edit("dep/src/lib.rs", "// two\n");
    let second = r.commit("two");
    assert_ne!(first, second);
    assert_eq!(r.built_sha(), second, "a commit on the same branch");
    // Another branch, then back: HEAD's text changes and changes back.
    r.git(&["checkout", "-q", "-b", "other"]);
    r.edit("dep/src/lib.rs", "// three\n");
    let third = r.commit("three");
    assert_eq!(r.built_sha(), third, "a different branch");
    r.git(&["checkout", "-q", "-"]);
    assert_eq!(r.head(), second);
    assert_eq!(r.built_sha(), second, "back on the first branch");
    // A detached HEAD at an older commit.
    r.git(&["checkout", "-q", "--detach", &first]);
    assert_eq!(r.built_sha(), first, "detached at an older commit");
}

#[test]
fn a_tree_with_no_git_reports_unknown() {
    let r = Repo::new("nogit");
    std::fs::remove_dir_all(r.root.join(".git")).unwrap();
    assert_eq!(r.built_sha(), "unknown");
}
