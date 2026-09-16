//! `rusty_git`: a real (if intentionally scoped-down) pure-Rust Git
//! implementation — real SHA-1 content addressing, real zlib-wrapped loose
//! objects, a real binary-compatible index (staging area), and real nested
//! tree/commit objects a genuine `git` installation can read.
//!
//! Known, explicitly-documented gaps (see each module's own doc comment for
//! detail): this crate can write objects real git reads, but can only read
//! back objects *it* wrote (real git's Huffman-compressed objects use a
//! DEFLATE block type this crate's decompressor doesn't implement yet — see
//! [`zlib`]); the index's stat-cache fields are always zeroed (see
//! [`index`]); there is no packfile support, no merge, and no remotes.

pub mod gitignore;
pub mod index;
pub mod objects;
pub mod repository;
pub mod sha1;
pub mod tree_builder;
pub mod zlib;

pub use index::{Index, IndexEntry};
pub use objects::{
    decode_commit, decode_tree, encode_commit, encode_tree, Commit, ObjectError, ObjectKind,
    TreeEntry,
};
pub use repository::{CommitLog, Repository, StatusEntry};

#[cfg(test)]
mod real_git_interop_tests {
    //! These tests shell out to the system `git` binary to verify this
    //! crate's output is genuinely readable by real git, not merely
    //! internally self-consistent. Skipped (not failed) if `git` isn't on
    //! `PATH`.

    use super::*;
    use std::process::Command;

    fn git_available() -> bool {
        Command::new("git")
            .arg("--version")
            .output()
            .map(|o| o.status.success())
            .unwrap_or(false)
    }

    fn temp_dir(name: &str) -> std::path::PathBuf {
        let dir = std::env::temp_dir().join(format!(
            "rusty_git_real_interop_{name}_{}",
            std::process::id()
        ));
        let _ = std::fs::remove_dir_all(&dir);
        std::fs::create_dir_all(&dir).unwrap();
        dir
    }

    #[test]
    fn real_git_cat_file_reads_a_blob_this_crate_wrote() {
        if !git_available() {
            eprintln!("skipping: git not on PATH");
            return;
        }
        let dir = temp_dir("blob");
        let repo = Repository::init(&dir).unwrap();
        let oid = objects::write_object(&repo.git_dir, ObjectKind::Blob, b"hello world").unwrap();

        let output = Command::new("git")
            .arg("--git-dir")
            .arg(&repo.git_dir)
            .arg("cat-file")
            .arg("-p")
            .arg(&oid)
            .output()
            .expect("git cat-file should run");
        assert!(output.status.success(), "git cat-file failed: {:?}", output);
        assert_eq!(output.stdout, b"hello world");

        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn real_git_log_reads_a_commit_this_crate_wrote() {
        if !git_available() {
            eprintln!("skipping: git not on PATH");
            return;
        }
        let dir = temp_dir("commit");
        let repo = Repository::init(&dir).unwrap();
        std::fs::write(dir.join("hello.txt"), b"hello world").unwrap();
        repo.add(&["hello.txt".to_string()]).unwrap();
        let commit_oid = repo
            .create_commit("Initial commit", "Rusty Git <rusty@git.test>")
            .unwrap();

        let output = Command::new("git")
            .arg("--git-dir")
            .arg(&repo.git_dir)
            .arg("--work-tree")
            .arg(&repo.work_tree)
            .arg("log")
            .arg("--oneline")
            .output()
            .expect("git log should run");
        assert!(output.status.success(), "git log failed: {:?}", output);
        let stdout = String::from_utf8_lossy(&output.stdout);
        assert!(
            stdout.contains(&commit_oid[..7]),
            "expected {commit_oid} in: {stdout}"
        );
        assert!(stdout.contains("Initial commit"));

        let ls_tree = Command::new("git")
            .arg("--git-dir")
            .arg(&repo.git_dir)
            .arg("ls-tree")
            .arg(&commit_oid)
            .output()
            .expect("git ls-tree should run");
        assert!(ls_tree.status.success());
        assert!(String::from_utf8_lossy(&ls_tree.stdout).contains("hello.txt"));

        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn real_git_ls_files_reads_this_crates_index() {
        if !git_available() {
            eprintln!("skipping: git not on PATH");
            return;
        }
        let dir = temp_dir("index");
        let repo = Repository::init(&dir).unwrap();
        std::fs::write(dir.join("a.txt"), b"a").unwrap();
        std::fs::create_dir_all(dir.join("sub")).unwrap();
        std::fs::write(dir.join("sub").join("b.txt"), b"b").unwrap();
        repo.add(&[".".to_string()]).unwrap();

        let output = Command::new("git")
            .arg("--git-dir")
            .arg(&repo.git_dir)
            .arg("--work-tree")
            .arg(&repo.work_tree)
            .arg("ls-files")
            .arg("--stage")
            .output()
            .expect("git ls-files should run");
        assert!(output.status.success(), "git ls-files failed: {:?}", output);
        let stdout = String::from_utf8_lossy(&output.stdout);
        assert!(stdout.contains("a.txt"), "expected a.txt in: {stdout}");
        assert!(
            stdout.contains("sub/b.txt"),
            "expected sub/b.txt in: {stdout}"
        );

        let _ = std::fs::remove_dir_all(&dir);
    }
}
