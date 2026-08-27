//! The `Repository` orchestrator: init/open, staging (`add`), commits with
//! real trees and parent chains, `status` (a real staged-vs-HEAD and
//! worktree-vs-index comparison), and `log` (walking real commit parent
//! links, not a side log file).

use std::collections::BTreeMap;
use std::fs;
use std::path::{Path, PathBuf};
use std::time::{SystemTime, UNIX_EPOCH};

use crate::gitignore::Gitignore;
use crate::index::{Index, IndexEntry, MODE_REGULAR};
use crate::objects::{
    decode_commit, decode_tree, hash, read_object, write_object, Commit, ObjectError, ObjectKind,
};
use crate::sha1::{hex, SHA1_DIGEST_LEN};
use crate::tree_builder::write_tree;

/// A staged-vs-committed or worktree-vs-staged change, as reported by
/// [`Repository::status`].
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct StatusEntry {
    /// Path relative to the repository root.
    pub path: String,
    /// A short, human-readable category: `"new file"`, `"modified"`,
    /// `"deleted"` (all relative to `HEAD`, i.e. staged), or `"modified
    /// (not staged)"` / `"untracked"` (relative to the index).
    pub status: String,
}

/// One entry from [`Repository::log`].
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct CommitLog {
    /// Hex object id of the commit.
    pub hash: String,
    /// The commit message (trailing newline stripped).
    pub message: String,
    /// The raw author line (`"Name <email> <unix-seconds> <+zzzz>"`).
    pub author: String,
}

/// A real (if simplified) git repository: real objects, a real index, real
/// commit parent chains. See `index.rs`'s module doc for the one
/// deliberate simplification (zeroed stat-cache fields) and `zlib.rs`'s for
/// the read/write asymmetry around loose objects this crate didn't write
/// itself.
pub struct Repository {
    /// The `.git` directory.
    pub git_dir: PathBuf,
    /// The repository root (working tree).
    pub work_tree: PathBuf,
}

impl Repository {
    /// Initializes a new git repository at `path`.
    pub fn init(path: &Path) -> Result<Self, String> {
        let git_dir = path.join(".git");
        if git_dir.exists() {
            return Err("Git repository already exists".to_string());
        }

        fs::create_dir_all(git_dir.join("objects")).map_err(|e| e.to_string())?;
        fs::create_dir_all(git_dir.join("refs").join("heads")).map_err(|e| e.to_string())?;
        fs::write(git_dir.join("HEAD"), "ref: refs/heads/main\n").map_err(|e| e.to_string())?;

        Ok(Repository {
            git_dir,
            work_tree: path.to_path_buf(),
        })
    }

    /// Opens an existing git repository, searching up from `path`.
    pub fn open(path: &Path) -> Result<Self, String> {
        let mut curr = path.to_path_buf();
        loop {
            let git_dir = curr.join(".git");
            if git_dir.is_dir() {
                return Ok(Repository {
                    git_dir,
                    work_tree: curr,
                });
            }
            if !curr.pop() {
                break;
            }
        }
        Err("Not a git repository (or any of the parent directories)".to_string())
    }

    /// The current branch name (`HEAD`'s target, or `HEAD`'s literal
    /// content if detached).
    pub fn current_branch(&self) -> String {
        let head = fs::read_to_string(self.git_dir.join("HEAD")).unwrap_or_default();
        if let Some(r) = head.strip_prefix("ref: refs/heads/") {
            r.trim().to_string()
        } else {
            head.trim().to_string()
        }
    }

    fn branch_ref_path(&self) -> PathBuf {
        self.git_dir
            .join("refs")
            .join("heads")
            .join(self.current_branch())
    }

    /// The hex object id `HEAD` currently points at, or `None` for a
    /// branch with no commits yet.
    pub fn head_commit(&self) -> Option<String> {
        let text = fs::read_to_string(self.branch_ref_path()).ok()?;
        let trimmed = text.trim();
        if trimmed.is_empty() {
            None
        } else {
            Some(trimmed.to_string())
        }
    }

    /// Reads and loads the current index (staging area), or an empty one.
    pub fn index(&self) -> Result<Index, String> {
        Index::read(&self.git_dir).map_err(|e| e.to_string())
    }

    /// Determines whether `path`'s file mode should be recorded as
    /// executable. Always `false` on platforms without a Unix execute bit
    /// (e.g. Windows) — a known simplification, not a claim of parity.
    fn file_mode(metadata: &fs::Metadata) -> u32 {
        #[cfg(unix)]
        {
            use std::os::unix::fs::PermissionsExt;
            if metadata.permissions().mode() & 0o111 != 0 {
                return crate::index::MODE_EXECUTABLE;
            }
        }
        let _ = metadata;
        MODE_REGULAR
    }

