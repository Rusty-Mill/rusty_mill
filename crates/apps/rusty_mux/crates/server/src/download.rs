//! `terminal.download` (C83): a file of the server's host, read a chunk at a
//! time for a copy to the client.

#![cfg_attr(not(any(target_os = "linux", windows)), allow(dead_code))]

use std::fs::{self, File};
#[cfg(any(target_os = "linux", windows))]
use std::io::{Read, Seek, SeekFrom};
use std::path::{Path, PathBuf};
use std::sync::{Arc, Mutex, MutexGuard, PoisonError};

/// The most bytes one reply carries before base64.
pub(crate) const CHUNK: usize = 192 << 10;
/// The largest file read; an offset is 32 bits, and this is the upload's limit.
pub(crate) const FILE_MAX: u64 = 2 << 30;

use crate::listing::is_network_path;

/// The file `path` names, taken relative to `base` (the block's working
/// directory), resolved to the path it really has.
fn locate(base: Option<&str>, path: &str) -> Result<PathBuf, String> {
    if is_network_path(path) {
        return Err("a network path is not read".into());
    }
    let file = match base {
        Some(base) => Path::new(base).join(path),
        None if Path::new(path).is_absolute() => path.into(),
        None => return Err("the program's working directory is unknown".into()),
    };
    let file = fs::canonicalize(&file).map_err(|e| format!("{}: {e}", file.display()))?;
    // Windows resolves a mapped or redirected path to its UNC form.
    if file.to_string_lossy().starts_with("\\\\?\\UNC\\") {
        return Err("a network path is not read".into());
    }
    Ok(file)
}

/// The state directory as it really is, or why not. With one to keep out and
/// no way to say where it is, nothing is read.
fn private_root(private: Option<&Path>) -> Result<Option<PathBuf>, String> {
    private
        .map(|dir| {
            fs::canonicalize(dir).map_err(|e| {
                format!("the server's state directory cannot be resolved ({e}), so no file is read")
            })
        })
        .transpose()
}

#[cfg(target_os = "linux")]
/// Checks the object `f` really is, not the name it was opened by: it is
/// the file `file` still names, and it is not inside `root`. Linux asks the
/// kernel for the opened object's own path; elsewhere the name is resolved
/// again and must still reach the same file.
fn verify(f: &File, file: &Path, root: Option<&Path>) -> Result<(), String> {
    let again = fs::canonicalize(file).map_err(|e| format!("{}: {e}", file.display()))?;
    if again != file {
        return Err(format!("{} changed while it was opened", file.display()));
    }
    #[cfg(unix)]
    {
        use std::os::unix::fs::MetadataExt;
        let (opened, named) = (
            f.metadata().map_err(|e| e.to_string())?,
            fs::metadata(file).map_err(|e| e.to_string())?,
        );
        if (opened.dev(), opened.ino()) != (named.dev(), named.ino()) {
            return Err(format!("{} changed while it was opened", file.display()));
        }
    }
    #[cfg(target_os = "linux")]
    if let Some(root) = root {
        use std::os::fd::AsRawFd;
        let real = fs::read_link(format!("/proc/self/fd/{}", f.as_raw_fd()))
            .map_err(|e| format!("the opened file cannot be placed ({e}), so it is not read"))?;
        if real.starts_with(root) {
            return Err("the server's own state directory is not read".into());
        }
    }
    let _ = (f, root);
    Ok(())
}

/// The reply for `path` from `offset`: its size, a newline, then the base64
/// of the next chunk (nothing past the end). A regular file of at most
/// [`FILE_MAX`], outside `private` (the server's state directory), judged on
/// the file as opened.
pub(crate) fn read(
    base: Option<&str>,
    path: &str,
    offset: u32,
    private: Option<&Path>,
    keep: &Protected,
) -> Result<String, String> {
    // Linux places the opened object by the kernel, Windows by its handle.
    // Elsewhere a path is not resolved at all (resolving may reach another
    // machine) and a file inside the state directory could not be told from
    // one outside: nothing is read until that platform has its own check.
    #[cfg(target_os = "linux")]
    return read_linux(base, path, offset, private, keep);
    #[cfg(windows)]
    return win::read(base, path, offset, private, keep);
    #[cfg(not(any(target_os = "linux", windows)))]
    {
        let _ = (base, path, offset, private, keep);
        Err("downloading is not supported on this platform yet".into())
    }
}

