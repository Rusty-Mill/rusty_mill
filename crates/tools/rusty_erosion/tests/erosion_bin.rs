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

#[cfg(unix)]
#[test]
fn an_unreadable_unsupported_file_never_blocks_enforcing() {
    let repo = Repo::new("langread");
    let base = repo.head();
    // Reading this path fails (it is a directory), but it is not Rust.
    std::os::unix::fs::symlink("src", repo.dir.join("tool.py")).expect("symlink");
    let out = repo.erosion(&enforcing(&base));
    assert_eq!(out.status.code(), Some(0), "{}", stdout(&out));
    assert!(
        stdout(&out).contains("\"unmeasured\":[\"tool.py\"]"),
        "{}",
        stdout(&out)
    );
}

#[cfg(unix)]
#[test]
fn an_unsupported_path_under_a_replaced_directory_never_blocks_enforcing() {
    let repo = Repo::new("langdir");
    std::fs::create_dir_all(repo.dir.join("scripts")).expect("mkdir");
    repo.write("scripts/tool.py", "print(1)\n");
    repo.git(&["add", "-A"]);
    repo.git(&[
        "-c",
        "user.name=t",
        "-c",
        "user.email=t@t",
        "commit",
        "--quiet",
        "-m",
        "py",
    ]);
    let base = repo.head();
    // `scripts/tool.py` now cannot even be looked up (NotADirectory).
    std::fs::remove_dir_all(repo.dir.join("scripts")).expect("rm");
    repo.write("scripts", "not a dir\n");
    let out = repo.erosion(&enforcing(&base));
    assert_eq!(out.status.code(), Some(0), "{}", stdout(&out));
    assert!(
        stdout(&out).contains("\"unmeasured\":[\"scripts/tool.py\"]"),
        "{}",
        stdout(&out)
    );
}

#[cfg(unix)]
#[test]
fn a_dangling_rust_symlink_is_not_a_deletion() {
    let repo = Repo::new("dangle");
    let base = repo.head();
    std::os::unix::fs::symlink("nowhere.rs", repo.dir.join("src/b.rs")).expect("symlink");
    let enforced = repo.erosion(&enforcing(&base));
    assert_eq!(enforced.status.code(), Some(2), "{}", stdout(&enforced));
    assert!(
        stdout(&enforced).contains("src/b.rs"),
        "{}",
        stdout(&enforced)
    );
    // Replacing existing Rust with one, on its own.
    std::fs::remove_file(repo.dir.join("src/b.rs")).expect("rm");
    std::fs::remove_file(repo.dir.join("src/a.rs")).expect("rm");
    std::os::unix::fs::symlink("nowhere.rs", repo.dir.join("src/a.rs")).expect("symlink");
    let replaced = repo.erosion(&enforcing(&base));
    assert_eq!(replaced.status.code(), Some(2), "{}", stdout(&replaced));
    assert!(
        stdout(&replaced).contains("src/a.rs"),
        "{}",
        stdout(&replaced)
    );
    assert!(
        !stdout(&replaced).contains("src/b.rs"),
        "{}",
        stdout(&replaced)
    );
}

#[cfg(unix)]
#[test]
fn rust_deleted_under_a_directory_replaced_by_a_file_is_a_deletion() {
    let repo = Repo::new("rustdir");
    std::fs::create_dir_all(repo.dir.join("src/m")).expect("mkdir");
    repo.write("src/m/b.rs", "pub fn b() {}\n");
    commit(&repo);
    let base = repo.head();
    std::fs::remove_dir_all(repo.dir.join("src/m")).expect("rm");
    repo.write("src/m", "not a dir\n");
    let out = repo.erosion(&enforcing(&base));
    assert_eq!(out.status.code(), Some(0), "{}", stdout(&out));
    assert!(
        stdout(&out).contains("\"functions\":-1"),
        "{}",
        stdout(&out)
    );
}

#[cfg(unix)]
#[test]
fn a_committed_rust_symlink_is_never_measured_as_source() {
    let repo = Repo::new("baselink");
    std::os::unix::fs::symlink("nowhere.rs", repo.dir.join("src/b.rs")).expect("symlink");
    commit(&repo);
    let base = repo.head();
    repo.write("src/a.rs", &format!("{ORIGINAL}\npub fn more() {{}}\n"));
    let report = repo.erosion(&["check", "--base", &base, "--report-only"]);
    assert_eq!(report.status.code(), Some(0));
    assert!(
        stdout(&report).contains("\"unmeasured\":[\"src/b.rs\"]"),
        "{}",
        stdout(&report)
    );
    let enforced = repo.erosion(&enforcing(&base));
    assert_eq!(enforced.status.code(), Some(2), "{}", stdout(&enforced));
    assert!(
        stdout(&enforced).contains("src/b.rs"),
        "{}",
        stdout(&enforced)
    );
}