    /// Stages `paths` (each relative to the repository root, using either
    /// path separator): hashes and writes each as a blob object, then
    /// records it in the index. A directory path stages every regular
    /// file under it, recursively.
    pub fn add(&self, paths: &[String]) -> Result<(), String> {
        let mut index = self.index()?;
        for raw_path in paths {
            let abs = self.work_tree.join(raw_path);
            if abs.is_dir() {
                self.add_dir_recursive(&abs, &mut index)?;
            } else {
                self.add_file(&abs, &mut index)?;
            }
        }
        index.write(&self.git_dir).map_err(|e| e.to_string())
    }

    fn add_dir_recursive(&self, dir: &Path, index: &mut Index) -> Result<(), String> {
        let gitignore = Gitignore::load(&self.work_tree);
        self.add_dir_recursive_inner(dir, index, &gitignore)
    }

    fn add_dir_recursive_inner(
        &self,
        dir: &Path,
        index: &mut Index,
        gitignore: &Gitignore,
    ) -> Result<(), String> {
        for entry in fs::read_dir(dir).map_err(|e| e.to_string())? {
            let entry = entry.map_err(|e| e.to_string())?;
            let path = entry.path();
            let name = entry.file_name();
            let is_dir = path.is_dir();
            if name == ".git" || gitignore.matches(&name.to_string_lossy(), is_dir) {
                continue;
            }
            if is_dir {
                self.add_dir_recursive_inner(&path, index, gitignore)?;
            } else {
                self.add_file(&path, index)?;
            }
        }
        Ok(())
    }

    fn add_file(&self, abs_path: &Path, index: &mut Index) -> Result<(), String> {
        let content = fs::read(abs_path).map_err(|e| e.to_string())?;
        let metadata = fs::metadata(abs_path).map_err(|e| e.to_string())?;
        let oid =
            write_object(&self.git_dir, ObjectKind::Blob, &content).map_err(|e| e.to_string())?;
        let rel = abs_path
            .strip_prefix(&self.work_tree)
            .map_err(|e| e.to_string())?
            .to_string_lossy()
            .replace('\\', "/");

        let mut raw = [0u8; SHA1_DIGEST_LEN];
        for (i, byte) in raw.iter_mut().enumerate() {
            *byte = u8::from_str_radix(&oid[i * 2..i * 2 + 2], 16).unwrap_or(0);
        }
        index.upsert(IndexEntry {
            mode: Self::file_mode(&metadata),
            size: content.len() as u32,
            hash: raw,
            path: rel,
        });
        Ok(())
    }

    /// Commits the current index: builds a real (possibly nested) tree,
    /// writes a real commit object with `HEAD`'s current commit as parent
    /// (if any), and advances the current branch ref to it.
    pub fn create_commit(&self, message: &str, author: &str) -> Result<String, String> {
        let index = self.index()?;
        let tree_oid = write_tree(&self.git_dir, index.entries()).map_err(|e| e.to_string())?;

        let now = SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .map(|d| d.as_secs())
            .unwrap_or(0);
        // Always UTC (+0000) -- this crate doesn't compute the local
        // timezone offset; a known simplification.
        let identity = format!("{author} {now} +0000");

        let parents = self.head_commit().into_iter().collect();
        let commit = Commit {
            tree: tree_oid,
            parents,
            author: identity.clone(),
            committer: identity,
            message: message.to_string(),
        };
        let content = crate::objects::encode_commit(&commit);
        let commit_oid = hex(&hash(ObjectKind::Commit, &content));
        write_object(&self.git_dir, ObjectKind::Commit, &content).map_err(|e| e.to_string())?;

        let ref_path = self.branch_ref_path();
        if let Some(parent) = ref_path.parent() {
            fs::create_dir_all(parent).map_err(|e| e.to_string())?;
        }
        fs::write(&ref_path, format!("{commit_oid}\n")).map_err(|e| e.to_string())?;

        Ok(commit_oid)
    }

    /// Recursively flattens a tree object into `path -> raw blob hash`,
    /// joining subtree paths with `/`.
    fn flatten_tree(
        &self,
        tree_oid: &str,
        prefix: &str,
        out: &mut BTreeMap<String, [u8; SHA1_DIGEST_LEN]>,
    ) -> Result<(), ObjectError> {
        let (_, content) = read_object(&self.git_dir, tree_oid)?;
        for entry in decode_tree(&content)? {
            let path = if prefix.is_empty() {
                entry.name.clone()
            } else {
                format!("{prefix}/{}", entry.name)
            };
            if entry.mode == "40000" {
                self.flatten_tree(&hex(&entry.hash), &path, out)?;
            } else {
                out.insert(path, entry.hash);
            }
        }
        Ok(())
    }

