//! Path-allowlisted config file read/write, backing `fedora_read_config`/
//! `fedora_write_config`. Not a trait like [`crate::ports::SystemController`]/
//! [`crate::ports::PackageController`] -- it's plain `std::fs` with an
//! allowlist check in front, nothing to mock a Fedora box away from.

use std::path::{Path, PathBuf};
use std::sync::Arc;

use crate::allowlist::Allowlist;
use crate::error::AgentError;

pub struct ConfigStore {
    allowlist: Arc<Allowlist>,
}

impl ConfigStore {
    pub fn new(allowlist: Arc<Allowlist>) -> Self {
        Self { allowlist }
    }

    /// Reads `path`'s contents. Refuses any path outside the config-path
    /// allowlist before touching the filesystem.
    pub fn read(&self, path: &str) -> Result<String, AgentError> {
        let path = self.allowlist.check_config_path(Path::new(path))?;
        Ok(std::fs::read_to_string(&path)?)
    }

    /// Writes `content` to `path`, replacing whatever was there. Refuses
    /// any path outside the config-path allowlist before touching the
    /// filesystem. When `backup` is true and the file already exists, a
    /// `.bak` copy of the *previous* contents is written first -- best-
    /// effort undo for a bad edit, not a version history.
    pub fn write(&self, path: &str, content: &str, backup: bool) -> Result<(), AgentError> {
        let path = self.allowlist.check_config_path(Path::new(path))?;
        if backup && path.exists() {
            std::fs::copy(&path, backup_path(&path))?;
        }
        std::fs::write(&path, content)?;
        Ok(())
    }
}

fn backup_path(path: &Path) -> PathBuf {
    let mut name = path.as_os_str().to_os_string();
    name.push(".bak");
    PathBuf::from(name)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::allowlist::AllowlistConfig;

    /// A fresh temp directory under the OS temp dir, allowlisted as the
    /// only permitted config-path prefix. Not cleaned up afterward --
    /// tests run in an ephemeral CI/container filesystem, same as every
    /// other `std::env::temp_dir()`-based test in this workspace.
    fn sandbox() -> (Arc<Allowlist>, PathBuf) {
        let dir = std::env::temp_dir().join(format!(
            "rusty_fedora_agent_test_{}_{}",
            std::process::id(),
            std::time::SystemTime::now()
                .duration_since(std::time::UNIX_EPOCH)
                .map(|d| d.as_nanos())
                .unwrap_or_default()
        ));
        std::fs::create_dir_all(&dir).expect("create sandbox dir");
        let allowlist = Arc::new(Allowlist::new(AllowlistConfig {
            units: Vec::new(),
            packages: Vec::new(),
            config_path_prefixes: vec![dir.clone()],
        }));
        (allowlist, dir)
    }

    #[test]
    fn write_then_read_round_trips() {
        let (allowlist, dir) = sandbox();
        let store = ConfigStore::new(allowlist);
        let path = dir.join("test.conf");
        let path_str = path.to_str().expect("utf8 path");

        store
            .write(path_str, "hello=world\n", false)
            .expect("write succeeds");
        assert_eq!(store.read(path_str).expect("read succeeds"), "hello=world\n");
    }

    #[test]
    fn write_with_backup_preserves_the_previous_content() {
        let (allowlist, dir) = sandbox();
        let store = ConfigStore::new(allowlist);
        let path = dir.join("test.conf");
        let path_str = path.to_str().expect("utf8 path");

        store.write(path_str, "version=1\n", true).expect("first write");
        store.write(path_str, "version=2\n", true).expect("second write, backed up");

        assert_eq!(store.read(path_str).expect("read current"), "version=2\n");
        let bak = std::fs::read_to_string(format!("{path_str}.bak")).expect("read backup");
        assert_eq!(bak, "version=1\n");
    }

    #[test]
    fn a_first_write_with_backup_requested_does_not_fail_when_nothing_exists_yet() {
        let (allowlist, dir) = sandbox();
        let store = ConfigStore::new(allowlist);
        let path = dir.join("new.conf");
        let path_str = path.to_str().expect("utf8 path");

        store
            .write(path_str, "fresh=1\n", true)
            .expect("no prior file to back up is not an error");
        assert!(!Path::new(&format!("{path_str}.bak")).exists());
    }

    #[test]
    fn reading_a_path_outside_the_allowlist_is_rejected() {
        let (allowlist, _dir) = sandbox();
        let store = ConfigStore::new(allowlist);

        let err = store.read("/etc/shadow").expect_err("outside the allowlist");
        assert!(matches!(err, AgentError::PathNotAllowed(_)));
    }

    #[test]
    fn writing_a_path_outside_the_allowlist_is_rejected_before_touching_disk() {
        let (allowlist, _dir) = sandbox();
        let store = ConfigStore::new(allowlist);

        let err = store
            .write("/etc/shadow", "root::0:0:::", false)
            .expect_err("outside the allowlist");
        assert!(matches!(err, AgentError::PathNotAllowed(_)));
        assert!(!Path::new("/etc/shadow.bak").exists());
    }
}