#[cfg(windows)]
#[path = "download_win.rs"]
mod win;

/// The device and inode of an opened file (Windows: volume serial and file index).
pub type FileId = (u64, u64);

/// The files downloads never read. A file is named by its path, looked up
/// again at each download, and, where the server loaded it, by the identity
/// of the very descriptor it was read from, so a rotated or hard-linked copy
/// of what is in force stays out. Shared by clones: the server updates it
/// when it reloads a file.
#[derive(Clone, Debug, Default)]
pub struct Protected(Arc<Mutex<Set>>);

#[derive(Debug, Default)]
struct Set {
    /// Kept out by path alone (the config file).
    paths: Vec<PathBuf>,
    /// Kept out by path and by the identity of what was loaded; until that
    /// is known, nothing is downloaded.
    loaded: Vec<(PathBuf, Option<FileId>)>,
    /// Counts the changes to the two lists, so a download can tell that the
    /// policy it judged a file by is no longer the one in force.
    generation: u64,
}

impl PartialEq for Protected {
    fn eq(&self, other: &Self) -> bool {
        Arc::ptr_eq(&self.0, &other.0) || {
            let (a, b) = (self.set(), other.set());
            a.paths == b.paths && a.loaded == b.loaded
        }
    }
}
impl Eq for Protected {}

impl Protected {
    fn set(&self) -> MutexGuard<'_, Set> {
        self.0.lock().unwrap_or_else(PoisonError::into_inner)
    }

    /// Keeps `path` out, by the path as it resolves at each download.
    pub fn path(&self, path: PathBuf) {
        let mut set = self.set();
        if !set.paths.contains(&path) {
            set.paths.push(path);
            set.generation += 1;
        }
    }

    /// Keeps `path` out and expects its loaded identity: until
    /// [`Protected::loaded`] gives it, no file is downloaded.
    pub fn require(&self, path: PathBuf) {
        let mut set = self.set();
        if !set.loaded.iter().any(|(p, _)| *p == path) {
            set.loaded.push((path, None));
            set.generation += 1;
        }
    }

    /// `path` was loaded from a descriptor with this identity; the one it
    /// replaces, if any, is no longer in force.
    pub fn loaded(&self, path: &Path, id: FileId) {
        let mut set = self.set();
        match set.loaded.iter_mut().find(|(p, _)| p == path) {
            Some(slot) if slot.1 == Some(id) => return,
            Some(slot) => slot.1 = Some(id),
            None => set.loaded.push((path.to_path_buf(), Some(id))),
        }
        set.generation += 1;
    }

    /// Their paths and identities as they are now, with the generation of
    /// the policy they came from, or why that cannot be told: then nothing
    /// is read. The lock is held only to copy the lists; the paths are
    /// resolved after it is let go, so a slow file system never holds up a
    /// reload.
    #[cfg(any(target_os = "linux", windows))]
    fn secrets(&self) -> Result<(Vec<Secret>, u64), String> {
        #[cfg(windows)]
        let resolve = win::protected;
        #[cfg(target_os = "linux")]
        let resolve = protected;
        self.secrets_with(resolve)
    }

    #[cfg(any(target_os = "linux", windows))]
    fn secrets_with(
        &self,
        resolve: impl FnOnce(&[PathBuf]) -> Result<Vec<Secret>, String>,
    ) -> Result<(Vec<Secret>, u64), String> {
        let (mut files, loaded, generation) = {
            let set = self.set();
            (set.paths.clone(), set.loaded.clone(), set.generation)
        };
        let mut ids = Vec::new();
        for (path, id) in loaded {
            let Some(id) = id else {
                return Err(format!(
                    "{} has no identity from when it was loaded, so no file is read",
                    path.display()
                ));
            };
            files.push(path);
            ids.push(id);
        }
        let mut now = resolve(&files)?;
        now.extend(ids.into_iter().map(|id| (PathBuf::new(), id)));
        Ok((now, generation))
    }

    /// Whether the policy is still the one of `generation`.
    #[cfg(any(target_os = "linux", windows))]
    fn is_current(&self, generation: u64) -> bool {
        self.set().generation == generation
    }
}