    fn head_tree_map(&self) -> BTreeMap<String, [u8; SHA1_DIGEST_LEN]> {
        let mut out = BTreeMap::new();
        if let Some(commit_oid) = self.head_commit() {
            if let Ok((_, content)) = read_object(&self.git_dir, &commit_oid) {
                if let Ok(commit) = decode_commit(&content) {
                    let _ = self.flatten_tree(&commit.tree, "", &mut out);
                }
            }
        }
        out
    }

    /// Reports what's staged relative to `HEAD` (`"new file"` / `"modified"`
    /// / `"deleted"`), plus what's changed in the working tree relative to
    /// the index (`"modified (not staged)"` / `"untracked"`).
    pub fn status(&self) -> Result<Vec<StatusEntry>, String> {
        let index = self.index()?;
        let head = self.head_tree_map();
        let mut entries = Vec::new();

        let index_map: BTreeMap<&str, &IndexEntry> = index
            .entries()
            .iter()
            .map(|e| (e.path.as_str(), e))
            .collect();

        for (path, head_hash) in &head {
            match index_map.get(path.as_str()) {
                None => entries.push(StatusEntry {
                    path: path.clone(),
                    status: "deleted".to_string(),
                }),
                Some(idx_entry) if idx_entry.hash != *head_hash => {
                    entries.push(StatusEntry {
                        path: path.clone(),
                        status: "modified".to_string(),
                    });
                }
                _ => {}
            }
        }
        for path in index_map.keys() {
            if !head.contains_key(*path) {
                entries.push(StatusEntry {
                    path: path.to_string(),
                    status: "new file".to_string(),
                });
            }
        }

        let mut worktree_files = Vec::new();
        self.collect_worktree_files(&self.work_tree, &mut worktree_files)?;
        for rel in &worktree_files {
            let content = fs::read(self.work_tree.join(rel)).map_err(|e| e.to_string())?;
            let blob_hash = hash(ObjectKind::Blob, &content);
            match index.get(rel) {
                None => entries.push(StatusEntry {
                    path: rel.clone(),
                    status: "untracked".to_string(),
                }),
                Some(idx_entry) if idx_entry.hash != blob_hash => {
                    entries.push(StatusEntry {
                        path: rel.clone(),
                        status: "modified (not staged)".to_string(),
                    });
                }
                _ => {}
            }
        }

        Ok(entries)
    }

    fn collect_worktree_files(&self, dir: &Path, out: &mut Vec<String>) -> Result<(), String> {
        let gitignore = Gitignore::load(&self.work_tree);
        self.collect_worktree_files_inner(dir, out, &gitignore)
    }

    fn collect_worktree_files_inner(
        &self,
        dir: &Path,
        out: &mut Vec<String>,
        gitignore: &Gitignore,
    ) -> Result<(), String> {
        for entry in fs::read_dir(dir).map_err(|e| e.to_string())? {
            let entry = entry.map_err(|e| e.to_string())?;
            let path = entry.path();
            let name = entry.file_name();
            let is_dir = path.is_dir();
            if name == ".git" || gitignore.matches(&name.to_string_lossy(), is_dir) {
                continue;
            }
            if is_dir {
                self.collect_worktree_files_inner(&path, out, gitignore)?;
            } else {
                let rel = path
                    .strip_prefix(&self.work_tree)
                    .map_err(|e| e.to_string())?
                    .to_string_lossy()
                    .replace('\\', "/");
                out.push(rel);
            }
        }
        Ok(())
    }

