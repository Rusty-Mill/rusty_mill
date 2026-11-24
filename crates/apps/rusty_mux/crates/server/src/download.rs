//! `terminal.download` (C83): a file of the server's host, read a chunk at a
//! time for a copy to the client.

use std::fs::{self, File};
use std::io::{Read, Seek, SeekFrom};
use std::path::{Path, PathBuf};

/// The most bytes one reply carries before base64.
pub(crate) const CHUNK: usize = 192 << 10;
/// The largest file read; an offset is 32 bits, and this is the upload's limit.
pub(crate) const FILE_MAX: u64 = 2 << 30;

/// Whether `path` names another machine (a UNC path), which opening would
/// make the server contact.
fn is_network_path(path: &str) -> bool {
    path.starts_with("\\\\") || (cfg!(windows) && path.starts_with("//"))
}

/// The file `path` names, taken relative to `base` (the block's working
/// directory), resolved; refused when it is not a regular file, is over
/// [`FILE_MAX`], or lies inside `private` (the server's state directory).
fn resolve(
    base: Option<&str>,
    path: &str,
    private: Option<&Path>,
) -> Result<(PathBuf, u64), String> {
    if is_network_path(path) {
        return Err("a network path is not read".into());
    }
    let file = match base {
        Some(base) => Path::new(base).join(path),
        None if Path::new(path).is_absolute() => path.into(),
        None => return Err("the program's working directory is unknown".into()),
    };
    let file = fs::canonicalize(&file).map_err(|e| format!("{}: {e}", file.display()))?;
    if private.is_some_and(|dir| fs::canonicalize(dir).is_ok_and(|d| file.starts_with(d))) {
        return Err("the server's own state directory is not read".into());
    }
    let meta = fs::metadata(&file).map_err(|e| format!("{}: {e}", file.display()))?;
    if !meta.is_file() {
        return Err(format!("{} is not a regular file", file.display()));
    }
    if meta.len() > FILE_MAX {
        return Err(format!("{} is over {FILE_MAX} bytes", file.display()));
    }
    Ok((file, meta.len()))
}

/// The reply for `path` from `offset`: its size, a newline, then the base64
/// of the next chunk (nothing past the end).
pub(crate) fn read(
    base: Option<&str>,
    path: &str,
    offset: u32,
    private: Option<&Path>,
) -> Result<String, String> {
    let (file, size) = resolve(base, path, private)?;
    let io = |e: std::io::Error| format!("{}: {e}", file.display());
    let mut f = File::open(&file).map_err(io)?;
    f.seek(SeekFrom::Start(u64::from(offset))).map_err(io)?;
    let mut chunk = Vec::with_capacity(CHUNK);
    f.take(CHUNK as u64).read_to_end(&mut chunk).map_err(io)?;
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
}
