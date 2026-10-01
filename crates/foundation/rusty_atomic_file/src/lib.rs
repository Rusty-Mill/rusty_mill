//! Replace a file's contents crash-atomically.
//!
//! [`write`] and [`write_private`] write the new bytes to a fresh sibling
//! temp file, `fsync` it, rename it over the target, then `fsync` the
//! parent directory (Unix) so the rename itself survives a power loss.
//! After a crash a reader sees either the old file or the new one, never
//! a truncated or half-written one. This is the same sequence as rustils'
//! `Dir::write_atomic`, for callers that hold a `std::path::Path` (rustils
//! has no filesystem backend on macOS).
//!
//! Concurrent writers to one path are safe: each uses its own temp name,
//! and the last rename wins.

use std::ffi::OsString;
use std::fs::{self, File, OpenOptions};
use std::io::{self, Write};
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicU64, Ordering};

/// Replaces `path` with `bytes`, creating it if it does not exist.
///
/// A new file gets the process's default permissions; see
/// [`write_private`] for secrets.
///
/// # Errors
/// Any I/O error from creating, writing, syncing or renaming the temp
/// file, or from syncing the directory. `InvalidInput` if `path` has no
/// file name. On error the target is unchanged and the temp file is
/// removed.
pub fn write(path: &Path, bytes: &[u8]) -> io::Result<()> {
    replace(path, bytes, Permissions::Default)
}

/// [`write`] for secrets: on Unix the temp file is created `0600`, so the
/// contents are never readable by other users, not even briefly before a
/// `chmod`. Elsewhere it behaves like [`write`], and the file inherits its
/// directory's ACL.
///
/// # Errors
/// As [`write`].
pub fn write_private(path: &Path, bytes: &[u8]) -> io::Result<()> {
    replace(path, bytes, Permissions::OwnerOnly)
}

#[derive(Clone, Copy)]
enum Permissions {
    Default,
    OwnerOnly,
}

fn replace(path: &Path, bytes: &[u8], permissions: Permissions) -> io::Result<()> {
    let tmp = temp_sibling(path)?;
    if let Err(err) = write_synced(&tmp, bytes, permissions) {
        let _ = fs::remove_file(&tmp);
        return Err(err);
    }
    if let Err(err) = fs::rename(&tmp, path) {
        let _ = fs::remove_file(&tmp);
        return Err(err);
    }
    sync_parent_dir(path)
}

/// A name in `path`'s directory that no other writer uses: hidden, and
/// unique per process and call.
fn temp_sibling(path: &Path) -> io::Result<PathBuf> {
    static COUNTER: AtomicU64 = AtomicU64::new(0);
    let Some(name) = path.file_name() else {
        return Err(io::Error::new(
            io::ErrorKind::InvalidInput,
            "atomic write target has no file name",
        ));
    };
    let mut tmp = OsString::from(".");
    tmp.push(name);
    tmp.push(format!(
        ".{}.{}.tmp",
        std::process::id(),
        COUNTER.fetch_add(1, Ordering::Relaxed)
    ));
    Ok(path.with_file_name(tmp))
}

fn write_synced(tmp: &Path, bytes: &[u8], permissions: Permissions) -> io::Result<()> {
    // `create_new` never reuses or follows anything already at `tmp`.
    let mut options = OpenOptions::new();
    options.write(true).create_new(true);
    #[cfg(unix)]
    if let Permissions::OwnerOnly = permissions {
        use std::os::unix::fs::OpenOptionsExt;
        options.mode(0o600);
    }
    #[cfg(not(unix))]
    let _ = permissions;
    let mut file: File = options.open(tmp)?;
    file.write_all(bytes)?;
    file.sync_all()
}

#[cfg(unix)]
fn sync_parent_dir(path: &Path) -> io::Result<()> {
    let dir = match path.parent() {
        Some(dir) if !dir.as_os_str().is_empty() => dir,
        _ => Path::new("."),
    };
    File::open(dir)?.sync_all()
}

