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
//! and the last rename wins. Temp-name collisions are retried without
//! touching the existing filesystem entry.

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
/// file name. Before the rename, an error leaves the target unchanged and
/// removes any temp file created by this call. An error syncing the parent
/// directory occurs after replacement and cannot roll it back.
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
    replace_with(
        path,
        bytes,
        permissions,
        temp_sibling,
        |file, contents| file.write_all(contents),
        File::sync_all,
    )
}

const MAX_TEMP_ATTEMPTS: usize = 16;

/// The closures are a deliberately narrow test seam for candidate selection and
/// failures after this invocation has acquired ownership of a candidate.
fn replace_with<C, W, S>(
    path: &Path,
    bytes: &[u8],
    permissions: Permissions,
    mut candidate: C,
    mut write_file: W,
    mut sync_file: S,
) -> io::Result<()>
where
    C: FnMut(&Path) -> io::Result<PathBuf>,
    W: FnMut(&mut File, &[u8]) -> io::Result<()>,
    S: FnMut(&File) -> io::Result<()>,
{
    for _ in 0..MAX_TEMP_ATTEMPTS {
        let tmp = candidate(path)?;
        let mut file = match create_new(&tmp, permissions) {
            Ok(file) => file,
            Err(err) if err.kind() == io::ErrorKind::AlreadyExists => continue,
            Err(err) => return Err(err),
        };

        let result = write_file(&mut file, bytes).and_then(|()| sync_file(&file));
        drop(file); // Windows requires all handles closed before remove or rename.
        if let Err(err) = result {
            let _ = fs::remove_file(&tmp);
            return Err(err);
        }
        if let Err(err) = fs::rename(&tmp, path) {
            let _ = fs::remove_file(&tmp);
            return Err(err);
        }
        return sync_parent_dir(path);
    }

    Err(io::Error::new(
        io::ErrorKind::AlreadyExists,
        format!("could not create an atomic-write temp file after {MAX_TEMP_ATTEMPTS} attempts"),
    ))
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

fn create_new(tmp: &Path, permissions: Permissions) -> io::Result<File> {
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
    options.open(tmp)
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

    fn replace_with_candidates(
        target: &Path,
        bytes: &[u8],
        candidates: &[PathBuf],
    ) -> io::Result<()> {
        let mut candidates = candidates.iter().cloned();
        replace_with(
            target,
            bytes,
            Permissions::Default,
            |_| Ok(candidates.next().expect("enough candidates")),
            |file, contents| file.write_all(contents),
            File::sync_all,
        )
    }

    #[test]
    fn a_regular_file_collision_is_preserved_before_later_success() {
        let dir = Scratch::new("file-collision");
        let target = dir.0.join("target");
        fs::write(&target, b"old target").expect("seed target");
        let collision = dir.0.join("collision");
        fs::write(&collision, b"sentinel").expect("seed collision");
        let candidate = dir.0.join("candidate");

        replace_with_candidates(&target, b"new target", &[collision.clone(), candidate])
            .expect("retry succeeds");
        assert_eq!(fs::read(&collision).expect("read collision"), b"sentinel");
        assert_eq!(fs::read(&target).expect("read target"), b"new target");
    }

    #[test]
    fn a_directory_collision_is_preserved_before_later_success() {
        let dir = Scratch::new("directory-collision");
        let target = dir.0.join("target");
        let collision = dir.0.join("collision");
        fs::create_dir(&collision).expect("seed collision");
        fs::write(collision.join("sentinel"), b"keep").expect("populate collision");

        replace_with_candidates(
            &target,
            b"new",
            &[collision.clone(), dir.0.join("candidate")],
        )
        .expect("retry succeeds");
        assert_eq!(
            fs::read(collision.join("sentinel")).expect("read sentinel"),
            b"keep"
        );
    }

    #[cfg(unix)]
    #[test]
    fn a_symlink_collision_and_its_target_are_preserved() {
        let dir = Scratch::new("symlink-collision");
        let target = dir.0.join("target");
        let link_target = dir.0.join("link-target");
        fs::write(&link_target, b"sentinel").expect("seed link target");
        let collision = dir.0.join("collision");
        std::os::unix::fs::symlink(&link_target, &collision).expect("symlink");

        replace_with_candidates(
            &target,
            b"new",
            &[collision.clone(), dir.0.join("candidate")],
        )
        .expect("retry succeeds");
        assert!(
            fs::symlink_metadata(&collision)
                .expect("lstat")
                .file_type()
                .is_symlink()
        );
        assert_eq!(
            fs::read(&link_target).expect("read link target"),
            b"sentinel"
        );
    }

    #[test]
    fn collision_exhaustion_preserves_all_entries_and_the_target() {
        let dir = Scratch::new("collision-exhaustion");
        let target = dir.0.join("target");
        fs::write(&target, b"old target").expect("seed target");
        let candidates: Vec<PathBuf> = (0..MAX_TEMP_ATTEMPTS)
            .map(|index| dir.0.join(format!("collision-{index}")))
            .collect();
        for (index, candidate) in candidates.iter().enumerate() {
            fs::write(candidate, format!("sentinel-{index}")).expect("seed collision");
        }

        let err = replace_with_candidates(&target, b"new", &candidates).expect_err("exhausted");
        assert_eq!(err.kind(), io::ErrorKind::AlreadyExists);
        assert!(err.to_string().contains("16 attempts"));
        assert_eq!(fs::read(&target).expect("read target"), b"old target");
        for (index, candidate) in candidates.iter().enumerate() {
            assert_eq!(
                fs::read_to_string(candidate).expect("read collision"),
                format!("sentinel-{index}")
            );
        }
    }

    fn assert_owned_temp_is_cleaned_after_failure(sync_failure: bool) {
        let dir = Scratch::new(if sync_failure {
            "sync-failure"
        } else {
            "write-failure"
        });
        let target = dir.0.join("target");
        fs::write(&target, b"old target").expect("seed target");
        let candidate = dir.0.join("owned-temp");
        let err = replace_with(
            &target,
            b"new",
            Permissions::Default,
            |_| Ok(candidate.clone()),
            |file, contents| {
                if sync_failure {
                    file.write_all(contents)
                } else {
                    Err(io::Error::other("injected write failure"))
                }
            },
            |_| {
                if sync_failure {
                    Err(io::Error::other("injected sync failure"))
                } else {
                    Ok(())
                }
            },
        )
        .expect_err("injected failure");
        assert_eq!(err.kind(), io::ErrorKind::Other);
        assert_eq!(fs::read(&target).expect("read target"), b"old target");
        assert!(!candidate.exists(), "owned candidate was cleaned up");
    }

    #[test]
    fn a_write_failure_cleans_up_the_owned_temp_and_preserves_the_target() {
        assert_owned_temp_is_cleaned_after_failure(false);
    }

    #[test]
    fn a_file_sync_failure_cleans_up_the_owned_temp_and_preserves_the_target() {
        assert_owned_temp_is_cleaned_after_failure(true);
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
