//! Path-allowlisted config file read/write, backing `fedora_read_config`/
//! `fedora_write_config`. Not a trait like [`crate::ports::SystemController`]/
//! [`crate::ports::PackageController`] -- it's plain `std::fs` with an
//! allowlist check in front, nothing to mock a Fedora box away from.

use std::io::{Read, Write};
use std::path::{Path, PathBuf};
use std::sync::Arc;

use crate::allowlist::{Allowlist, ConfigTarget};
use crate::error::AgentError;

pub struct ConfigStore {
    allowlist: Arc<Allowlist>,
}

impl ConfigStore {
    pub fn new(allowlist: Arc<Allowlist>) -> Self {
        Self { allowlist }
    }

    /// Reads `path`'s contents. Refuses any path outside the config-path
    /// allowlist before touching the filesystem, and opens it beneath its
    /// allowed prefix without following a symlink at any component.
    pub fn read(&self, path: &str) -> Result<String, AgentError> {
        let target = self.allowlist.resolve_config_path(Path::new(path))?;
        let mut file = rusty_confined_fs::open_for_read(&target.root, &target.rel)
            .map_err(|e| refused(&target, e))?;
        let mut out = String::new();
        file.read_to_string(&mut out)?;
        Ok(out)
    }

    /// Writes `content` to `path`, replacing whatever was there. Refuses
    /// any path outside the config-path allowlist before touching the
    /// filesystem. When `backup` is true and the file already exists, a
    /// `.bak` copy of the *previous* contents is written first -- best-
    /// effort undo for a bad edit, not a version history.
    pub fn write(&self, path: &str, content: &str, backup: bool) -> Result<(), AgentError> {
        let target = self.allowlist.resolve_config_path(Path::new(path))?;
        let full = target.path();
        if backup && full.exists() {
            let previous = self.read(&full.to_string_lossy())?;
            let bak = self.allowlist.resolve_config_path(&backup_path(&full))?;
            write_confined(&bak, previous.as_bytes())?;
        }
        write_confined(&target, content.as_bytes())
    }
}

/// Writes `content` to `target` beneath its allowed prefix, never following
/// a symlink at any component (`rusty_confined_fs`), so one swapped in
/// after the allowlist check is refused rather than written through.
fn write_confined(target: &ConfigTarget, content: &[u8]) -> Result<(), AgentError> {
    let mut file = rusty_confined_fs::open_for_write(&target.root, &target.rel)
        .map_err(|e| refused(target, e))?
        .file;
    file.set_len(0)?;
    file.write_all(content)?;
    file.sync_all()?;
    Ok(())
}

/// A confined open that failed: a symlink or non-regular file is a refusal,
/// anything else an I/O error.
fn refused(target: &ConfigTarget, e: std::io::Error) -> AgentError {
    match e.kind() {
        std::io::ErrorKind::NotFound | std::io::ErrorKind::PermissionDenied => e.into(),
        _ => AgentError::PathNotAllowed(target.path().display().to_string()),
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
        assert_eq!(
            store.read(path_str).expect("read succeeds"),
            "hello=world\n"
        );
    }

    #[test]
    fn write_with_backup_preserves_the_previous_content() {
        let (allowlist, dir) = sandbox();
        let store = ConfigStore::new(allowlist);
        let path = dir.join("test.conf");
        let path_str = path.to_str().expect("utf8 path");

        store
            .write(path_str, "version=1\n", true)
            .expect("first write");
        store
            .write(path_str, "version=2\n", true)
            .expect("second write, backed up");

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

        let err = store
            .read("/etc/shadow")
            .expect_err("outside the allowlist");
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

    /// Design review 3.2: the allowlist was lexical, so a symlink inside an
    /// allowed prefix reached any file. A symlinked file and a symlinked
    /// directory are both refused now, for read and write.
    #[test]
    fn a_symlink_inside_an_allowed_prefix_is_refused() {
        let (allowlist, dir) = sandbox();
        let store = ConfigStore::new(allowlist);
        let outside = dir.with_extension("outside");
        std::fs::create_dir_all(&outside).expect("test setup");
        let victim = outside.join("shadow");
        std::fs::write(&victim, "secret").expect("test setup");
        std::os::unix::fs::symlink(&victim, dir.join("file.conf")).expect("test setup");
        std::os::unix::fs::symlink(&outside, dir.join("sub")).expect("test setup");

        for p in [dir.join("file.conf"), dir.join("sub/shadow")] {
            let p = p.to_str().expect("test setup").to_string();
            assert!(
                matches!(store.read(&p), Err(AgentError::PathNotAllowed(_))),
                "{p}"
            );
            assert!(
                matches!(
                    store.write(&p, "pwned", true),
                    Err(AgentError::PathNotAllowed(_))
                ),
                "{p}"
            );
        }
        assert_eq!(
            std::fs::read_to_string(&victim).expect("test setup"),
            "secret"
        );
        assert!(!outside.join("shadow.bak").exists());
    }

    /// The allowlist check and the open are separate steps. A directory
    /// swapped for a symlink after the check (simulated by building the
    /// checked target directly) is refused at the open, not written
    /// through.
    #[test]
    fn a_symlink_swapped_in_after_the_check_is_refused_at_the_open() {
        let (_, dir) = sandbox();
        let root = dir.canonicalize().expect("test setup");
        let outside = dir.with_extension("swapped");
        std::fs::create_dir_all(&outside).expect("test setup");
        std::fs::write(outside.join("unit.conf"), "secret").expect("test setup");
        std::os::unix::fs::symlink(&outside, root.join("sub")).expect("test setup");

        let target = ConfigTarget {
            root,
            rel: PathBuf::from("sub/unit.conf"),
        };
        assert!(matches!(
            write_confined(&target, b"pwned"),
            Err(AgentError::PathNotAllowed(_))
        ));
        assert_eq!(
            std::fs::read_to_string(outside.join("unit.conf")).expect("test setup"),
            "secret"
        );
    }
}
