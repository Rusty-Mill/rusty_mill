//! Where the daemon's socket lives, duplicated from
//! `sessionmgr-daemon::paths` rather than shared.
//!
//! This crate depends on `sessionmgr-protocol` only, the same boundary
//! `sessionmgr-tui` already established (see that crate's `client.rs`
//! module docs) -- a UI that cannot name a process type cannot
//! accidentally spawn one directly, only ever through the `sessionmgr`
//! binary's own subcommands. `state_root`/`daemon_socket` are small
//! enough, and change rarely enough, that duplicating them here costs
//! far less than a dependency that would break that boundary.

use std::path::PathBuf;

/// Same environment variable `sessionmgr-daemon::paths::HOME_ENV` uses --
/// kept identical so a state root set for one is honored by the other.
pub const HOME_ENV: &str = "SESSIONMGR_HOME";

pub fn state_root() -> Result<PathBuf, String> {
    if let Some(dir) = std::env::var_os(HOME_ENV) {
        let dir = PathBuf::from(dir);
        if dir.as_os_str().is_empty() {
            return Err(format!("{HOME_ENV} is set but empty"));
        }
        return Ok(dir);
    }
    #[cfg(windows)]
    {
        let base = std::env::var_os("LOCALAPPDATA")
            .ok_or_else(|| format!("neither {HOME_ENV} nor LOCALAPPDATA is set"))?;
        Ok(PathBuf::from(base).join("sessionmgr"))
    }
    #[cfg(unix)]
    {
        if let Some(base) = std::env::var_os("XDG_STATE_HOME") {
            return Ok(PathBuf::from(base).join("sessionmgr"));
        }
        let home = std::env::var_os("HOME")
            .ok_or_else(|| format!("neither {HOME_ENV}, XDG_STATE_HOME, nor HOME is set"))?;
        Ok(PathBuf::from(home).join(".local/state/sessionmgr"))
    }
}

/// The daemon's public socket -- `<root>/s/d.sock`, matching
/// `sessionmgr-daemon::paths::daemon_socket` exactly.
pub fn daemon_socket(root: &std::path::Path) -> PathBuf {
    root.join("s").join("d.sock")
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn daemon_socket_is_the_flat_layout_daemon_paths_also_uses() {
        assert_eq!(
            daemon_socket(&PathBuf::from("/r")),
            PathBuf::from("/r/s/d.sock")
        );
    }

    #[test]
    fn an_empty_home_env_is_rejected_rather_than_silently_using_the_cwd() {
        // Mirrors `sessionmgr-daemon::paths`'s own test of the identical
        // rule (same env var, same reasoning: "" is a common scripting
        // slip, and treating it as "the current directory" would scatter
        // state into whatever repo the user happened to be standing in).
        // `std::env::set_var` mutates process-global state shared with
        // every other test in this binary, so this is the only test here
        // that touches `SESSIONMGR_HOME`.
        let previous = std::env::var_os(HOME_ENV);
        std::env::set_var(HOME_ENV, "");
        let result = state_root();
        match previous {
            Some(value) => std::env::set_var(HOME_ENV, value),
            None => std::env::remove_var(HOME_ENV),
        }
        assert!(result.is_err());
    }
}