/// A file never read, as it really is, and its device and inode.
#[cfg(any(target_os = "linux", windows))]
type Secret = (PathBuf, (u64, u64));

/// The files that are never read, as they really are, with their identity
/// (device and inode), so a hard link to one is told too. A file that does
/// not exist is nothing to keep out; one that cannot be resolved otherwise
/// leaves nothing read.
#[cfg(target_os = "linux")]
fn protected(files: &[PathBuf]) -> Result<Vec<Secret>, String> {
    use std::os::unix::fs::MetadataExt;
    let mut out = Vec::new();
    for file in files {
        match fs::canonicalize(file).and_then(|real| Ok((fs::metadata(&real)?, real))) {
            Ok((meta, real)) => out.push((real, (meta.dev(), meta.ino()))),
            Err(e) if e.kind() == std::io::ErrorKind::NotFound => {}
            Err(e) => {
                return Err(format!(
                    "{} cannot be resolved ({e}), so no file is read",
                    file.display()
                ));
            }
        }
    }
    Ok(out)
}

#[cfg(target_os = "linux")]
fn read_linux(
    base: Option<&str>,
    path: &str,
    offset: u32,
    private: Option<&Path>,
    keep: &Protected,
) -> Result<String, String> {
    read_linux_with(base, path, offset, private, keep, || {})
}

/// The policy is linearized at the worker's last check: bytes are returned
/// only while the generation they were judged under is still the one in
/// force, checked before the file is read and again once its chunk is in
/// memory. A reload in between drops the chunk. That check is the cutoff, not
/// the server loop's enqueue of the reply: a reply already returned is not
/// recalled by a reload that lands after it. `after_read` runs between the read and that
/// last check, for tests.
#[cfg(target_os = "linux")]
fn read_linux_with(
    base: Option<&str>,
    path: &str,
    offset: u32,
    private: Option<&Path>,
    keep: &Protected,
    after_read: impl FnOnce(),
) -> Result<String, String> {
    use std::os::unix::fs::{MetadataExt, OpenOptionsExt};
    let file = locate(base, path)?;
    let root = private_root(private)?;
    let (secrets, generation) = keep.secrets()?;
    if root.as_ref().is_some_and(|root| file.starts_with(root)) {
        return Err("the server's own state directory is not read".into());
    }
    if secrets.iter().any(|(real, _)| *real == file) {
        return Err("a file the server keeps secret is not read".into());
    }
    let io = |e: std::io::Error| format!("{}: {e}", file.display());
    // Before opening, which a pipe or device would make wait; the opened
    // object is checked again below.
    if !fs::metadata(&file).map_err(io)?.is_file() {
        return Err(format!("{} is not a regular file", file.display()));
    }
    // Never waits on a pipe swapped in since the check above, and never
    // follows a link swapped in: the type is judged on what was opened.
    let f = fs::OpenOptions::new()
        .read(true)
        .custom_flags(rusty_libc::fd::O_NONBLOCK | rusty_libc::fd::O_NOFOLLOW)
        .open(&file)
        .map_err(io)?;
    verify(&f, &file, root.as_deref())?;
    let meta = f.metadata().map_err(io)?;
    // A hard link to a secret file is that file.
    if secrets
        .iter()
        .any(|(_, id)| *id == (meta.dev(), meta.ino()))
    {
        return Err("a file the server keeps secret is not read".into());
    }
    // The file was judged by a policy a reload may since have replaced:
    // nothing is read until it is judged again.
    if !keep.is_current(generation) {
        return Err("the server's access files changed during this download; try again".into());
    }
    if !meta.is_file() {
        return Err(format!("{} is not a regular file", file.display()));
    }
    chunk(f, &file, meta.len(), offset, keep, generation, after_read)
}

