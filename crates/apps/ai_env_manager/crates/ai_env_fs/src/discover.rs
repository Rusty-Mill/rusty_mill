//! Finds skill entries in a folder: each subfolder holding a `SKILL.md` is one
//! entry, named by its folder (pack D5: linked in place, nothing copied).

use ai_env_core::{Entry, Kind};
use std::fs;
use std::io;
use std::path::{Path, PathBuf};

/// A folder that looked like a skill but could not become an entry.
#[derive(Debug, PartialEq, Eq)]
pub struct Skipped {
    pub path: PathBuf,
    pub reason: String,
}

/// What a scan found: usable entries, sorted by id, and what was passed over.
#[derive(Debug, Default, PartialEq, Eq)]
pub struct Discovered {
    pub entries: Vec<Entry>,
    pub skipped: Vec<Skipped>,
}

/// Scans `dir`. The source path of each entry is absolute (a relative link
/// target would break once the link is read from another folder). Folders
/// without a `SKILL.md` are ignored; a skill folder whose name is not a valid
/// id is reported in `skipped`, never silently dropped.
pub fn discover_skills(dir: &Path) -> io::Result<Discovered> {
    let root = std::path::absolute(dir)?;
    let mut found = Discovered::default();
    for item in fs::read_dir(&root)? {
        let path = item?.path();
        if !path.is_dir() || !path.join("SKILL.md").is_file() {
            continue;
        }
        let name = path
            .file_name()
            .and_then(|n| n.to_str())
            .unwrap_or_default();
        match name.parse() {
            Ok(id) => found.entries.push(Entry {
                id,
                kind: Kind::Skill,
                source: path,
            }),
            Err(e) => found.skipped.push(Skipped {
                path,
                reason: format!("{e}"),
            }),
        }
    }
    found.entries.sort_by(|a, b| a.id.cmp(&b.id));
    found.skipped.sort_by(|a, b| a.path.cmp(&b.path));
    Ok(found)
}
