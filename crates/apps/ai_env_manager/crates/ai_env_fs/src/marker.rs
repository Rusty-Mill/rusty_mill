//! A marked copy: a folder holding our marker file, which records the source
//! path and a SHA-256 of the tree we wrote (pack D6, D20). The hash is what
//! tells an unmodified copy (safe to replace) from one edited in place.

use rusty_sha2::{Hash, Sha256};
use std::fs;
use std::io;
use std::path::{Path, PathBuf};

/// Marker file name, at the root of a managed copy.
pub const MARKER: &str = ".aienv-managed";

/// What the marker records.
#[derive(Debug, PartialEq, Eq)]
pub struct Marker {
    pub source: String,
    pub sha256: String,
}

fn invalid(msg: &str, p: &Path) -> io::Error {
    io::Error::new(
        io::ErrorKind::InvalidInput,
        format!("{msg}: {}", p.display()),
    )
}

/// Files and folders under `root` as (relative `/` path, is_dir), sorted. The
/// marker at the root is skipped. Symlinks are refused: following them could
/// escape the entry or loop.
fn walk(root: &Path) -> io::Result<Vec<(String, bool)>> {
    let mut out = Vec::new();
    let mut stack = vec![PathBuf::new()];
    while let Some(rel) = stack.pop() {
        for item in fs::read_dir(root.join(&rel))? {
            let item = item?;
            let name = item.file_name();
            if rel.as_os_str().is_empty() && name == MARKER {
                continue;
            }
            let rel_item = rel.join(&name);
            let kind = item.file_type()?;
            if kind.is_symlink() {
                return Err(invalid(
                    "symlink inside an entry is not supported",
                    &item.path(),
                ));
            }
            let text = rel_item.to_string_lossy().replace('\\', "/");
            out.push((text, kind.is_dir()));
            if kind.is_dir() {
                stack.push(rel_item);
            }
        }
    }
    out.sort();
    Ok(out)
}

fn hex(bytes: &[u8]) -> String {
    bytes.iter().map(|b| format!("{b:02x}")).collect()
}

/// SHA-256 over the tree's paths and file contents, independent of the order
/// the OS lists them in.
pub fn tree_hash(root: &Path) -> io::Result<String> {
    let mut h = Sha256::new();
    for (rel, is_dir) in walk(root)? {
        h.update(&(rel.len() as u64).to_le_bytes());
        h.update(rel.as_bytes());
        if is_dir {
            h.update(b"d");
            continue;
        }
        let data = fs::read(root.join(&rel))?;
        h.update(b"f");
        h.update(&(data.len() as u64).to_le_bytes());
        h.update(&data);
    }
    Ok(hex(h.finalize().as_ref()))
}

/// Reads the marker in `dir`; `None` if absent or not ours.
pub fn read_marker(dir: &Path) -> Option<Marker> {
    let text = fs::read_to_string(dir.join(MARKER)).ok()?;
    let mut source = None;
    let mut sha256 = None;
    for line in text.lines() {
        match line.split_once('=') {
            Some(("source", v)) => source = Some(v.to_owned()),
            Some(("sha256", v)) => sha256 = Some(v.to_owned()),
            _ => {}
        }
    }
    Some(Marker {
        source: source?,
        sha256: sha256?,
    })
}

/// The marker stores the source as one text line, so a path that cannot
/// round-trip through it is refused up front.
fn marker_source(from: &Path) -> io::Result<&str> {
    from.to_str()
        .filter(|s| !s.contains(['\n', '\r']))
        .ok_or_else(|| invalid("source path is not valid UTF-8 or has a line break", from))
}

/// Copies `from` to the new folder `to` and writes the marker last, so a
/// folder with a marker is always complete. The recorded hash is of what was
/// written to `to`, so a source that changes mid-copy reads as stale, not as an
/// edited copy. Cleans up after a failed copy and says so if it cannot.
pub fn copy_marked(from: &Path, to: &Path) -> io::Result<()> {
    let source = marker_source(from)?;
    let entries = walk(from)?;
    fs::create_dir(to)?;
    let copy = || -> io::Result<()> {
        for (rel, is_dir) in &entries {
            let (src, dst) = (from.join(rel), to.join(rel));
            if *is_dir {
                fs::create_dir_all(&dst)?;
            } else {
                fs::copy(&src, &dst)?;
            }
        }
        let hash = tree_hash(to)?;
        fs::write(to.join(MARKER), format!("source={source}\nsha256={hash}\n"))
    };
    copy().map_err(|e| match fs::remove_dir_all(to) {
        Ok(()) => e,
        Err(c) => io::Error::new(
            e.kind(),
            format!("{e}; cleaning up {} also failed: {c}", to.display()),
        ),
    })
}
