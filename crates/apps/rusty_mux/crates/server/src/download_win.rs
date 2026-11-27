//! `terminal.download` on Windows: the opened handle is asked what it really
//! is (its final path and its volume and file index), not the name it was
//! opened by. The only `unsafe` in the download path lives here, in
//! [`handle_id`] and [`handle_path`].

use super::{FileId, Protected, Secret, chunk, locate, private_root};
use rusty_win32::fs::{FILE_ATTRIBUTE_REPARSE_POINT, final_path, stat_by_handle};
use std::fs::{self, File};
use std::os::windows::fs::OpenOptionsExt;
use std::os::windows::io::AsRawHandle;
use std::path::{Path, PathBuf};

/// `CreateFileW`'s flag to open a link itself instead of what it points to,
/// so a link swapped in after the name was resolved is seen, not followed.
const FILE_FLAG_OPEN_REPARSE_POINT: u32 = 0x0020_0000;

/// The volume and file index of the file `f` is, and its attributes.
fn handle_id(f: &File) -> Result<(FileId, u32), String> {
    // SAFETY: `f` owns a valid, open file handle for the whole call, and
    // `File` only ever holds a handle to a file, not a console or pipe
    // handle type this call cannot take.
    let info = unsafe { stat_by_handle(f.as_raw_handle()) }.map_err(|e| e.to_string())?;
    Ok((
        (u64::from(info.volume_serial_number), info.file_index),
        info.attributes,
    ))
}

/// The path the open handle `f` really names.
fn handle_path(f: &File) -> Result<PathBuf, String> {
    // SAFETY: `f` owns a valid, open file handle for the whole call.
    unsafe { final_path(f.as_raw_handle()) }
        .map(PathBuf::from)
        .map_err(|e| e.to_string())
}

/// `path` as comparable text: Windows names ignore case.
fn folded(path: &Path) -> String {
    path.to_string_lossy().to_lowercase()
}

/// Whether `path` is `root` or lies inside it.
fn within(path: &Path, root: &Path) -> bool {
    Path::new(&folded(path)).starts_with(folded(root))
}

/// The files that are never read, as they really are, with their identity.
/// A file that does not exist is nothing to keep out; one that cannot be
/// resolved otherwise leaves nothing read.
pub(super) fn protected(files: &[PathBuf]) -> Result<Vec<Secret>, String> {
    let mut out = Vec::new();
    for file in files {
        let opened = fs::canonicalize(file).and_then(|real| Ok((File::open(&real)?, real)));
        match opened {
            Ok((f, real)) => out.push((real, handle_id(&f)?.0)),
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

/// Opens `file` and checks the object opened: still the file the name
/// resolved to, a plain file (not a link), outside `root`. Its identity.
fn open_checked(file: &Path, root: Option<&Path>) -> Result<(File, FileId), String> {
    let io = |e: std::io::Error| format!("{}: {e}", file.display());
    let f = fs::OpenOptions::new()
        .read(true)
        .custom_flags(FILE_FLAG_OPEN_REPARSE_POINT)
        .open(file)
        .map_err(io)?;
    let (id, attributes) = handle_id(&f)?;
    if attributes & FILE_ATTRIBUTE_REPARSE_POINT != 0 {
        return Err(format!("{} changed while it was opened", file.display()));
    }
    let real = handle_path(&f)?;
    if folded(&real) != folded(file) {
        return Err(format!("{} changed while it was opened", file.display()));
    }
    if real.to_string_lossy().starts_with("\\\\?\\UNC\\") {
        return Err("a network path is not read".into());
    }
    if root.is_some_and(|root| within(&real, root)) {
        return Err("the server's own state directory is not read".into());
    }
    Ok((f, id))
}

/// [`super::read`] for Windows; judged as the Linux read is, on the file as
/// opened.
pub(super) fn read(
    base: Option<&str>,
    path: &str,
    offset: u32,
    private: Option<&Path>,
    keep: &Protected,
) -> Result<String, String> {
    let file = locate(base, path)?;
    let root = private_root(private)?;
    let (secrets, generation) = keep.secrets()?;
    if root.as_ref().is_some_and(|root| within(&file, root)) {
        return Err("the server's own state directory is not read".into());
    }
    let secret = || "a file the server keeps secret is not read".to_string();
    if secrets
        .iter()
        .any(|(real, _)| folded(real) == folded(&file))
    {
        return Err(secret());
    }
    // Before opening, which a device name would make wait.
    let meta = fs::metadata(&file).map_err(|e| format!("{}: {e}", file.display()))?;
    if !meta.is_file() {
        return Err(format!("{} is not a regular file", file.display()));
    }
    let (f, id) = open_checked(&file, root.as_deref())?;
    // A hard link to a secret file is that file.
    if secrets.iter().any(|(_, secret_id)| *secret_id == id) {
        return Err(secret());
    }
    if !keep.is_current(generation) {
        return Err("the server's access files changed during this download; try again".into());
    }
    let size = f.metadata().map_err(|e| e.to_string())?.len();
    chunk(f, &file, size, offset, keep, generation, || {})
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::sync::atomic::{AtomicU32, Ordering};

    fn scratch() -> PathBuf {
        static N: AtomicU32 = AtomicU32::new(0);
        let dir = std::env::temp_dir().join(format!(
            "rusty-mux-download-win-{}-{}",
            std::process::id(),
            N.fetch_add(1, Ordering::Relaxed)
        ));
        fs::create_dir_all(&dir).unwrap();
        dir
    }

    #[test]
    fn a_file_is_read_in_chunks_and_a_directory_is_refused() {
        let dir = scratch();
        fs::write(dir.join("f"), b"hello").unwrap();
        let keep = Protected::default();
        let reply = super::super::read(dir.to_str(), "f", 0, None, &keep).unwrap();
        let (size, data) = reply.split_once('\n').unwrap();
        assert_eq!(size, "5");
        assert_eq!(rusty_base64::decode_standard(data).unwrap(), b"hello");
        let err = super::super::read(dir.to_str(), ".", 0, None, &keep).unwrap_err();
        assert!(err.contains("not a regular file"), "{err}");
        fs::remove_dir_all(dir).unwrap();
    }

    #[test]
    fn the_state_directory_a_secret_and_its_hard_link_are_not_read() {
        let state = scratch();
        fs::write(state.join("key"), b"secret").unwrap();
        let err = super::super::read(
            state.to_str(),
            "key",
            0,
            Some(&state),
            &Protected::default(),
        )
        .unwrap_err();
        assert!(err.contains("state directory"), "{err}");

        let dir = scratch();
        let key = dir.join("tls.key");
        fs::write(&key, b"secret").unwrap();
        fs::hard_link(&key, dir.join("alias")).unwrap();
        let keep = Protected::default();
        keep.path(key);
        for name in ["tls.key", "alias"] {
            let err = super::super::read(dir.to_str(), name, 0, None, &keep).unwrap_err();
            assert!(err.contains("keeps secret"), "{name}: {err}");
        }
        fs::remove_dir_all(state).unwrap();
        fs::remove_dir_all(dir).unwrap();
    }

    #[test]
    fn a_network_path_is_refused() {
        let err = super::super::read(None, "\\\\host\\share\\f", 0, None, &Protected::default())
            .unwrap_err();
        assert!(err.contains("network"), "{err}");
    }

    #[test]
    fn a_name_is_judged_without_regard_to_case() {
        assert!(within(Path::new("C:\\State\\key"), Path::new("c:\\state")));
        assert!(!within(Path::new("C:\\Statement"), Path::new("c:\\state")));
    }
}
