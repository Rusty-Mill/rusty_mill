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

/// Windows keeps machine-local (non-roaming) data separately, and several
/// tools use it rather than Roaming. On macOS and Linux there is no such
/// split, so this collapses onto the same directory.
fn local_data(home: &Path) -> PathBuf {
    if cfg!(target_os = "windows") {
        home.join("AppData").join("Local")
    } else {
        app_support(home)
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
        SourceId::Cursor => vscode_fork_roots(&home, &support, &config, "Cursor"),
        SourceId::Kiro => vscode_fork_roots(&home, &support, &config, "Kiro"),
        SourceId::Antigravity => vscode_fork_roots(&home, &support, &config, "Antigravity"),

        // Zed keeps agent threads in its own SQLite database.
        // Zed keeps agent threads in its own SQLite database. It uses a
        // capitalised directory under Application Support on macOS, a
        // lowercase one under XDG data on Linux, and Local (not Roaming)
        // AppData on Windows.
        SourceId::Zed => {
            let mut out = vec![support.join("Zed").join("threads")];
            for candidate in [
                local_data(&home).join("Zed").join("threads"),
                home.join(".local")
                    .join("share")
                    .join("zed")
                    .join("threads"),
            ] {
                if !out.contains(&candidate) {
                    out.push(candidate);
                }
            }
            out
        }
    }
}

fn vscode_fork_roots(home: &Path, support: &Path, config: &Path, name: &str) -> Vec<PathBuf> {
    let mut out = vec![support.join(name).join("User")];
    // Roaming is where Code and its forks put `User` on Windows, but a
    // portable or per-machine install lands in Local instead.
    for root in [config, &local_data(home)] {
        let candidate = root.join(name).join("User");
        if !out.contains(&candidate) {
            out.push(candidate);
        }
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
    use std::sync::{Mutex, MutexGuard};

    /// `HOME_ENV` and `DATA_ENV` are process-global, so tests that install a
    /// fixture home must not overlap — otherwise one test's cleanup makes
    /// another fall back to the real home directory mid-assertion.
    static ENV_LOCK: Mutex<()> = Mutex::new(());

    fn locked() -> MutexGuard<'static, ()> {
        ENV_LOCK.lock().unwrap_or_else(|e| e.into_inner())
    }

    #[test]
    fn index_path_honours_override() {
        let _guard = locked();
        let dir = tempfile::tempdir().unwrap();
        std::env::set_var(DATA_ENV, dir.path());
        let p = index_path();
        std::env::remove_var(DATA_ENV);
        assert_eq!(p.file_name().unwrap(), "inventory.sqlite3");
        assert!(p.starts_with(dir.path()));
    }

    /// Every candidate must sit under the (overridden) home directory —
    /// a path table that reaches outside it would read another account's data.
    #[test]
    fn candidate_roots_stay_inside_the_home_directory() {
        let _guard = locked();
        let dir = tempfile::tempdir().unwrap();
        std::env::set_var(HOME_ENV, dir.path());
        let all: Vec<(SourceId, PathBuf)> = SourceId::ALL
            .into_iter()
            .flat_map(|id| candidates(id).into_iter().map(move |p| (id, p)))
            .collect();
        std::env::remove_var(HOME_ENV);
        for (id, path) in all {
            assert!(
                path.starts_with(dir.path()),
                "{id} probes {} outside the home directory",
                path.display()
            );
        }
    }

    #[test]
    fn candidate_roots_are_free_of_duplicates() {
        let _guard = locked();
        let dir = tempfile::tempdir().unwrap();
        std::env::set_var(HOME_ENV, dir.path());
        let per_source: Vec<(SourceId, Vec<PathBuf>)> = SourceId::ALL
            .into_iter()
            .map(|id| (id, candidates(id)))
            .collect();
        std::env::remove_var(HOME_ENV);
        for (id, mut roots) in per_source {
            let before = roots.len();
            roots.sort();
            roots.dedup();
            assert_eq!(before, roots.len(), "{id} probes the same root twice");
        }
    }

    #[test]
    fn every_source_has_at_least_one_candidate_root() {
        let _guard = locked();
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