/// The reply for the chunk of `f` at `offset`, once the file was judged
/// under `generation`: dropped if a reload changed the policy since.
#[cfg(any(target_os = "linux", windows))]
fn chunk(
    mut f: File,
    file: &Path,
    size: u64,
    offset: u32,
    keep: &Protected,
    generation: u64,
    after_read: impl FnOnce(),
) -> Result<String, String> {
    let io = |e: std::io::Error| format!("{}: {e}", file.display());
    if size > FILE_MAX {
        return Err(format!("{} is over {FILE_MAX} bytes", file.display()));
    }
    f.seek(SeekFrom::Start(u64::from(offset))).map_err(io)?;
    // No more than the size it announces: a file that grew since its
    // metadata was read does not make the reply longer than it declared.
    let left = size.saturating_sub(u64::from(offset)).min(CHUNK as u64);
    let mut chunk = Vec::with_capacity(left as usize);
    f.take(left).read_to_end(&mut chunk).map_err(io)?;
    after_read();
    if !keep.is_current(generation) {
        return Err("the server's access files changed during this download; try again".into());
    }
    Ok(format!("{size}\n{}", rusty_base64::encode_standard(&chunk)))
}

#[cfg(all(test, not(any(target_os = "linux", windows))))]
mod refusal_tests {
    #[test]
    fn nothing_is_read_where_the_opened_file_cannot_be_checked() {
        let err = super::read(None, "f", 0, None, &super::Protected::default()).unwrap_err();
        assert!(err.contains("not supported"), "{err}");
    }
}

#[cfg(all(test, target_os = "linux"))]
mod tests {
    use super::*;
    use std::sync::atomic::{AtomicU32, Ordering};

    fn scratch() -> PathBuf {
        static N: AtomicU32 = AtomicU32::new(0);
        let dir = std::env::temp_dir().join(format!(
            "rusty-mux-download-{}-{}",
            std::process::id(),
            N.fetch_add(1, Ordering::Relaxed)
        ));
        fs::create_dir_all(&dir).unwrap();
        dir
    }

    fn body(reply: &str) -> (u64, Vec<u8>) {
        let (size, data) = reply.split_once('\n').unwrap();
        (
            size.parse().unwrap(),
            rusty_base64::decode_standard(data).unwrap(),
        )
    }

    #[test]
    fn a_file_is_read_in_chunks_from_an_offset() {
        let dir = scratch();
        let data: Vec<u8> = (0..CHUNK + 5).map(|i| i as u8).collect();
        fs::write(dir.join("f"), &data).unwrap();
        let base = dir.to_str();
        let (size, first) = body(&read(base, "f", 0, None, &Protected::default()).unwrap());
        assert_eq!((size, first.len()), (data.len() as u64, CHUNK));
        let (_, rest) = body(&read(base, "f", CHUNK as u32, None, &Protected::default()).unwrap());
        assert_eq!(rest, data[CHUNK..]);
        let (_, past) = body(
            &read(
                base,
                "f",
                data.len() as u32 + 9,
                None,
                &Protected::default(),
            )
            .unwrap(),
        );
        assert!(past.is_empty());
        fs::remove_dir_all(dir).unwrap();
    }

    #[test]
    fn what_is_not_a_plain_file_is_refused() {
        let dir = scratch();
        let base = dir.to_str();
        assert!(
            read(base, ".", 0, None, &Protected::default())
                .unwrap_err()
                .contains("not a regular file")
        );
        assert!(read(base, "missing", 0, None, &Protected::default()).is_err());
        assert!(read(None, "relative", 0, None, &Protected::default()).is_err());
        assert!(
            read(None, "\\\\host\\share\\f", 0, None, &Protected::default())
                .unwrap_err()
                .contains("network")
        );
        fs::remove_dir_all(dir).unwrap();
    }

