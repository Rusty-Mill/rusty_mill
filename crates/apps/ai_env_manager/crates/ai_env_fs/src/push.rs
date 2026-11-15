//! Disk side of pushing a marked copy's edits back to the source (pack D42):
//! which paths differ, and re-marking a copy once it equals the source.

use crate::marker::{read_marker, tree_hash, walk, MARKER};
use crate::Error;
use ai_env_core::Entry;
use std::collections::BTreeMap;
use std::fs;
use std::io;
use std::path::Path;

/// How a path differs between a source folder and its marked copy.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Delta {
    /// A file in both, with different bytes: the only kind a push can fix.
    Differs,
    OnlyInCopy,
    OnlyInSource,
    /// A file on one side and a folder on the other.
    KindDiffers,
}

/// One path that differs, `/`-separated and relative to both folders.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Difference {
    pub path: String,
    pub delta: Delta,
}

/// Paths that differ between `source` and `copy`, sorted. Folders count as
/// differing only when present on one side.
pub fn differences(source: &Path, copy: &Path) -> io::Result<Vec<Difference>> {
    let side = |root: &Path| -> io::Result<BTreeMap<String, bool>> {
        Ok(walk(root)?.into_iter().collect())
    };
    let (s, c) = (side(source)?, side(copy)?);
    let mut out = Vec::new();
    for path in s
        .keys()
        .chain(c.keys())
        .collect::<std::collections::BTreeSet<_>>()
    {
        let delta = match (s.get(path), c.get(path)) {
            (Some(_), None) => Delta::OnlyInSource,
            (None, Some(_)) => Delta::OnlyInCopy,
            (Some(a), Some(b)) if a != b => Delta::KindDiffers,
            (Some(false), Some(false))
                if fs::read(source.join(path))? != fs::read(copy.join(path))? =>
            {
                Delta::Differs
            }
            _ => continue,
        };
        out.push(Difference {
            path: path.clone(),
            delta,
        });
    }
    Ok(out)
}

/// Re-marks `tool`'s copy of `entry` as a copy of the current source, if (and
/// only if) the two trees are now identical; returns whether it did. A copy
/// that still differs keeps its old marker, so it still reads "edited" and
/// nothing replaces it (D20, D42). Refuses a folder without our marker.
pub fn remark_if_equal(entry: &Entry, copy: &Path) -> Result<bool, Error> {
    let io_err = |e| Error::Io(copy.to_owned(), e);
    if read_marker(copy).is_none() {
        return Err(Error::NotManaged(copy.to_owned()));
    }
    let hash = tree_hash(copy).map_err(io_err)?;
    if tree_hash(&entry.source).map_err(io_err)? != hash {
        return Ok(false);
    }
    let source = crate::marker::marker_source(&entry.source).map_err(io_err)?;
    let tmp = copy.join(format!("{MARKER}.tmp"));
    fs::write(&tmp, format!("source={source}\nsha256={hash}\n")).map_err(io_err)?;
    fs::rename(&tmp, copy.join(MARKER)).map_err(|e| {
        let _ = fs::remove_file(&tmp);
        io_err(e)
    })?;
    Ok(true)
}
