//! `terminal.download` (C83): a file of the server's host, read a chunk at a
//! time for a copy to the client.

use std::fs::{self, File};
use std::io::{Read, Seek, SeekFrom};
use std::path::{Path, PathBuf};

/// The most bytes one reply carries before base64.
pub(crate) const CHUNK: usize = 192 << 10;
/// The largest file read; an offset is 32 bits, and this is the upload's limit.
pub(crate) const FILE_MAX: u64 = 2 << 30;

/// Whether `path` names another machine (a UNC path, or what a Windows
/// path resolves to on one), which opening would make the server contact.
fn is_network_path(path: &str) -> bool {
    path.starts_with("\\\\") || (cfg!(windows) && path.starts_with("//"))
}

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
) -> Result<String, String> {
    let file = locate(base, path)?;
    let root = private_root(private)?;
    // Where the opened object's own path cannot be read, a file inside the
    // state directory could not be told from outside it: nothing is read.
    #[cfg(not(target_os = "linux"))]
    if root.is_some() {
        return Err("this platform cannot check an opened file against the state directory".into());
    }
    if root.as_ref().is_some_and(|root| file.starts_with(root)) {
        return Err("the server's own state directory is not read".into());
    }
    let io = |e: std::io::Error| format!("{}: {e}", file.display());
    // Before opening, which a pipe or device would make wait; the opened
    // object is checked again below.
    if !fs::metadata(&file).map_err(io)?.is_file() {
        return Err(format!("{} is not a regular file", file.display()));
    }
    let mut f = File::open(&file).map_err(io)?;
    verify(&f, &file, root.as_deref())?;
    let meta = f.metadata().map_err(io)?;
    if !meta.is_file() {
        return Err(format!("{} is not a regular file", file.display()));
    }
    let size = meta.len();
    if size > FILE_MAX {
        return Err(format!("{} is over {FILE_MAX} bytes", file.display()));
    }
    f.seek(SeekFrom::Start(u64::from(offset))).map_err(io)?;
    // No more than the size it announces: a file that grew since its
    // metadata was read does not make the reply longer than it declared.
    let left = size.saturating_sub(u64::from(offset)).min(CHUNK as u64);
    let mut chunk = Vec::with_capacity(left as usize);
    f.take(left).read_to_end(&mut chunk).map_err(io)?;
    Ok(format!("{size}\n{}", rusty_base64::encode_standard(&chunk)))
}

#[cfg(test)]
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
        let (size, first) = body(&read(base, "f", 0, None).unwrap());
        assert_eq!((size, first.len()), (data.len() as u64, CHUNK));
        let (_, rest) = body(&read(base, "f", CHUNK as u32, None).unwrap());
        assert_eq!(rest, data[CHUNK..]);
        let (_, past) = body(&read(base, "f", data.len() as u32 + 9, None).unwrap());
        assert!(past.is_empty());
        fs::remove_dir_all(dir).unwrap();
    }

    #[test]
    fn what_is_not_a_plain_file_is_refused() {
        let dir = scratch();
        let base = dir.to_str();
        assert!(
            read(base, ".", 0, None)
                .unwrap_err()
                .contains("not a regular file")
        );
        assert!(read(base, "missing", 0, None).is_err());
        assert!(read(None, "relative", 0, None).is_err());
        assert!(
            read(None, "\\\\host\\share\\f", 0, None)
                .unwrap_err()
                .contains("network")
        );
        fs::remove_dir_all(dir).unwrap();
    }

    #[test]
    fn the_state_directory_is_not_read() {
        let state = scratch();
        fs::write(state.join("key"), b"secret").unwrap();
        let err = read(state.to_str(), "key", 0, Some(&state)).unwrap_err();
        assert!(err.contains("state directory"), "{err}");
        // A link out of a free folder into it is no way round.
        let free = scratch();
        #[cfg(unix)]
        {
            std::os::unix::fs::symlink(state.join("key"), free.join("l")).unwrap();
            assert!(read(free.to_str(), "l", 0, Some(&state)).is_err());
        }
        fs::remove_dir_all(state).unwrap();
        fs::remove_dir_all(free).unwrap();
    }

    #[test]
    fn no_file_is_read_when_the_state_directory_cannot_be_placed() {
        let dir = scratch();
        fs::write(dir.join("f"), b"x").unwrap();
        let gone = dir.join("no-such-state-dir");
        let err = read(dir.to_str(), "f", 0, Some(&gone)).unwrap_err();
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