    #[test]
    fn the_state_directory_is_not_read() {
        let state = scratch();
        fs::write(state.join("key"), b"secret").unwrap();
        let err = read(
            state.to_str(),
            "key",
            0,
            Some(&state),
            &Protected::default(),
        )
        .unwrap_err();
        assert!(err.contains("state directory"), "{err}");
        // A link out of a free folder into it is no way round.
        let free = scratch();
        #[cfg(unix)]
        {
            std::os::unix::fs::symlink(state.join("key"), free.join("l")).unwrap();
            assert!(read(free.to_str(), "l", 0, Some(&state), &Protected::default()).is_err());
        }
        fs::remove_dir_all(state).unwrap();
        fs::remove_dir_all(free).unwrap();
    }

    fn id_of(path: &Path) -> FileId {
        use std::os::unix::fs::MetadataExt;
        let meta = fs::metadata(path).unwrap();
        (meta.dev(), meta.ino())
    }

    fn refused(dir: &Path, name: &str, keep: &Protected) -> String {
        read(dir.to_str(), name, 0, None, keep).unwrap_err()
    }

    #[test]
    fn a_secret_file_outside_the_state_directory_is_not_read_even_by_a_hard_link() {
        let dir = scratch();
        let key = dir.join("tls.key");
        fs::write(&key, b"secret").unwrap();
        fs::hard_link(&key, dir.join("alias")).unwrap();
        let keep = Protected::default();
        keep.path(key.clone());
        keep.path(dir.join("not-yet-made"));
        for name in ["tls.key", "alias"] {
            assert!(
                refused(&dir, name, &keep).contains("keeps secret"),
                "{name}"
            );
        }
        fs::write(dir.join("plain"), b"ok").unwrap();
        assert!(read(dir.to_str(), "plain", 0, None, &keep).is_ok());
        fs::remove_dir_all(dir).unwrap();
    }

    #[test]
    fn a_rotated_secret_stays_out_until_a_reload_replaces_it() {
        let dir = scratch();
        let key = dir.join("allow");
        fs::write(&key, b"old").unwrap();
        fs::hard_link(&key, dir.join("old-copy")).unwrap();
        let keep = Protected::default();
        keep.require(key.clone());
        keep.loaded(&key, id_of(&key));
        // Rotation without a reload: the path names a new file; the loaded
        // object lives on as `old-copy` and stays out.
        fs::remove_file(&key).unwrap();
        fs::write(&key, b"new").unwrap();
        fs::hard_link(&key, dir.join("new-copy")).unwrap();
        for name in ["allow", "old-copy", "new-copy"] {
            assert!(
                refused(&dir, name, &keep).contains("keeps secret"),
                "{name}"
            );
        }
        // The reload loads the new file: it is what is in force now, and the
        // object it replaced is no longer secret.
        keep.loaded(&key, id_of(&key));
        for name in ["allow", "new-copy"] {
            assert!(
                refused(&dir, name, &keep).contains("keeps secret"),
                "{name}"
            );
        }
        assert!(read(dir.to_str(), "old-copy", 0, None, &keep).is_ok());
        fs::remove_dir_all(dir).unwrap();
    }

    #[test]
    fn resolving_paths_never_holds_the_lock_and_a_policy_change_is_seen() {
        let keep = Protected::default();
        keep.require(PathBuf::from("/allow"));
        keep.loaded(Path::new("/allow"), (1, 1));
        // A slow path lookup: the lock is free while it runs, and a reload
        // landing meanwhile moves the generation.
        let (_, generation) = keep
            .secrets_with(|_| {
                assert!(
                    keep.0.try_lock().is_ok(),
                    "the lock is held while resolving"
                );
                keep.loaded(Path::new("/allow"), (2, 2));
                Ok(Vec::new())
            })
            .unwrap();
        assert!(!keep.is_current(generation));
        let (_, now) = keep.secrets_with(|_| Ok(Vec::new())).unwrap();
        assert!(keep.is_current(now));
        // The same identity loaded again is no change.
        keep.loaded(Path::new("/allow"), (2, 2));
        assert!(keep.is_current(now));
    }

