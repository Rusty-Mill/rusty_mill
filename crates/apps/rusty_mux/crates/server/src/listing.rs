//! `terminal.list-dir` (C08): one directory on the server's host, read for a
//! client's directory browse.

use std::fs;
use std::path::Path;

use crate::process::escape;

/// Entries listed at most; the rest are counted in the `+` line.
const LISTED_MAX: usize = 1000;
/// Entries read at most, so one huge directory cannot hold the server.
const READ_MAX: usize = 20_000;

/// What an entry is, as the one-letter kind a line starts with.
fn kind(path: &Path, entry: &fs::DirEntry) -> char {
    let Ok(link) = entry.file_type() else {
        return 'o';
    };
    // A link is told by what it points to; a broken one stays `l`.
    let shown = if link.is_symlink() {
        fs::metadata(path).ok().map(|m| m.file_type())
    } else {
        Some(link)
    };
    match shown {
        Some(t) if t.is_dir() => 'd',
        Some(t) if t.is_file() => {
            if link.is_symlink() {
                'l'
            } else {
                'f'
            }
        }
        None => 'l',
        Some(_) => 'o',
    }
}

/// The listing of `path`, taken relative to `base` (the block's working
/// directory) when it is not absolute, and `base` itself when `None`.
///
/// The first line is the directory's resolved path. Every other line is
/// `KIND<TAB>NAME`: `d` for a directory (a link to one too), `f` a file,
/// `l` another link, `o` anything else; directories first, then by name.
/// A last `+<TAB>N` line says how many entries were left out. Names are
/// [`escape`]d, so none adds a field or a line.
pub(crate) fn list(base: Option<&str>, path: Option<&str>) -> Result<String, String> {
    let dir = match (path, base) {
        (Some(p), Some(b)) => Path::new(b).join(p),
        (Some(p), None) if Path::new(p).is_absolute() => p.into(),
        (Some(_), None) => return Err("the program's working directory is unknown".into()),
        (None, Some(b)) => b.into(),
        (None, None) => return Err("the program's working directory is unknown".into()),
    };
    let dir = fs::canonicalize(&dir).map_err(|e| format!("{}: {e}", dir.display()))?;
    let read = fs::read_dir(&dir).map_err(|e| format!("{}: {e}", dir.display()))?;
    let mut entries: Vec<(char, String)> = read
        .filter_map(Result::ok)
        .take(READ_MAX)
        .map(|e| {
            (
                kind(&e.path(), &e),
                e.file_name().to_string_lossy().into_owned(),
            )
        })
        .collect();
    entries.sort_by_cached_key(|(k, name)| (*k != 'd', name.to_lowercase(), name.clone()));
    let left = entries.len().saturating_sub(LISTED_MAX);
    entries.truncate(LISTED_MAX);
    let mut out = escape(&dir.to_string_lossy());
    for (k, name) in &entries {
        out.push_str(&format!("\n{k}\t{}", escape(name)));
    }
    if left > 0 {
        out.push_str(&format!("\n+\t{left}"));
    }
    Ok(out)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn scratch(name: &str) -> std::path::PathBuf {
        let dir =
            std::env::temp_dir().join(format!("rusty-mux-list-{name}-{}", std::process::id()));
        let _ = fs::remove_dir_all(&dir);
        fs::create_dir_all(&dir).unwrap();
        dir
    }

    #[test]
    fn directories_come_first_then_names_without_regard_to_case() {
        let dir = scratch("order");
        fs::create_dir(dir.join("zeta")).unwrap();
        fs::create_dir(dir.join("Alpha")).unwrap();
        fs::write(dir.join("b.txt"), "").unwrap();
        fs::write(dir.join("A.txt"), "").unwrap();
        let text = list(Some(dir.to_str().unwrap()), None).unwrap();
        let lines: Vec<_> = text.lines().skip(1).collect();
        assert_eq!(lines, ["d\tAlpha", "d\tzeta", "f\tA.txt", "f\tb.txt"]);
        fs::remove_dir_all(dir).unwrap();
    }

    #[test]
    fn a_relative_path_is_taken_from_the_base_and_names_are_escaped() {
        let dir = scratch("rel");
        fs::create_dir_all(dir.join("sub")).unwrap();
        fs::write(dir.join("sub").join("a\tb"), "").unwrap();
        let text = list(Some(dir.to_str().unwrap()), Some("sub/../sub")).unwrap();
        let mut lines = text.lines();
        assert!(lines.next().unwrap().ends_with("sub"));
        assert_eq!(lines.collect::<Vec<_>>(), ["f\ta\\tb"]);
        fs::remove_dir_all(dir).unwrap();
    }

    #[cfg(unix)]
    #[test]
    fn a_link_is_told_by_what_it_points_to() {
        let dir = scratch("link");
        fs::create_dir(dir.join("real")).unwrap();
        fs::write(dir.join("file"), "").unwrap();
        std::os::unix::fs::symlink("real", dir.join("to-dir")).unwrap();
        std::os::unix::fs::symlink("file", dir.join("to-file")).unwrap();
        std::os::unix::fs::symlink("nowhere", dir.join("broken")).unwrap();
        let text = list(Some(dir.to_str().unwrap()), None).unwrap();
        let lines: Vec<_> = text.lines().skip(1).collect();
        assert_eq!(
            lines,
            ["d\treal", "d\tto-dir", "l\tbroken", "f\tfile", "l\tto-file"]
        );
        fs::remove_dir_all(dir).unwrap();
    }

    #[test]
    fn what_is_left_out_is_counted() {
        let dir = scratch("many");
        for i in 0..LISTED_MAX + 3 {
            fs::write(dir.join(format!("f{i:05}")), "").unwrap();
        }
        let text = list(Some(dir.to_str().unwrap()), None).unwrap();
        assert_eq!(text.lines().count(), 1 + LISTED_MAX + 1);
        assert_eq!(text.lines().last(), Some("+\t3"));
        fs::remove_dir_all(dir).unwrap();
    }

    #[test]
    fn a_missing_directory_or_base_is_refused() {
        assert!(list(None, None).is_err());
        assert!(list(None, Some("rel")).is_err());
        assert!(list(Some("/"), Some("no/such/dir")).is_err());
    }
}
