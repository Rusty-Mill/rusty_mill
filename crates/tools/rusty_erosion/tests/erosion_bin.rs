//! The `erosion` binary against a real git repository: a pasted duplicate is
//! reported in report-only mode and blocks once limits are enforced.

use std::path::PathBuf;
use std::process::{Command, Output};

const ORIGINAL: &str = r#"
pub fn total(items: &[Item], rate: u32) -> u32 {
    let mut sum = 0;
    for item in items.iter() {
        let price = item.price * item.count;
        let tax = price * rate / 100;
        sum += price + tax;
    }
    sum
}
"#;

const PASTED: &str = r#"
pub fn grand(goods: &[Thing], pct: u32) -> u32 {
    let mut acc = 7;
    for g in goods.iter() {
        let cost = g.cost * g.qty;
        let duty = cost * pct / 250;
        acc += cost + duty;
    }
    acc
}
"#;

struct Repo {
    dir: PathBuf,
}

impl Repo {
    fn new(name: &str) -> Repo {
        let dir = std::env::temp_dir().join(format!("erosion-{name}-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);
        std::fs::create_dir_all(dir.join("src")).expect("create repo dir");
        let repo = Repo { dir };
        repo.git(&["init", "--quiet"]);
        repo.write("src/a.rs", ORIGINAL);
        repo.write("README.md", "# fixture\n");
        repo.git(&["add", "-A"]);
        repo.git(&[
            "-c",
            "user.name=t",
            "-c",
            "user.email=t@t",
            "commit",
            "--quiet",
            "-m",
            "base",
        ]);
        repo
    }

    fn write(&self, path: &str, text: &str) {
        std::fs::write(self.dir.join(path), text).expect("write fixture");
    }

    fn git(&self, args: &[&str]) -> String {
        let out = Command::new("git")
            .args(args)
            .current_dir(&self.dir)
            .output()
            .expect("run git");
        assert!(
            out.status.success(),
            "git {args:?}: {}",
            String::from_utf8_lossy(&out.stderr)
        );
        String::from_utf8_lossy(&out.stdout).trim().to_owned()
    }

    fn erosion(&self, args: &[&str]) -> Output {
        Command::new(env!("CARGO_BIN_EXE_erosion"))
            .args(args)
            .current_dir(&self.dir)
            .output()
            .expect("run erosion")
    }

    fn head(&self) -> String {
        self.git(&["rev-parse", "HEAD"])
    }
}

impl Drop for Repo {
    fn drop(&mut self) {
        let _ = std::fs::remove_dir_all(&self.dir);
    }
}

fn stdout(out: &Output) -> String {
    String::from_utf8_lossy(&out.stdout).into_owned()
}

fn enforcing(base: &str) -> [&str; 7] {
    [
        "check",
        "--base",
        base,
        "--max-concentration-bp",
        "100",
        "--max-duplication-bp",
        "100",
    ]
}

#[test]
fn unchanged_tree_has_zero_deltas() {
    let repo = Repo::new("same");
    let base = repo.head();
    let out = repo.erosion(&enforcing(&base));
    assert_eq!(out.status.code(), Some(0), "{}", stdout(&out));
    let line = stdout(&out);
    assert!(
        line.contains("\"duplication_bp\":{\"base\":0,\"candidate\":0,\"delta\":0}"),
        "{line}"
    );
    assert!(line.contains("\"within_limits\":true"), "{line}");
}

#[test]
fn a_pasted_duplicate_is_reported_then_blocked_once_enforcing() {
    let repo = Repo::new("paste");
    let base = repo.head();
    repo.write("src/b.rs", PASTED);
    repo.write("tool.py", "print(1)\n");

    let report = repo.erosion(&["check", "--base", &base, "--report-only"]);
    assert_eq!(report.status.code(), Some(0));
    let line = stdout(&report);
    assert!(
        line.contains("\"duplication_bp\":{\"base\":0,\"candidate\":10000,\"delta\":10000}"),
        "{line}"
    );
    assert!(line.contains("\"mode\":\"report-only\""), "{line}");
    assert!(line.contains("\"within_limits\":null"), "{line}");
    assert!(line.contains("\"functions\":1"), "{line}");
    assert!(line.contains("\"unmeasured\":[\"tool.py\"]"), "{line}");

    let enforced = repo.erosion(&enforcing(&base));
    assert_eq!(enforced.status.code(), Some(1), "{}", stdout(&enforced));
}

#[test]
fn worst_affected_and_deleted_files() {
    let repo = Repo::new("cc");
    let base = repo.head();
    let rungs: String = (0..12)
        .map(|i| format!("    if x == {i} {{ return {i}; }}\n"))
        .collect();
    repo.write(
        "src/a.rs",
        &format!("pub fn total(x: u8) -> u8 {{\n{rungs}    0\n}}\n"),
    );
    let line = stdout(&repo.erosion(&["check", "--base", &base, "--report-only"]));
    assert!(
        line.contains("\"worst_affected\":[{\"key\":\"src/a.rs::total\",\"base_cc\":2,\"cc\":13}]"),
        "{line}"
    );

    std::fs::remove_file(repo.dir.join("src/a.rs")).expect("delete");
    let line = stdout(&repo.erosion(&["check", "--base", &base, "--report-only"]));
    assert!(line.contains("\"functions\":-1"), "{line}");
}

#[test]
fn measurement_errors_never_block_report_only() {
    let repo = Repo::new("err");
    let missing = "0123456789abcdef0123456789abcdef01234567";
    let ro = repo.erosion(&["check", "--base", missing, "--report-only"]);
    assert_eq!(ro.status.code(), Some(0));
    assert!(stdout(&ro).contains("\"error\""));
    assert_eq!(repo.erosion(&enforcing(missing)).status.code(), Some(2));
    assert_eq!(repo.erosion(&["check"]).status.code(), Some(2));
}

#[test]
fn a_nested_working_directory_measures_the_same_whole_tree() {
    let repo = Repo::new("nested");
    let base = repo.head();
    repo.write("src/b.rs", PASTED);
    let root = stdout(&repo.erosion(&["check", "--base", &base, "--report-only"]));
    let nested = Command::new(env!("CARGO_BIN_EXE_erosion"))
        .args(["check", "--base", &base, "--report-only"])
        .current_dir(repo.dir.join("src"))
        .output()
        .expect("run erosion");
    assert_eq!(stdout(&nested), root);
    assert!(root.contains("\"candidate\":10000"), "{root}");
    let enforced = Command::new(env!("CARGO_BIN_EXE_erosion"))
        .args(enforcing(&base))
        .current_dir(repo.dir.join("src"))
        .output()
        .expect("run erosion");
    assert_eq!(enforced.status.code(), Some(1), "{}", stdout(&enforced));
}

#[test]
fn changed_rust_that_is_not_utf8_is_reported_unmeasured() {
    let repo = Repo::new("utf8");
    let base = repo.head();
    std::fs::write(repo.dir.join("src/bad.rs"), b"fn a() { \xff }\n").expect("write");
    let line = stdout(&repo.erosion(&["check", "--base", &base, "--report-only"]));
    assert!(line.contains("\"unmeasured\":[\"src/bad.rs\"]"), "{line}");
    let enforced = repo.erosion(&enforcing(&base));
    assert_eq!(enforced.status.code(), Some(2), "{}", stdout(&enforced));
    assert!(
        stdout(&enforced).contains("not UTF-8"),
        "{}",
        stdout(&enforced)
    );
}

#[test]
fn modified_rust_that_is_not_utf8_fails_enforcing_only() {
    let repo = Repo::new("utf8mod");
    let base = repo.head();
    std::fs::write(repo.dir.join("src/a.rs"), b"pub fn total() { \xff }\n").expect("write");
    let line = stdout(&repo.erosion(&["check", "--base", &base, "--report-only"]));
    assert!(line.contains("\"unmeasured\":[\"src/a.rs\"]"), "{line}");
    assert_eq!(repo.erosion(&enforcing(&base)).status.code(), Some(2));
}

/// A repo whose base commit holds `src/bad.rs` that is not UTF-8.
fn unreadable_base(name: &str) -> (Repo, String) {
    let repo = Repo::new(name);
    std::fs::write(repo.dir.join("src/bad.rs"), b"fn b() { \xff }\n").expect("write");
    repo.git(&["add", "-A"]);
    repo.git(&[
        "-c",
        "user.name=t",
        "-c",
        "user.email=t@t",
        "commit",
        "--quiet",
        "-m",
        "bad",
    ]);
    let base = repo.head();
    (repo, base)
}

#[test]
fn an_unchanged_unreadable_base_fails_enforcing_only() {
    let (repo, base) = unreadable_base("utf8base");
    repo.write("src/a.rs", &format!("{ORIGINAL}\npub fn more() {{}}\n"));
    let report = repo.erosion(&["check", "--base", &base, "--report-only"]);
    assert_eq!(report.status.code(), Some(0));
    assert!(
        stdout(&report).contains("\"unmeasured\":[\"src/bad.rs\"]"),
        "{}",
        stdout(&report)
    );
    let enforced = repo.erosion(&enforcing(&base));
    assert_eq!(enforced.status.code(), Some(2), "{}", stdout(&enforced));
    assert!(
        stdout(&enforced).contains("src/bad.rs"),
        "{}",
        stdout(&enforced)
    );
}

#[test]
fn an_unreadable_base_replaced_with_readable_rust_still_fails_enforcing() {
    let (repo, base) = unreadable_base("utf8fixed");
    repo.write("src/bad.rs", "pub fn b() {}\n");
    let report = repo.erosion(&["check", "--base", &base, "--report-only"]);
    assert_eq!(report.status.code(), Some(0));
    assert!(
        stdout(&report).contains("\"unmeasured\":[\"src/bad.rs\"]"),
        "{}",
        stdout(&report)
    );
    let enforced = repo.erosion(&enforcing(&base));
    assert_eq!(enforced.status.code(), Some(2), "{}", stdout(&enforced));
}

#[test]
fn unsupported_language_changes_never_block_enforcing() {
    let repo = Repo::new("lang");
    let base = repo.head();
    repo.write("tool.py", "print(1)\n");
    let out = repo.erosion(&enforcing(&base));
    assert_eq!(out.status.code(), Some(0), "{}", stdout(&out));
    assert!(
        stdout(&out).contains("\"unmeasured\":[\"tool.py\"]"),
        "{}",
        stdout(&out)
    );
}