    /// Walks `HEAD`'s commit parent chain (first-parent only), newest first.
    pub fn log(&self) -> Result<Vec<CommitLog>, String> {
        let mut logs = Vec::new();
        let mut current = self.head_commit();
        while let Some(oid) = current {
            let (_, content) = read_object(&self.git_dir, &oid).map_err(|e| e.to_string())?;
            let commit = decode_commit(&content).map_err(|e| e.to_string())?;
            current = commit.parents.first().cloned();
            logs.push(CommitLog {
                hash: oid,
                message: commit.message.trim_end().to_string(),
                author: commit.author.clone(),
            });
        }
        Ok(logs)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn temp_repo(name: &str) -> Repository {
        let dir =
            std::env::temp_dir().join(format!("rusty_git_repo_test_{name}_{}", std::process::id()));
        let _ = fs::remove_dir_all(&dir);
        fs::create_dir_all(&dir).unwrap();
        Repository::init(&dir).unwrap()
    }

    #[test]
    fn init_creates_expected_layout() {
        let repo = temp_repo("init");
        assert!(repo.git_dir.join("objects").is_dir());
        assert!(repo.git_dir.join("refs").join("heads").is_dir());
        assert_eq!(repo.current_branch(), "main");
        assert_eq!(repo.head_commit(), None);
        let _ = fs::remove_dir_all(&repo.work_tree);
    }

    #[test]
    fn add_then_commit_produces_a_real_readable_commit_and_tree() {
        let repo = temp_repo("commit");
        fs::write(repo.work_tree.join("hello.txt"), b"hello world").unwrap();

        repo.add(&["hello.txt".to_string()]).unwrap();
        let index = repo.index().unwrap();
        assert_eq!(index.entries().len(), 1);

        let commit_oid = repo.create_commit("Initial commit", "A <a@b.c>").unwrap();
        assert_eq!(repo.head_commit(), Some(commit_oid.clone()));

        let logs = repo.log().unwrap();
        assert_eq!(logs.len(), 1);
        assert_eq!(logs[0].hash, commit_oid);
        assert_eq!(logs[0].message, "Initial commit");

        let (_, commit_content) = read_object(&repo.git_dir, &commit_oid).unwrap();
        let commit = decode_commit(&commit_content).unwrap();
        assert!(commit.parents.is_empty());

        let (_, tree_content) = read_object(&repo.git_dir, &commit.tree).unwrap();
        let tree_entries = decode_tree(&tree_content).unwrap();
        assert_eq!(tree_entries.len(), 1);
        assert_eq!(tree_entries[0].name, "hello.txt");

        let _ = fs::remove_dir_all(&repo.work_tree);
    }

    #[test]
    fn second_commit_chains_to_the_first_as_parent() {
        let repo = temp_repo("chain");
        fs::write(repo.work_tree.join("a.txt"), b"a").unwrap();
        repo.add(&["a.txt".to_string()]).unwrap();
        let first = repo.create_commit("first", "A <a@b.c>").unwrap();

        fs::write(repo.work_tree.join("b.txt"), b"b").unwrap();
        repo.add(&["b.txt".to_string()]).unwrap();
        let second = repo.create_commit("second", "A <a@b.c>").unwrap();

        let (_, content) = read_object(&repo.git_dir, &second).unwrap();
        let commit = decode_commit(&content).unwrap();
        assert_eq!(commit.parents, vec![first.clone()]);

        let logs = repo.log().unwrap();
        assert_eq!(logs.len(), 2);
        assert_eq!(logs[0].hash, second);
        assert_eq!(logs[1].hash, first);

        let _ = fs::remove_dir_all(&repo.work_tree);
    }

    #[test]
    fn status_reports_untracked_then_staged_new_file_then_clean() {
        let repo = temp_repo("status");
        fs::write(repo.work_tree.join("a.txt"), b"a").unwrap();

        let entries = repo.status().unwrap();
        assert_eq!(entries.len(), 1);
        assert_eq!(entries[0].status, "untracked");

        repo.add(&["a.txt".to_string()]).unwrap();
        let entries = repo.status().unwrap();
        assert_eq!(entries.len(), 1);
        assert_eq!(entries[0].status, "new file");

        repo.create_commit("add a.txt", "A <a@b.c>").unwrap();
        let entries = repo.status().unwrap();
        assert!(entries.is_empty(), "expected clean status, got {entries:?}");

        let _ = fs::remove_dir_all(&repo.work_tree);
    }

    #[test]
    fn status_reports_modified_not_staged_after_editing_a_committed_file() {
        let repo = temp_repo("status_modified");
        fs::write(repo.work_tree.join("a.txt"), b"original").unwrap();
        repo.add(&["a.txt".to_string()]).unwrap();
        repo.create_commit("initial", "A <a@b.c>").unwrap();

        fs::write(repo.work_tree.join("a.txt"), b"changed").unwrap();
        let entries = repo.status().unwrap();
        assert_eq!(entries.len(), 1);
        assert_eq!(entries[0].status, "modified (not staged)");

        let _ = fs::remove_dir_all(&repo.work_tree);
    }

    #[test]
    fn add_on_a_directory_stages_every_file_recursively() {
        let repo = temp_repo("add_dir");
        fs::create_dir_all(repo.work_tree.join("src")).unwrap();
        fs::write(repo.work_tree.join("src").join("main.rs"), b"fn main() {}").unwrap();
        fs::write(repo.work_tree.join("README.md"), b"# hi").unwrap();

        repo.add(&[".".to_string()]).unwrap();
        let index = repo.index().unwrap();
        let paths: Vec<&str> = index.entries().iter().map(|e| e.path.as_str()).collect();
        assert_eq!(paths, vec!["README.md", "src/main.rs"]);

        let _ = fs::remove_dir_all(&repo.work_tree);
    }
}
