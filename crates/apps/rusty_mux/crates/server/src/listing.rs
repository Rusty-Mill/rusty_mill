//! `terminal.list-dir` (C08): one directory on the server's host, read for a
//! client's directory browse.

use std::fs;
use std::path::Path;
use std::sync::Arc;
use std::sync::atomic::{AtomicUsize, Ordering};
use std::time::Duration;

use crate::process::escape;

/// Entries listed at most; the rest are counted in the `+` line.
const LISTED_MAX: usize = 1000;
/// Entries read at most, so one huge directory cannot hold the server.
const READ_MAX: usize = 20_000;

/// What an entry is, as the one-letter kind a line starts with; `None` when
/// the file system will not say.
fn kind(path: &Path, entry: &fs::DirEntry) -> Option<char> {
    let link = entry.file_type().ok()?;
    if !link.is_symlink() {
        return Some(match (link.is_dir(), link.is_file()) {
            (true, _) => 'd',
            (_, true) => 'f',
            _ => 'o',
        });
    }
    // A link is told by what it points to; a broken one is `l`.
    Some(match fs::metadata(path) {
        Ok(m) if m.is_dir() => 'd',
        _ => 'l',
    })
}

/// Whether `path` names another machine (a UNC path), which opening would
/// make the server contact.
fn is_network_path(path: &str) -> bool {
    path.starts_with("\\\\") || (cfg!(windows) && path.starts_with("//"))
}

/// What lists a directory: [`list`], or in tests a stand-in that stalls.
pub(crate) type Lister =
    Arc<dyn Fn(Option<&str>, Option<&str>, Option<&str>) -> Result<String, String> + Send + Sync>;

/// The real [`Lister`].
pub(crate) fn lister() -> Lister {
    Arc::new(list)
}

/// How long one listing may take before it is refused.
pub(crate) const DEADLINE: Duration = Duration::from_secs(5);
/// Listings that may be running (or hung on a dead mount) at once.
const MAX_RUNNING: usize = 8;

/// Counts listings from the moment they start to the moment their thread
/// ends, which is later than their deadline when one hangs.
#[derive(Clone, Default)]
pub(crate) struct Gate(Arc<AtomicUsize>);

/// One listing's place under [`Gate::take`]; frees it on drop.
pub(crate) struct Slot(Arc<AtomicUsize>);

impl Gate {
    pub(crate) fn take(&self) -> Result<Slot, String> {
        let taken = self.0.fetch_add(1, Ordering::AcqRel);
        if taken >= MAX_RUNNING {
            self.0.fetch_sub(1, Ordering::AcqRel);
            return Err("too many directory listings are still running".into());
        }
        Ok(Slot(self.0.clone()))
    }
}

impl Drop for Slot {
    fn drop(&mut self) {
        self.0.fetch_sub(1, Ordering::AcqRel);
    }
}

/// The listing of `path`, taken relative to `base` (the block's working
/// directory) when it is not absolute, and `base` itself when `None`.
/// With a `root` (the workspace, D337) the resolved directory must lie under
/// it, links followed, or the listing is refused.
///
/// The first line is the directory's resolved path. Every other line is
/// `KIND<TAB>NAME`: `d` for a directory (a link to one too), `f` a file,
/// `l` another link, `o` anything else; directories first, then by name.
/// Then, each only when it applies: `+<TAB>N`, exactly N entries read were
/// left out; `><TAB>N`, the directory has more entries than are read, so at
/// least N were left out; `!<TAB>N`, N entries could not be read or have a name that is not UTF-8. Names
/// are [`escape`]d, so none adds a field or a line.
pub(crate) fn list(
    base: Option<&str>,
    path: Option<&str>,
    root: Option<&str>,
) -> Result<String, String> {
    list_with(base, path, root, READ_MAX, LISTED_MAX)
}