#[cfg(unix)]
/// Runs `erosion` but gives up after 60 s, so a blocking read fails the test
/// instead of hanging it.
fn erosion_bounded(repo: &Repo, args: &[&str]) -> Output {
    let mut child = Command::new(env!("CARGO_BIN_EXE_erosion"))
        .args(args)
        .current_dir(&repo.dir)
        .stdout(std::process::Stdio::piped())
        .stderr(std::process::Stdio::piped())
        .spawn()
        .expect("run erosion");
    let start = std::time::Instant::now();
    while child.try_wait().expect("wait").is_none() {
        if start.elapsed() > std::time::Duration::from_secs(60) {
            let _ = child.kill();
            panic!("erosion blocked");
        }
        std::thread::sleep(std::time::Duration::from_millis(50));
    }
    child.wait_with_output().expect("output")
}

#[cfg(unix)]
#[test]
fn rust_replaced_by_a_fifo_is_unreadable_not_read() {
    let repo = Repo::new("fifo");
    let base = repo.head();
    std::fs::remove_file(repo.dir.join("src/a.rs")).expect("rm");
    let made = Command::new("mkfifo")
        .arg(repo.dir.join("src/a.rs"))
        .status()
        .expect("mkfifo");
    assert!(made.success());
    let report = erosion_bounded(&repo, &["check", "--base", &base, "--report-only"]);
    assert_eq!(report.status.code(), Some(0));
    assert!(
        stdout(&report).contains("\"unmeasured\":[\"src/a.rs\"]"),
        "{}",
        stdout(&report)
    );
    let enforced = erosion_bounded(&repo, &enforcing(&base));
    assert_eq!(enforced.status.code(), Some(2), "{}", stdout(&enforced));
    assert!(
        stdout(&enforced).contains("src/a.rs"),
        "{}",
        stdout(&enforced)
    );
}

#[cfg(unix)]
#[test]
fn rust_names_that_are_not_utf8_fail_enforcing_committed_or_changed() {
    use std::os::unix::ffi::OsStrExt;
    let bad = std::ffi::OsStr::from_bytes(b"src/\xff.rs");
    for committed in [true, false] {
        let repo = Repo::new(if committed { "namebase" } else { "namecand" });
        if committed {
            std::fs::write(repo.dir.join(bad), "pub fn b() {}\n").expect("write");
            commit(&repo);
        }
        let base = repo.head();
        if !committed {
            std::fs::write(repo.dir.join(bad), "pub fn b() {}\n").expect("write");
        }
        let report = repo.erosion(&["check", "--base", &base, "--report-only"]);
        assert_eq!(report.status.code(), Some(0));
        assert!(
            stdout(&report).contains("\"unmeasured\":[\"src/\u{fffd}.rs\"]"),
            "{}",
            stdout(&report)
        );
        let enforced = repo.erosion(&enforcing(&base));
        assert_eq!(enforced.status.code(), Some(2), "{}", stdout(&enforced));
    }
}

// Windows file names cannot hold a newline.
#[cfg(unix)]
#[test]
fn newlines_in_names_do_not_break_blob_framing() {
    let repo = Repo::new("newline");
    repo.write("x\ny.py", "print(1)\n");
    repo.write(
        "src/evil\n.rs",
        "pub fn evil(x: u8) -> u8 { if x > 1 { 1 } else { 0 } }\n",
    );
    commit(&repo);
    let base = repo.head();
    // Unchanged: both Rust files are measured from the base.
    let same = stdout(&repo.erosion(&enforcing(&base)));
    assert!(same.contains("\"functions\":0"), "{same}");
    // Deleting the newline-named file removes a function it measured.
    std::fs::remove_file(repo.dir.join("src/evil\n.rs")).expect("rm");
    let out = repo.erosion(&enforcing(&base));
    assert_eq!(out.status.code(), Some(0), "{}", stdout(&out));
    assert!(
        stdout(&out).contains("\"functions\":-1"),
        "{}",
        stdout(&out)
    );
    assert!(stdout(&out).contains("\"total_cc\":-2"), "{}", stdout(&out));
}

#[cfg(unix)]
#[test]
fn a_symlinked_ancestor_is_a_deletion_not_a_redirect() {
    let repo = Repo::new("ancestor");
    let base = repo.head();
    let elsewhere = repo.dir.join("elsewhere");
    std::fs::create_dir_all(&elsewhere).expect("mkdir");
    let rungs = "    if true { }\n".repeat(20);
    std::fs::write(
        elsewhere.join("a.rs"),
        format!("pub fn total() {{\n{rungs}}}\n"),
    )
    .expect("write");
    std::fs::write(repo.dir.join(".gitignore"), "elsewhere/\n").expect("ignore");
    std::fs::remove_dir_all(repo.dir.join("src")).expect("rm");
    std::os::unix::fs::symlink("elsewhere", repo.dir.join("src")).expect("symlink");
    let out = repo.erosion(&enforcing(&base));
    assert_eq!(out.status.code(), Some(0), "{}", stdout(&out));
    let line = stdout(&out);
    assert!(line.contains("\"functions\":-1"), "{line}");
    assert!(
        line.contains("\"concentration_bp\":{\"base\":0,\"candidate\":0"),
        "{line}"
    );
}

