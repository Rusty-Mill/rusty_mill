//! `rusty_git`: Pure Rust Git repository object model, index, refs, and working directory manager.

use std::fs;
use std::io::{Write};
use std::path::{Path, PathBuf};

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum ObjectKind {
    Blob,
    Tree,
    Commit,
    Tag,
}

#[derive(Debug, Clone)]
pub struct StatusEntry {
    pub path: String,
    pub status: &'static str,
}

#[derive(Debug, Clone)]
pub struct CommitLog {
    pub hash: String,
    pub message: String,
}

pub struct Repository {
    pub git_dir: PathBuf,
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

        let head_path = git_dir.join("HEAD");
        fs::write(head_path, "ref: refs/heads/main\n").map_err(|e| e.to_string())?;

        Ok(Repository {
            git_dir,
            work_tree: path.to_path_buf(),
        })
    }

    /// Opens an existing git repository searching up from `path`.
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

    /// Gets current branch name.
    pub fn current_branch(&self) -> String {
        let head = fs::read_to_string(self.git_dir.join("HEAD")).unwrap_or_default();
        if let Some(r) = head.strip_prefix("ref: refs/heads/") {
            r.trim().to_string()
        } else {
            head.trim().to_string()
        }
    }

    /// Writes a simple blob object returning pseudo SHA1 string.
    pub fn create_blob(&self, content: &[u8]) -> Result<String, String> {
        let mut hash_bytes = [0u8; 20];
        let mut seed = content.len() as u32;
        for (i, b) in content.iter().enumerate() {
            seed = seed.wrapping_add((*b as u32).wrapping_mul(31)).wrapping_add(i as u32);
            hash_bytes[i % 20] ^= (seed & 0xFF) as u8;
        }

        let hex_hash = hash_bytes.iter().map(|b| format!("{:02x}", b)).collect::<String>();
        let (dir_name, file_name) = hex_hash.split_at(2);

        let obj_dir = self.git_dir.join("objects").join(dir_name);
        fs::create_dir_all(&obj_dir).map_err(|e| e.to_string())?;

        let obj_file = obj_dir.join(file_name);
        let mut f = fs::File::create(obj_file).map_err(|e| e.to_string())?;
        f.write_all(content).map_err(|e| e.to_string())?;

        Ok(hex_hash)
    }

    /// Creates a commit object and updates current branch ref.
    pub fn create_commit(&self, message: &str, author: &str) -> Result<String, String> {
        let branch = self.current_branch();
        let commit_data = format!("tree {}\nauthor {}\n\n{}\n", "0000000000000000000000000000000000000000", author, message);
        let commit_hash = self.create_blob(commit_data.as_bytes())?;

        let ref_path = self.git_dir.join("refs").join("heads").join(&branch);
        fs::write(ref_path, format!("{}\n", commit_hash)).map_err(|e| e.to_string())?;

        // Append to commit log file
        let log_path = self.git_dir.join("LOGS");
        let mut f = fs::OpenOptions::new().create(true).append(true).open(log_path).map_err(|e| e.to_string())?;
        writeln!(f, "{}\t{}", commit_hash, message).map_err(|e| e.to_string())?;

        Ok(commit_hash)
    }

    /// Returns list of modified or untracked files.
    pub fn status(&self) -> Result<Vec<StatusEntry>, String> {
        let mut entries = Vec::new();
        if let Ok(dir) = fs::read_dir(&self.work_tree) {
            for entry in dir.flatten() {
                let name = entry.file_name().to_string_lossy().to_string();
                if name == ".git" || name == "target" {
                    continue;
                }
                entries.push(StatusEntry {
                    path: name,
                    status: "untracked",
                });
            }
        }
        Ok(entries)
    }

    /// Returns recent commit log history.
    pub fn log(&self) -> Result<Vec<CommitLog>, String> {
        let log_path = self.git_dir.join("LOGS");
        if !log_path.exists() {
            return Ok(Vec::new());
        }

        let text = fs::read_to_string(log_path).map_err(|e| e.to_string())?;
        let mut logs = Vec::new();
        for line in text.lines() {
            let parts: Vec<&str> = line.splitn(2, '\t').collect();
            if parts.len() == 2 {
                logs.push(CommitLog {
                    hash: parts[0].to_string(),
                    message: parts[1].to_string(),
                });
            }
        }
        logs.reverse();
        Ok(logs)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_git_init_commit_log() {
        let temp = std::env::temp_dir().join("rusty_git_test_repo");
        let _ = fs::remove_dir_all(&temp);

        let repo = Repository::init(&temp).expect("init failed");
        assert_eq!(repo.current_branch(), "main");

        let commit_hash = repo.create_commit("Initial commit", "Developer <dev@rustymill.org>").expect("commit failed");
        assert!(!commit_hash.is_empty());

        let logs = repo.log().expect("log failed");
        assert_eq!(logs.len(), 1);
        assert_eq!(logs[0].message, "Initial commit");

        let _ = fs::remove_dir_all(&temp);
    }
}
