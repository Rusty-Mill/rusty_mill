//! Disk side of the editor (pack D37, D38): list, read and rewrite one text
//! file inside an entry. The rules live in `ai_env_core::edit`; this module
//! only observes the disk and obeys them. Nothing here follows a link or
//! leaves the entry folder.

use crate::marker::{hex, walk};
use crate::{observe, Error};
use ai_env_core::{
    check_text, EditPlan, EditRefusal, Entry, Observed, Ownership, RelPath, Tool, MAX_EDIT_BYTES,
};
use rusty_sha2::{Hash, Sha256};
use std::fs;
use std::io;
use std::io::Write;
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicU64, Ordering};

/// A file's text and the hash of its bytes, for [`write_file`] to re-check.
#[derive(Debug, PartialEq, Eq)]
pub struct FileRead {
    pub text: String,
    pub hash: String,
}

/// Where `tool`'s copy of `entry` lives and who owns it, or `None` if it is
/// not there. A link to our source edits the source; a folder with our marker
/// is a marked copy; anything else is not ours (D6).
pub fn locate(entry: &Entry, tool: &Tool) -> io::Result<Option<(PathBuf, Ownership)>> {
    let to = tool.target(entry);
    Ok(match observe(entry, tool)? {
        Observed::Absent => None,
        Observed::Unmanaged => Some((to, Ownership::Unmanaged)),
        _ if fs::symlink_metadata(&to)?.file_type().is_symlink() => {
            Some((entry.source.clone(), Ownership::Source))
        }
        _ => Some((to, Ownership::MarkedCopy)),
    })
}

/// Regular files under `root` that the editor can name, sorted. The marker is
/// left out; a link inside `root` is an error, as for any entry walk.
pub fn list_files(root: &Path) -> io::Result<Vec<RelPath>> {
    Ok(walk(root)?
        .into_iter()
        .filter(|(_, is_dir)| !is_dir)
        .filter_map(|(rel, _)| RelPath::parse(&rel).ok())
        .collect())
}

/// The path of `rel` under `root`, if every part inside `root` is a real
/// directory and the last is a regular file.
fn regular_file(root: &Path, rel: &RelPath) -> Result<PathBuf, Error> {
    let parts: Vec<&str> = rel.as_str().split('/').collect();
    let mut cur = root.to_path_buf();
    for (i, part) in parts.iter().enumerate() {
        cur.push(part);
        let meta = fs::symlink_metadata(&cur).map_err(|e| Error::Io(cur.clone(), e))?;
        if meta.file_type().is_symlink() {
            return Err(Error::Link(cur));
        }
        let last = i + 1 == parts.len();
        if meta.is_file() != last {
            return Err(Error::NotFile(cur));
        }
    }
    Ok(cur)
}

/// Reads `rel` under `root` as editable text with its hash.
pub fn read_file(root: &Path, rel: &RelPath) -> Result<FileRead, Error> {
    if rel.is_marker() {
        return Err(EditRefusal::MarkerFile.into());
    }
    let path = regular_file(root, rel)?;
    let io_err = |e| Error::Io(path.clone(), e);
    let len = fs::metadata(&path).map_err(io_err)?.len();
    if len > MAX_EDIT_BYTES as u64 {
        return Err(Error::Edit(EditRefusal::TooLarge(
            usize::try_from(len).unwrap_or(usize::MAX),
        )));
    }
    let bytes = fs::read(&path).map_err(io_err)?;
    let text = check_text(&bytes)?.to_owned();
    let mut h = Sha256::new();
    h.update(&bytes);
    Ok(FileRead {
        text,
        hash: hex(h.finalize().as_ref()),
    })
}

/// Writes `plan` over its file under `root`, only if the file still has the
/// hash it was read with. The text is never readable by anyone who could not
/// read the original, and the owner and group are kept or the write is
/// refused; see [`replace`] for how, per platform.
pub fn write_file(root: &Path, plan: &EditPlan) -> Result<(), Error> {
    let now = read_file(root, plan.path())?;
    let path = plan.path().under(root);
    if now.hash != plan.base_hash() {
        return Err(Error::Changed(path));
    }
    replace(&path, plan.new_text().as_bytes()).map_err(|e| Error::Io(path, e))
}

/// Unix: stages the text in a sibling file that is owner-only until it has the
/// original's group and mode, then renames it over the original, so a failure
/// before the rename leaves the original. The file's owner and group must come
/// out the same as before, else nothing is published. A failure syncing the
/// folder afterwards is reported although the new text is already in place.
/// Not carried over: POSIX extended ACLs and other extended attributes.
#[cfg(unix)]
fn replace(path: &Path, bytes: &[u8]) -> io::Result<()> {
    let original = fs::metadata(path)?;
    let staged = stage(path, bytes, &original)?;
    fs::rename(&staged, path).inspect_err(|_| {
        let _ = fs::remove_file(&staged);
    })?;
    fs::File::open(
        path.parent()
            .filter(|d| !d.as_os_str().is_empty())
            .unwrap_or(Path::new(".")),
    )?
    .sync_all()
}