/// Windows cannot open a directory to sync it; the rename (`MoveFileExW`
/// with replace) is the durability point there.
#[cfg(not(unix))]
#[allow(clippy::unnecessary_wraps)] // same signature as the Unix version
fn sync_parent_dir(_path: &Path) -> io::Result<()> {
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    /// A fresh directory per test, removed on drop.
    struct Scratch(PathBuf);

    impl Scratch {
        fn new(label: &str) -> Self {
            static N: AtomicU64 = AtomicU64::new(0);
            let dir = std::env::temp_dir().join(format!(
                "rusty_atomic_file-{label}-{}-{}",
                std::process::id(),
                N.fetch_add(1, Ordering::Relaxed)
            ));
            fs::create_dir_all(&dir).expect("create scratch dir");
            Self(dir)
        }

        fn names(&self) -> Vec<String> {
            let mut names: Vec<String> = fs::read_dir(&self.0)
                .expect("read scratch dir")
                .map(|e| e.expect("entry").file_name().to_string_lossy().into_owned())
                .collect();
            names.sort();
            names
        }
    }

    impl Drop for Scratch {
        fn drop(&mut self) {
            let _ = fs::remove_dir_all(&self.0);
        }
    }

    #[test]
    fn creates_then_replaces_and_leaves_no_temp_file() {
        let dir = Scratch::new("replace");
        let target = dir.0.join("state.json");
        write(&target, b"first, longer contents").expect("create");
        write(&target, b"second").expect("replace");
        assert_eq!(fs::read(&target).expect("read"), b"second");
        assert_eq!(dir.names(), ["state.json"]);
    }

    #[test]
    fn empty_contents_produce_an_empty_file() {
        let dir = Scratch::new("empty");
        let target = dir.0.join("empty");
        write(&target, b"").expect("write");
        assert_eq!(fs::read(&target).expect("read"), b"");
    }

    #[test]
    fn a_failed_rename_leaves_the_target_and_no_temp_file() {
        let dir = Scratch::new("failed-rename");
        // A non-empty directory cannot be replaced by a file.
        let target = dir.0.join("occupied");
        fs::create_dir(&target).expect("mkdir");
        fs::write(target.join("keep"), b"x").expect("populate");

        assert!(write(&target, b"new").is_err());
        assert!(target.join("keep").exists());
        assert_eq!(dir.names(), ["occupied"]);
    }

    #[test]
    fn a_missing_parent_directory_is_an_error_not_a_create() {
        let dir = Scratch::new("missing-parent");
        let target = dir.0.join("absent").join("file");
        let err = write(&target, b"x").expect_err("no parent");
        assert_eq!(err.kind(), io::ErrorKind::NotFound);
        assert_eq!(dir.names(), Vec::<String>::new());
    }

    #[test]
    fn a_path_without_a_file_name_is_invalid_input() {
        let err = write(Path::new("/"), b"x").expect_err("root has no name");
        assert_eq!(err.kind(), io::ErrorKind::InvalidInput);
    }

    #[test]
    fn concurrent_writers_never_leave_a_mixed_or_partial_file() {
        let dir = Scratch::new("concurrent");
        let target = dir.0.join("shared");
        let payloads: Vec<Vec<u8>> = (0u8..8).map(|i| vec![i; 64 * 1024]).collect();
        std::thread::scope(|s| {
            for payload in &payloads {
                let target = &target;
                s.spawn(move || write(target, payload).expect("write"));
            }
        });
        let got = fs::read(&target).expect("read");
        assert!(
            payloads.contains(&got),
            "the file is one writer's whole payload"
        );
        assert_eq!(dir.names(), ["shared"]);
    }

    #[cfg(unix)]
    #[test]
    fn write_private_creates_an_owner_only_file() {
        use std::os::unix::fs::PermissionsExt;
        let dir = Scratch::new("private");
        let target = dir.0.join("secret");
        write_private(&target, b"key").expect("write");
        let mode = fs::metadata(&target).expect("stat").permissions().mode();
        assert_eq!(mode & 0o777, 0o600);
    }

    #[cfg(unix)]
    #[test]
    fn write_private_tightens_a_previously_readable_file() {
        use std::os::unix::fs::PermissionsExt;
        let dir = Scratch::new("tighten");
        let target = dir.0.join("secret");
        fs::write(&target, b"old").expect("seed");
        fs::set_permissions(&target, fs::Permissions::from_mode(0o644)).expect("chmod");
        write_private(&target, b"new").expect("write");
        let mode = fs::metadata(&target).expect("stat").permissions().mode();
        assert_eq!(mode & 0o777, 0o600);
    }

    #[cfg(unix)]
    #[test]
    fn replacing_a_symlink_replaces_the_link_not_its_target() {
        let dir = Scratch::new("symlink");
        let outside = dir.0.join("outside");
        fs::write(&outside, b"untouched").expect("seed");
        let link = dir.0.join("link");
        std::os::unix::fs::symlink(&outside, &link).expect("symlink");

        write(&link, b"new").expect("write");
        assert_eq!(fs::read(&outside).expect("read"), b"untouched");
        assert!(
            !fs::symlink_metadata(&link)
                .expect("lstat")
                .file_type()
                .is_symlink()
        );
        assert_eq!(fs::read(&link).expect("read"), b"new");
    }
}
