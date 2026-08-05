//! Where each tool keeps its history, per platform.
//!
//! The reviewed product is macOS-only; these tables add the Linux and Windows
//! equivalents so the core is portable. Every candidate is probed and the ones
//! that do not exist are simply skipped — "it reads whichever of them are
//! installed and ignores the rest".

use crate::model::SourceId;
use std::path::{Path, PathBuf};

/// Override the home directory. Used by the test-suite to point the whole
/// path layer at a fixture tree.
pub const HOME_ENV: &str = "INVENTORY_HOME";

/// Override the index location.
pub const DATA_ENV: &str = "INVENTORY_DATA_DIR";

pub fn home_dir() -> Option<PathBuf> {
    if let Some(h) = std::env::var_os(HOME_ENV) {
        return Some(PathBuf::from(h));
    }
    directories::BaseDirs::new().map(|b| b.home_dir().to_path_buf())
}

/// `~/Library/Application Support` on macOS, `~/.local/share` on Linux,
/// `~/AppData/Roaming` on Windows — resolved against our overridable home so
/// tests stay hermetic.
fn app_support(home: &Path) -> PathBuf {
    if cfg!(target_os = "macos") {
        home.join("Library").join("Application Support")
    } else if cfg!(target_os = "windows") {
        home.join("AppData").join("Roaming")
    } else {
        home.join(".local").join("share")
    }
}

/// Config root, which is where the VS Code forks live on Linux.
fn config_root(home: &Path) -> PathBuf {
    if cfg!(target_os = "linux") {
        home.join(".config")
    } else {
        app_support(home)
    }
}

/// The single file everything lives in.
///
/// macOS: `~/Library/Application Support/site.myinventory.app/inventory.sqlite3`
pub fn index_path() -> PathBuf {
    if let Some(dir) = std::env::var_os(DATA_ENV) {
        return PathBuf::from(dir).join("inventory.sqlite3");
    }
    let home = home_dir().unwrap_or_else(|| PathBuf::from("."));
    app_support(&home)
        .join("site.myinventory.app")
        .join("inventory.sqlite3")
}

/// Candidate roots for a source, most-canonical first. Non-existent entries
/// are filtered by the caller.
pub fn candidates(source: SourceId) -> Vec<PathBuf> {
    let Some(home) = home_dir() else {
        return Vec::new();
    };
    let support = app_support(&home);
    let config = config_root(&home);

    match source {
        // Claude Code writes one JSONL transcript per session, filed under a
        // directory named for the project it ran in.
        SourceId::ClaudeCode => vec![home.join(".claude").join("projects")],

        // Codex writes rollout JSONL files bucketed by date.
        SourceId::Codex => vec![home.join(".codex").join("sessions")],

        // The three VS Code forks share Code's storage layout: a LevelDB-era
        // `ItemTable` key/value SQLite database, one global and one per
        // workspace.
        SourceId::Cursor => vscode_fork_roots(&support, &config, "Cursor"),
        SourceId::Kiro => vscode_fork_roots(&support, &config, "Kiro"),
        SourceId::Antigravity => vscode_fork_roots(&support, &config, "Antigravity"),

        // Zed keeps agent threads in its own SQLite database.
        SourceId::Zed => vec![
            support.join("Zed").join("threads"),
            home.join(".local")
                .join("share")
                .join("zed")
                .join("threads"),
        ],
    }
}

fn vscode_fork_roots(support: &Path, config: &Path, name: &str) -> Vec<PathBuf> {
    let mut out = vec![support.join(name).join("User")];
    let from_config = config.join(name).join("User");
    if !out.contains(&from_config) {
        out.push(from_config);
    }
    out
}

/// Roots that actually exist on this machine.
pub fn existing_roots(source: SourceId) -> Vec<PathBuf> {
    candidates(source)
        .into_iter()
        .filter(|p| p.exists())
        .collect()
}

pub fn is_installed(source: SourceId) -> bool {
    !existing_roots(source).is_empty()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn index_path_honours_override() {
        let dir = tempfile::tempdir().unwrap();
        std::env::set_var(DATA_ENV, dir.path());
        let p = index_path();
        std::env::remove_var(DATA_ENV);
        assert_eq!(p.file_name().unwrap(), "inventory.sqlite3");
        assert!(p.starts_with(dir.path()));
    }

    #[test]
    fn every_source_has_at_least_one_candidate_root() {
        let dir = tempfile::tempdir().unwrap();
        std::env::set_var(HOME_ENV, dir.path());
        for id in SourceId::ALL {
            assert!(
                !candidates(id).is_empty(),
                "{id} has no candidate root on this platform"
            );
        }
        std::env::remove_var(HOME_ENV);
    }
}
