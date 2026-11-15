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
use std::io::{self, Write};
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
/// hash it was read with. The new text is staged in a sibling file that is
/// private until it holds the file's own permissions, then renamed over it, so
/// the text is never readable by more users than could read the original. A
/// failure before the rename leaves the original; a failure syncing the folder
/// afterwards is reported although the new text is already in place.
pub fn write_file(root: &Path, plan: &EditPlan) -> Result<(), Error> {
    let now = read_file(root, plan.path())?;
    let path = plan.path().under(root);
    if now.hash != plan.base_hash() {
        return Err(Error::Changed(path));
    }
    let io_err = |e| Error::Io(path.clone(), e);
    let perms = fs::metadata(&path).map_err(io_err)?.permissions();
    let staged = stage(&path, plan.new_text().as_bytes(), perms).map_err(io_err)?;
    fs::rename(&staged, &path).map_err(|e| {
        let _ = fs::remove_file(&staged);
        io_err(e)
    })?;
    sync_dir(&path).map_err(io_err)
}

/// Writes `bytes` to a new file beside `path`, created owner-only, synced and
/// given `perms` last. Removes it again if any step fails.
fn stage(path: &Path, bytes: &[u8], perms: fs::Permissions) -> io::Result<PathBuf> {
    static N: AtomicU64 = AtomicU64::new(0);
    let name = path.file_name().unwrap_or_default().to_string_lossy();
    let tmp = path.with_file_name(format!(
        ".{name}.aienv-{}-{}.tmp",
        std::process::id(),
        N.fetch_add(1, Ordering::Relaxed)
    ));
    let mut open = fs::OpenOptions::new();
    open.write(true).create_new(true);
    #[cfg(unix)]
    std::os::unix::fs::OpenOptionsExt::mode(&mut open, 0o600);
    let mut file = open.open(&tmp)?;
    let done = file
        .write_all(bytes)
        .and_then(|()| file.sync_all())
        .and_then(|()| file.set_permissions(perms));
    drop(file);
    done.map(|()| tmp.clone()).inspect_err(|_| {
        let _ = fs::remove_file(&tmp);
    })
}

/// Makes the rename durable (Unix); Windows has no folder to sync.
#[cfg(unix)]
fn sync_dir(path: &Path) -> io::Result<()> {
    let dir = path.parent().filter(|d| !d.as_os_str().is_empty());
    fs::File::open(dir.unwrap_or(Path::new(".")))?.sync_all()
}

#[cfg(not(unix))]
#[allow(clippy::unnecessary_wraps)]
fn sync_dir(_: &Path) -> io::Result<()> {
    Ok(())
}

#[cfg(test)]
pub(crate) fn stage_for_test(p: &Path, b: &[u8], m: fs::Permissions) -> io::Result<PathBuf> {
    stage(p, b, m)
}