/// A one-commit repository at `dir` holding `files`.
fn nested_repo(dir: &std::path::Path, files: &[(&str, &str)]) {
    std::fs::create_dir_all(dir).expect("mkdir");
    for (name, text) in files {
        std::fs::write(dir.join(name), text).expect("write");
    }
    let git = |args: &[&str]| {
        let ok = Command::new("git")
            .args(["-c", "user.name=t", "-c", "user.email=t@t"])
            .args(args)
            .current_dir(dir)
            .status()
            .expect("git");
        assert!(ok.success(), "git {args:?}");
    };
    git(&["init", "--quiet"]);
    git(&["add", "-A"]);
    git(&["commit", "--quiet", "-m", "sub"]);
}

#[test]
fn rust_replaced_by_a_submodule_is_a_deletion_not_read() {
    let repo = Repo::new("subrepl");
    let base = repo.head();
    let rungs = "    if true { }\n".repeat(20);
    repo.git(&["rm", "--cached", "--quiet", "src/a.rs"]);
    std::fs::remove_dir_all(repo.dir.join("src")).expect("rm");
    nested_repo(
        &repo.dir.join("src"),
        &[("a.rs", &format!("pub fn total() {{\n{rungs}}}\n"))],
    );
    repo.git(&["add", "src"]);
    // The intended case: `src` is a gitlink and `src/a.rs` left the index.
    assert!(repo.git(&["ls-files", "-s", "src"]).starts_with("160000 "));
    let status = repo.git(&["diff", "--name-status", "--no-renames", &base]);
    assert!(status.contains("D\tsrc/a.rs"), "{status}");
    let out = repo.erosion(&enforcing(&base));
    assert_eq!(out.status.code(), Some(0), "{}", stdout(&out));
    let line = stdout(&out);
    assert!(line.contains("\"functions\":-1"), "{line}");
    assert!(
        line.contains("\"concentration_bp\":{\"base\":0,\"candidate\":0"),
        "{line}"
    );
}

#[test]
fn parent_tracked_rust_beside_a_nested_git_is_still_measured() {
    let repo = Repo::new("subkeep");
    let base = repo.head();
    // A nested repository in `src`, while the parent still tracks src/a.rs.
    nested_repo(&repo.dir.join("src"), &[("note.txt", "x\n")]);
    assert!(
        repo.git(&["ls-files", "-s", "src/a.rs"])
            .starts_with("100644 ")
    );
    repo.write("src/a.rs", &format!("{ORIGINAL}{PASTED}"));
    let out = repo.erosion(&enforcing(&base));
    assert_eq!(out.status.code(), Some(1), "{}", stdout(&out));
    assert!(stdout(&out).contains("\"functions\":1"), "{}", stdout(&out));
}

#[cfg(unix)]
#[test]
fn a_gitlink_with_a_non_utf8_name_never_hides_a_real_directory() {
    use std::os::unix::ffi::OsStrExt;
    let repo = Repo::new("subbytes");
    // A gitlink named by byte FF, and a tracked directory named U+FFFD,
    // which is what FF reads as when decoded lossily.
    nested_repo(
        &repo.dir.join(std::ffi::OsStr::from_bytes(b"\xff")),
        &[("x.txt", "x\n")],
    );
    std::fs::create_dir_all(repo.dir.join("\u{fffd}")).expect("mkdir");
    repo.write("\u{fffd}/a.rs", "pub fn z() {}\n");
    commit(&repo);
    let base = repo.head();
    // A copy of src/a.rs: only measured if this path is.
    repo.write("\u{fffd}/a.rs", &format!("pub fn z() {{}}\n{PASTED}"));
    let out = repo.erosion(&enforcing(&base));
    assert_eq!(out.status.code(), Some(1), "{}", stdout(&out));
    assert!(stdout(&out).contains("\"functions\":1"), "{}", stdout(&out));
}

#[cfg(unix)]
#[test]
fn an_updated_submodule_with_a_non_utf8_rust_name_is_skipped() {
    use std::os::unix::ffi::OsStrExt;
    let repo = Repo::new("subffrs");
    let sub = repo.dir.join(std::ffi::OsStr::from_bytes(b"\xff.rs"));
    nested_repo(&sub, &[("lib.rs", "pub fn v() {}\n")]);
    commit(&repo);
    assert!(repo.git(&["ls-files", "-s"]).contains("160000 "));
    let base = repo.head();
    nested_repo(&sub, &[("lib.rs", "pub fn v() { if true {} }\n")]);
    let out = repo.erosion(&enforcing(&base));
    assert_eq!(out.status.code(), Some(0), "{}", stdout(&out));
    assert!(
        stdout(&out).contains("\"unmeasured\":[]"),
        "{}",
        stdout(&out)
    );
}