fn list_with(
    base: Option<&str>,
    path: Option<&str>,
    root: Option<&str>,
    read_max: usize,
    listed_max: usize,
) -> Result<String, String> {
    if path.is_some_and(is_network_path) {
        return Err("a network path is not listed".into());
    }
    let dir = match (path, base) {
        (Some(p), Some(b)) => Path::new(b).join(p),
        (Some(p), None) if Path::new(p).is_absolute() => p.into(),
        (Some(_), None) | (None, None) => {
            return Err("the program's working directory is unknown".into());
        }
        (None, Some(b)) => b.into(),
    };
    let dir = fs::canonicalize(&dir).map_err(|e| format!("{}: {e}", dir.display()))?;
    if let Some(root) = root {
        let root = fs::canonicalize(root).map_err(|e| format!("{root}: {e}"))?;
        if !dir.starts_with(&root) {
            return Err("the path is outside the workspace".into());
        }
    }
    let mut read = fs::read_dir(&dir).map_err(|e| format!("{}: {e}", dir.display()))?;
    let (mut entries, mut unread) = (Vec::new(), 0_usize);
    for item in read.by_ref().take(read_max) {
        match item {
            Ok(e) => match (kind(&e.path(), &e), e.file_name().into_string()) {
                (Some(k), Ok(name)) => entries.push((k, name)),
                // A name that is not UTF-8 cannot be sent or typed back.
                _ => unread += 1,
            },
            Err(_) => unread += 1,
        }
    }
    let more = read.next().is_some();
    entries.sort_by_cached_key(|(k, name)| (*k != 'd', name.to_lowercase(), name.clone()));
    let left = entries.len().saturating_sub(listed_max);
    entries.truncate(listed_max);
    let mut out = escape(&dir.to_string_lossy());
    for (k, name) in &entries {
        out.push_str(&format!("\n{k}\t{}", escape(name)));
    }
    if more {
        out.push_str(&format!("\n>\t{}", left + 1));
    } else if left > 0 {
        out.push_str(&format!("\n+\t{left}"));
    }
    if unread > 0 {
        out.push_str(&format!("\n!\t{unread}"));
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
        let text = list(Some(dir.to_str().unwrap()), None, None).unwrap();
        let lines: Vec<_> = text.lines().skip(1).collect();
        assert_eq!(lines, ["d\tAlpha", "d\tzeta", "f\tA.txt", "f\tb.txt"]);
        fs::remove_dir_all(dir).unwrap();
    }

    #[test]
    fn a_relative_path_is_taken_from_the_base() {
        let dir = scratch("rel");
        fs::create_dir_all(dir.join("sub")).unwrap();
        fs::write(dir.join("sub").join("a.txt"), "").unwrap();
        let text = list(Some(dir.to_str().unwrap()), Some("sub/../sub"), None).unwrap();
        let mut lines = text.lines();
        assert!(lines.next().unwrap().ends_with("sub"));
        assert_eq!(lines.collect::<Vec<_>>(), ["f\ta.txt"]);
        fs::remove_dir_all(dir).unwrap();
    }

    /// A tab cannot be in a Windows file name, so this one is Unix only.
    #[cfg(unix)]
    #[test]
    fn names_are_escaped() {
        let dir = scratch("esc");
        fs::write(dir.join("a\tb"), "").unwrap();
        let text = list(Some(dir.to_str().unwrap()), None, None).unwrap();
        assert_eq!(text.lines().last(), Some("f\ta\\tb"));
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
        let text = list(Some(dir.to_str().unwrap()), None, None).unwrap();
        let lines: Vec<_> = text.lines().skip(1).collect();
        assert_eq!(
            lines,
            ["d\treal", "d\tto-dir", "l\tbroken", "f\tfile", "l\tto-file"]
        );
        fs::remove_dir_all(dir).unwrap();
    }

    fn files(dir: &Path, n: usize) {
        for i in 0..n {
            fs::write(dir.join(format!("f{i:05}")), "").unwrap();
        }
    }

    #[test]
    fn what_is_left_out_is_counted() {
        let dir = scratch("many");
        files(&dir, 10);
        let text = list_with(Some(dir.to_str().unwrap()), None, None, 100, 7).unwrap();
        assert_eq!(text.lines().count(), 1 + 7 + 1);
        assert_eq!(text.lines().last(), Some("+\t3"));
        fs::remove_dir_all(dir).unwrap();
    }

    #[test]
    fn a_scan_that_stops_early_says_the_count_is_a_lower_bound() {
        let dir = scratch("scan");
        files(&dir, 11);
        let at = |max| list_with(Some(dir.to_str().unwrap()), None, None, max, 5).unwrap();
        // The directory holds exactly what is read: the count is exact.
        assert_eq!(at(11).lines().last(), Some("+\t6"));
        // One more than is read: at least 6 are out (5 read, 1 never read).
        assert_eq!(at(10).lines().last(), Some(">\t6"));
        assert_eq!(at(6).lines().last(), Some(">\t2"));
        fs::remove_dir_all(dir).unwrap();
    }

    #[test]
    fn the_gate_refuses_past_its_limit_until_a_listing_ends() {
        let gate = Gate::default();
        let held: Vec<_> = (0..MAX_RUNNING).map(|_| gate.take().unwrap()).collect();
        assert!(gate.take().is_err());
        drop(held);
        assert!(gate.take().is_ok());
    }

    #[test]
    fn a_workspace_listing_stays_under_its_root() {
        let dir = scratch("root");
        fs::create_dir_all(dir.join("ws").join("sub")).unwrap();
        fs::write(dir.join("outside.txt"), "").unwrap();
        let root = dir.join("ws");
        let root = root.to_str().unwrap();
        let ok = list(Some(root), Some("sub"), Some(root)).unwrap();
        assert!(ok.lines().next().unwrap().ends_with("sub"));
        for escape in ["..", "sub/../..", dir.to_str().unwrap()] {
            let reason = list(Some(root), Some(escape), Some(root)).unwrap_err();
            assert!(
                reason.contains("outside the workspace"),
                "{escape}: {reason}"
            );
        }
        fs::remove_dir_all(dir).unwrap();
    }

    #[cfg(unix)]
    #[test]
    fn a_link_out_of_the_workspace_is_refused() {
        let dir = scratch("rootlink");
        fs::create_dir_all(dir.join("ws")).unwrap();
        fs::create_dir(dir.join("elsewhere")).unwrap();
        std::os::unix::fs::symlink(dir.join("elsewhere"), dir.join("ws").join("out")).unwrap();
        let root = dir.join("ws");
        let root = root.to_str().unwrap();
        let reason = list(Some(root), Some("out"), Some(root)).unwrap_err();
        assert!(reason.contains("outside the workspace"), "{reason}");
        fs::remove_dir_all(dir).unwrap();
    }

    #[test]
    fn a_missing_directory_or_base_is_refused() {
        assert!(list(None, None, None).is_err());
        assert!(list(None, Some("rel"), None).is_err());
        assert!(list(Some("/"), Some("no/such/dir"), None).is_err());
    }
}

#[cfg(test)]
mod network_tests {
    use super::*;

    #[test]
    fn a_unc_path_is_refused() {
        for path in ["\\\\host\\share", "\\\\?\\C:\\", "\\\\.\\pipe"] {
            let reason = list(None, Some(path), None).unwrap_err();
            assert!(reason.contains("network"), "{path}: {reason}");
        }
    }
}