/// Windows: std cannot read or copy a file's ACL, and a new file would take
/// the folder's inherited one, which can be wider. So the file is rewritten in
/// place, which keeps its owner and ACL. It is not crash-atomic there: a crash
/// mid-write can leave a partly written file. An error (disk full, I/O) does
/// not: the old text is written back first (see [`rewrite_in_place`]).
#[cfg(not(unix))]
fn replace(path: &Path, bytes: &[u8]) -> io::Result<()> {
    rewrite_in_place(path, bytes, |f, b| f.write_all(b))
}

/// Overwrites `path` without truncating it first, so an error before the
/// first byte is written leaves it untouched. After an error part-way, the
/// original text is written back; if that also fails, the error says so.
#[cfg_attr(unix, allow(dead_code))]
fn rewrite_in_place(
    path: &Path,
    bytes: &[u8],
    write: impl Fn(&mut fs::File, &[u8]) -> io::Result<()>,
) -> io::Result<()> {
    use std::io::{Seek, SeekFrom};
    let original = fs::read(path)?;
    let mut file = fs::OpenOptions::new().write(true).open(path)?;
    let put = |file: &mut fs::File,
               data: &[u8],
               write: &dyn Fn(&mut fs::File, &[u8]) -> io::Result<()>| {
        file.seek(SeekFrom::Start(0))?;
        write(file, data)?;
        file.set_len(data.len() as u64)?;
        file.sync_all()
    };
    let Err(err) = put(&mut file, bytes, &write) else {
        return Ok(());
    };
    match put(&mut file, &original, &|f, b| f.write_all(b)) {
        Ok(()) => Err(err),
        Err(undo) => Err(io::Error::new(
            err.kind(),
            format!(
                "{err}; putting the old text back also failed ({undo}); the file may be damaged"
            ),
        )),
    }
}

/// Writes `bytes` to a new sibling of `path`: created owner-only, given the
/// `original`'s group (refusing if owner or group still differ), synced, and
/// only then given its mode. Removes it again if any step fails.
#[cfg(unix)]
fn stage(path: &Path, bytes: &[u8], original: &fs::Metadata) -> io::Result<PathBuf> {
    use std::os::unix::fs::{fchown, MetadataExt, OpenOptionsExt};
    static N: AtomicU64 = AtomicU64::new(0);
    let name = path.file_name().unwrap_or_default().to_string_lossy();
    let tmp = path.with_file_name(format!(
        ".{name}.aienv-{}-{}.tmp",
        std::process::id(),
        N.fetch_add(1, Ordering::Relaxed)
    ));
    let mut file = fs::OpenOptions::new()
        .write(true)
        .create_new(true)
        .mode(0o600)
        .open(&tmp)?;
    let done = (|| {
        fchown(&file, None, Some(original.gid()))?;
        let made = file.metadata()?;
        if (made.uid(), made.gid()) != (original.uid(), original.gid()) {
            return Err(io::Error::new(
                io::ErrorKind::PermissionDenied,
                "cannot keep the file's owner and group, so it is not replaced",
            ));
        }
        file.write_all(bytes)?;
        file.sync_all()?;
        file.set_permissions(original.permissions())
    })();
    drop(file);
    done.map(|()| tmp.clone()).inspect_err(|_| {
        let _ = fs::remove_file(&tmp);
    })
}

#[cfg(all(test, unix))]
pub(crate) fn stage_for_test(p: &Path, b: &[u8], m: &fs::Metadata) -> io::Result<PathBuf> {
    stage(p, b, m)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn file_with(text: &str) -> (PathBuf, impl Drop) {
        struct Cleanup(PathBuf);
        impl Drop for Cleanup {
            fn drop(&mut self) {
                let _ = fs::remove_file(&self.0);
            }
        }
        static N: AtomicU64 = AtomicU64::new(0);
        let n = N.fetch_add(1, Ordering::Relaxed);
        let p = std::env::temp_dir().join(format!("aienv-inplace-{}-{n}", std::process::id()));
        fs::write(&p, text).unwrap();
        (p.clone(), Cleanup(p))
    }

    #[test]
    fn in_place_rewrite_grows_and_shrinks_the_file() {
        let (p, _g) = file_with("hello world");
        rewrite_in_place(&p, b"hi", |f, b| f.write_all(b)).unwrap();
        assert_eq!(fs::read(&p).unwrap(), b"hi");
        rewrite_in_place(&p, b"a much longer text", |f, b| f.write_all(b)).unwrap();
        assert_eq!(fs::read(&p).unwrap(), b"a much longer text");
    }

    #[test]
    fn in_place_rewrite_puts_the_old_text_back_after_a_partial_write() {
        let (p, _g) = file_with("original text, kept");
        let half = |f: &mut fs::File, b: &[u8]| {
            f.write_all(&b[..b.len() / 2])?;
            Err(io::Error::other("disk full"))
        };
        let err = rewrite_in_place(&p, b"replacement that fails", half).unwrap_err();
        assert!(err.to_string().contains("disk full"));
        assert_eq!(fs::read(&p).unwrap(), b"original text, kept");
    }
}