#[test]
fn rust_removed_from_the_index_and_ignored_is_a_deletion() {
    let repo = Repo::new("ignored");
    let base = repo.head();
    repo.git(&["rm", "--cached", "--quiet", "src/a.rs"]);
    repo.write(".gitignore", "/src/a.rs\n");
    let status = repo.git(&["diff", "--name-status", "--no-renames", &base]);
    assert!(status.contains("D\tsrc/a.rs"), "{status}");
    let out = repo.erosion(&enforcing(&base));
    assert_eq!(out.status.code(), Some(0), "{}", stdout(&out));
    assert!(
        stdout(&out).contains("\"functions\":-1"),
        "{}",
        stdout(&out)
    );
    // Even an unreadable ignored copy is not the candidate's.
    std::fs::write(repo.dir.join("src/a.rs"), b"fn a() { \xff }\n").expect("write");
    let out = repo.erosion(&enforcing(&base));
    assert_eq!(out.status.code(), Some(0), "{}", stdout(&out));
}

#[test]
fn a_staged_deletion_of_another_language_is_still_reported() {
    let repo = Repo::new("pydel");
    repo.write("tool.py", "print(1)\n");
    commit(&repo);
    let base = repo.head();
    repo.git(&["rm", "--quiet", "tool.py"]);
    let out = repo.erosion(&enforcing(&base));
    assert_eq!(out.status.code(), Some(0), "{}", stdout(&out));
    assert!(
        stdout(&out).contains("\"unmeasured\":[\"tool.py\"]"),
        "{}",
        stdout(&out)
    );
}

#[test]
fn an_updated_submodule_with_a_rust_name_is_skipped() {
    let repo = Repo::new("subname");
    let sub = repo.dir.join("vendor.rs");
    nested_repo(&sub, &[("lib.rs", "pub fn v() {}\n")]);
    repo.git(&["add", "vendor.rs"]);
    commit(&repo);
    let base = repo.head();
    nested_repo(&sub, &[("lib.rs", "pub fn v() { if true {} }\n")]);
    let out = repo.erosion(&enforcing(&base));
    assert_eq!(out.status.code(), Some(0), "{}", stdout(&out));
    assert!(stdout(&out).contains("\"functions\":0"), "{}", stdout(&out));
}

fn commit(repo: &Repo) {
    repo.git(&["add", "-A"]);
    repo.git(&[
        "-c",
        "user.name=t",
        "-c",
        "user.email=t@t",
        "commit",
        "--quiet",
        "-m",
        "next",
    ]);
}

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
fn cleanup_passes_a_removed_duplicate_and_fails_an_unchanged_tree() {
    // D44: neither share may rise and one must fall, so an unchanged tree
    // fails even though 0/0 limits would pass it.
    let repo = Repo::new("cleanup");
    repo.write("src/b.rs", PASTED);
    commit(&repo);
    let base = repo.head();
    let cleanup = ["check", "--base", base.as_str(), "--cleanup"];

    let unchanged = repo.erosion(&cleanup);
    assert_eq!(unchanged.status.code(), Some(1), "{}", stdout(&unchanged));
    let line = stdout(&unchanged);
    assert!(line.contains("\"mode\":\"cleanup\""), "{line}");
    assert!(line.contains("\"within_limits\":false"), "{line}");

    repo.write("src/b.rs", "pub fn none() {}\n");
    let removed = repo.erosion(&cleanup);
    assert_eq!(removed.status.code(), Some(0), "{}", stdout(&removed));
    assert!(stdout(&removed).contains("\"within_limits\":true"));
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
    // The inbox parses these fields (`rusty_bbp_host` `erosion_detail`,
    // D91): changing them changes that parser too.
    let keys = [
        "{\"erosion\":1,\"base\":",
        ",\"mode\":",
        ",\"within_limits\":",
        ",\"concentration_bp\":{\"base\":",
        ",\"duplication_bp\":{\"base\":",
        ",\"total_cc\":",
        ",\"loc\":",
        ",\"functions\":",
        ",\"worst_affected\":[{\"key\":",
        ",\"base_cc\":",
        ",\"cc\":",
        ",\"unmeasured\":",
        ",\"parse_errors\":",
    ];
    let mut at = 0;
    for k in keys {
        let found = line[at..].find(k).map(|i| at + i);
        at = found.unwrap_or_else(|| panic!("{k} after byte {at}: {line}")) + k.len();
    }

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
    commit(&repo);
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