    #[test]
    fn a_reload_during_the_read_drops_the_bytes() {
        let dir = scratch();
        fs::write(dir.join("plain"), b"ok").unwrap();
        let keep = Protected::default();
        keep.require(dir.join("allow"));
        keep.loaded(&dir.join("allow"), (1, 1));
        let reload = || keep.loaded(&dir.join("allow"), (2, 2));
        let err = read_linux_with(dir.to_str(), "plain", 0, None, &keep, reload).unwrap_err();
        assert!(err.contains("changed during this download"), "{err}");
        // Judged again under the new policy, the same read goes through.
        assert!(read(dir.to_str(), "plain", 0, None, &keep).is_ok());
        fs::remove_dir_all(dir).unwrap();
    }

    #[test]
    fn nothing_is_read_while_a_required_identity_is_unknown() {
        let dir = scratch();
        fs::write(dir.join("plain"), b"ok").unwrap();
        let keep = Protected::default();
        keep.require(dir.join("allow"));
        let err = refused(&dir, "plain", &keep);
        assert!(err.contains("no identity"), "{err}");
        keep.loaded(&dir.join("allow"), (1, 1));
        assert!(read(dir.to_str(), "plain", 0, None, &keep).is_ok());
        fs::remove_dir_all(dir).unwrap();
    }

    #[test]
    fn a_named_pipe_is_refused_without_waiting() {
        use std::os::unix::fs::FileTypeExt;
        let dir = scratch();
        let fifo = dir.join("p");
        let made = std::process::Command::new("mkfifo").arg(&fifo).status();
        if made.is_ok_and(|s| s.success()) && fs::metadata(&fifo).unwrap().file_type().is_fifo() {
            let err = read(dir.to_str(), "p", 0, None, &Protected::default()).unwrap_err();
            assert!(err.contains("not a regular file"), "{err}");
        }
        fs::remove_dir_all(dir).unwrap();
    }

    #[test]
    fn no_file_is_read_when_the_state_directory_cannot_be_placed() {
        let dir = scratch();
        fs::write(dir.join("f"), b"x").unwrap();
        let gone = dir.join("no-such-state-dir");
        let err = read(dir.to_str(), "f", 0, Some(&gone), &Protected::default()).unwrap_err();
        assert!(err.contains("cannot be resolved"), "{err}");
        fs::remove_dir_all(dir).unwrap();
    }

    #[cfg(unix)]
    #[test]
    fn a_file_swapped_after_it_was_named_is_refused() {
        let dir = scratch();
        let file = fs::canonicalize(&dir).unwrap().join("f");
        fs::write(&file, b"first").unwrap();
        let opened = File::open(&file).unwrap();
        // Another file takes the name between the lookup and the open.
        fs::remove_file(&file).unwrap();
        fs::write(&file, b"second").unwrap();
        let err = verify(&opened, &file, None).unwrap_err();
        assert!(err.contains("changed while it was opened"), "{err}");
        fs::remove_dir_all(dir).unwrap();
    }

    #[cfg(target_os = "linux")]
    #[test]
    fn the_opened_object_is_placed_by_the_kernel_not_by_its_name() {
        let state = fs::canonicalize(scratch()).unwrap();
        let file = state.join("key");
        fs::write(&file, b"secret").unwrap();
        let opened = File::open(&file).unwrap();
        let err = verify(&opened, &file, Some(&state)).unwrap_err();
        assert!(err.contains("state directory"), "{err}");
        fs::remove_dir_all(state).unwrap();
    }
}
