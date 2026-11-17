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

/// Deepest folder level searched below the chosen folder (reported when hit).
const MAX_DEPTH: usize = 8;

/// Folders never searched: tool and build output, never skill packs.
const IGNORED: [&str; 3] = ["node_modules", "target", ".git"];

/// Scans `dir` and everything below it (hidden folders, `node_modules`,
/// `target` and anything deeper than eight levels are not searched; the last
/// is reported in `skipped`). A folder holding a `SKILL.md` is one
/// entry and is not searched further (a skill's own subfolders are its
/// content, not more skills). Symlinked folders are not followed, so a link
/// cycle cannot loop. The source path of each entry is absolute (a relative
/// link target would break once the link is read from another folder). A skill
/// folder whose name is not a valid id is reported in `skipped`, never
/// silently dropped. An unreadable `dir` is an error; an unreadable folder
/// below it is reported in `skipped`; so is a folder with one unreadable
/// entry (the whole folder is passed over, not just that entry).
pub fn discover_skills(dir: &Path) -> io::Result<Discovered> {
    let root = std::path::absolute(dir)?;
    let mut found = Discovered::default();
    // Fail on the chosen folder itself; below it, report and carry on.
    let mut pending: Vec<(PathBuf, usize)> =
        read_subdirs(&root)?.into_iter().map(|p| (p, 1)).collect();
    while let Some((path, depth)) = pending.pop() {
        if path.join("SKILL.md").is_file() {
            classify(path, &mut found);
            continue;
        }
        if depth >= MAX_DEPTH {
            found.skipped.push(Skipped {
                path,
                reason: format!("not searched: deeper than {MAX_DEPTH} levels"),
            });
            continue;
        }
        match read_subdirs(&path) {
            Ok(subs) => pending.extend(subs.into_iter().map(|p| (p, depth + 1))),
            Err(e) => found.skipped.push(Skipped {
                path,
                reason: format!("cannot read folder: {e}"),
            }),
        }
    }
    found.entries.sort_by(|a, b| a.id.cmp(&b.id));
    found.skipped.sort_by(|a, b| a.path.cmp(&b.path));
    Ok(found)
}

/// Real (not symlinked) subfolders of `dir`, minus hidden and ignored ones.
fn read_subdirs(dir: &Path) -> io::Result<Vec<PathBuf>> {
    let mut subs = Vec::new();
    for item in fs::read_dir(dir)? {
        let item = item?;
        let name = item.file_name();
        let name = name.to_string_lossy();
        if item.file_type()?.is_dir() && !name.starts_with('.') && !IGNORED.contains(&&*name) {
            subs.push(item.path());
        }
    }
    Ok(subs)
}

fn classify(path: PathBuf, found: &mut Discovered) {
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
