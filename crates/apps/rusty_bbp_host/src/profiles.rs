//! The profile set a task freezes when it opens: which commands the runner
//! executes, where they may read, their environment and limits. Stored at
//! `<dir>/profiles/<task>.json`; its SHA-256 is the task's `profile_digest`,
//! so a report whose digest differs is refused by the core.

use rusty_bbp::*;
use rusty_serde::{Deserialize, Serialize};
use std::path::{Path, PathBuf};

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct Profile {
    pub name: String,
    pub program: String,
    pub args: Vec<String>,
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct LimitSpec {
    pub cpu_secs: u64,
    pub wall_secs: u64,
    pub memory_bytes: u64,
    pub file_bytes: u64,
    pub open_files: u64,
    pub processes: u64,
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct ProfileSet {
    pub profiles: Vec<Profile>,
    /// Absolute directories the workload may read and execute from, beyond
    /// the checkout (toolchains, system libraries).
    pub read_roots: Vec<String>,
    /// The workload's complete environment.
    pub env: Vec<(String, String)>,
    pub limits: LimitSpec,
    /// How `Report.tree` is computed; informative.
    pub tree: String,
}

impl ProfileSet {
    /// A profile set that runs one shell command, for tests and first runs.
    pub fn shell(name: &str, command: &str) -> ProfileSet {
        ProfileSet {
            profiles: vec![Profile {
                name: name.into(),
                program: "/bin/sh".into(),
                args: vec!["-c".into(), command.into()],
            }],
            read_roots: vec!["/bin".into(), "/usr".into(), "/lib".into(), "/lib64".into()],
            env: vec![("PATH".into(), "/usr/bin:/bin".into())],
            limits: LimitSpec {
                cpu_secs: 60,
                wall_secs: 120,
                memory_bytes: 1 << 30,
                file_bytes: 1 << 28,
                open_files: 256,
                processes: 64,
            },
            tree: "sha256(git write-tree id)".into(),
        }
    }

    pub fn to_bytes(&self) -> Result<Vec<u8>, String> {
        rusty_serde::json::to_string(self)
            .map(String::into_bytes)
            .map_err(|e| e.to_string())
    }

    pub fn from_bytes(bytes: &[u8]) -> Result<ProfileSet, String> {
        let text = std::str::from_utf8(bytes).map_err(|e| e.to_string())?;
        rusty_serde::json::from_str(text).map_err(|e| e.to_string())
    }
}

/// `<dir>/profiles/<sha256(task id)>.json`: injective, so no two tasks share a file.
pub fn path(dir: &Path, task: &TaskId) -> PathBuf {
    dir.join("profiles")
        .join(format!("{}.json", Sha256::of(task.0.as_bytes()).hex()))
}

/// Read a profile set from a JSON file.
pub fn read_file(file: &Path) -> Result<ProfileSet, String> {
    let bytes = std::fs::read(file).map_err(|e| format!("{}: {e}", file.display()))?;
    ProfileSet::from_bytes(&bytes)
}

/// Write the task's profile set and return its digest.
pub fn freeze(dir: &Path, task: &TaskId, set: &ProfileSet) -> Result<Sha256, String> {
    if set.profiles.is_empty() {
        return Err("profile set has no profiles".into());
    }
    let bytes = set.to_bytes()?;
    let p = path(dir, task);
    if let Some(parent) = p.parent() {
        std::fs::create_dir_all(parent).map_err(|e| e.to_string())?;
    }
    rusty_atomic_file::write(&p, &bytes).map_err(|e| format!("profiles: {e}"))?;
    Ok(Sha256::of(&bytes))
}

/// Load the task's profile set and check it against the digest the task froze.
pub fn load(dir: &Path, task: &TaskId, expected: Sha256) -> Result<ProfileSet, String> {
    let bytes = std::fs::read(path(dir, task)).map_err(|e| format!("profiles: {e}"))?;
    if Sha256::of(&bytes) != expected {
        return Err("profile set on disk does not match the task's frozen digest".into());
    }
    ProfileSet::from_bytes(&bytes)
}
