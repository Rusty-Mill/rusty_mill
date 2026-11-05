//! Agent sessions contained by a cgroup instead of a process group
//! (Let Em Loose D82, mux-hosted-agents-design.md §3; build defaults D86).
//!
//! A mux-hosted agent is forked by the mux, not by its holder, so it can't
//! join the holder's process group. It joins a cgroup that orch names
//! (`lel-jail` writes itself into it), and a cgroup can't be left with
//! `setsid` or `setpgid`. Three pieces replace the process-group ones:
//!
//! - [`open`] records the cgroup as `mod/<task>.cgroups/<key>` and only then
//!   creates it, so no crash leaves a cgroup without a record.
//! - [`hold`] is `bbp slot-hold --cgroup`: it keeps the slot until stdin is
//!   closed and the cgroup is empty, and at a deadline writes `cgroup.kill`.
//!   It never signals a process group.
//! - [`settle`] is what `bbp quiesce` does with each record: a populated
//!   cgroup is killed, an empty one removed with its record. A cgroup path
//!   names one session only, so killing it by path is safe even after its
//!   holder is gone.

use std::path::{Component, Path, PathBuf};
use std::time::{Duration, SystemTime};

use rusty_bbp::{Sha256, TaskId};

use crate::moderator::task_dir;

/// Every session cgroup lives below this.
pub const CGROUP_ROOT: &str = "/sys/fs/cgroup/";
/// How often the holder looks at `cgroup.events`.
const POLL: Duration = Duration::from_millis(100);

/// Where `task`'s session cgroups are recorded.
fn dir_of(dir: &Path, task: &TaskId) -> PathBuf {
    task_dir(dir, "mod", task).with_extension("cgroups")
}

/// A session cgroup path: absolute, below [`CGROUP_ROOT`], without `..`.
///
/// # Errors
/// When `path` is anything else.
pub fn checked(path: &Path) -> Result<PathBuf, String> {
    let root = Path::new(CGROUP_ROOT);
    let escapes = path.components().any(|c| c == Component::ParentDir);
    if !path.is_absolute() || escapes || !path.starts_with(root) || path == root {
        return Err(format!(
            "{} is not a cgroup below {CGROUP_ROOT}",
            path.display()
        ));
    }
    Ok(path.to_path_buf())
}

/// The record's file name: a digest of the path, so any path maps to one
/// file and two sessions never share one.
fn key(cgroup: &Path) -> String {
    Sha256::of(cgroup.as_os_str().as_encoded_bytes()).hex()
}

/// Record `cgroup` for `task`, then create it. A cgroup that already exists
/// is refused: each session gets a path no earlier one used.
///
/// # Errors
/// When the path is not a session cgroup, it exists, or the record or the
/// directory cannot be written.
pub fn open(dir: &Path, task: &TaskId, cgroup: &Path) -> Result<PathBuf, String> {
    let cgroup = checked(cgroup)?;
    if cgroup.exists() {
        return Err(format!("{} already exists", cgroup.display()));
    }
    let records = dir_of(dir, task);
    std::fs::create_dir_all(&records).map_err(|e| format!("{}: {e}", records.display()))?;
    let record = records.join(key(&cgroup));
    let text = cgroup.to_str().ok_or("non-UTF-8 cgroup path")?;
    rusty_atomic_file::write(&record, text.as_bytes())
        .map_err(|e| format!("{}: {e}", record.display()))?;
    std::fs::create_dir_all(&cgroup).map_err(|e| format!("{}: {e}", cgroup.display()))?;
    Ok(cgroup)
}

/// One recorded session cgroup.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Record {
    path: PathBuf,
    cgroup: PathBuf,
}

impl Record {
    /// The cgroup it records.
    #[must_use]
    pub fn cgroup(&self) -> &Path {
        &self.cgroup
    }
}

/// Every session cgroup recorded for `task`. Run with the moderator lock held.
///
/// # Errors
/// When a record cannot be read or names something that is not a session
/// cgroup.
pub fn records(dir: &Path, task: &TaskId) -> Result<Vec<Record>, String> {
    let records = dir_of(dir, task);
    let entries = match std::fs::read_dir(&records) {
        Ok(entries) => entries,
        Err(e) if e.kind() == std::io::ErrorKind::NotFound => return Ok(Vec::new()),
        Err(e) => return Err(format!("{}: {e}", records.display())),
    };
    entries
        .map(|entry| {
            let path = entry.map_err(|e| e.to_string())?.path();
            let text =
                std::fs::read_to_string(&path).map_err(|e| format!("{}: {e}", path.display()))?;
            let cgroup = checked(Path::new(text.trim()))
                .map_err(|e| format!("cgroup record {}: {e}", path.display()))?;
            Ok(Record { path, cgroup })
        })
        .collect()
}

/// Whether any process is in `cgroup` or below it. A cgroup that is gone is
/// empty.
///
/// # Errors
/// When `cgroup.events` can't be read or has no `populated` line.
pub fn populated(cgroup: &Path) -> Result<bool, String> {
    let events = cgroup.join("cgroup.events");
    let text = match std::fs::read_to_string(&events) {
        Ok(text) => text,
        Err(e) if e.kind() == std::io::ErrorKind::NotFound && !cgroup.exists() => return Ok(false),
        Err(e) => return Err(format!("{}: {e}", events.display())),
    };
    parse_populated(&text).ok_or_else(|| format!("{}: no populated line", events.display()))
}

/// The `populated` value of a `cgroup.events` file.
fn parse_populated(events: &str) -> Option<bool> {
    events.lines().find_map(|line| match line.split_once(' ')? {
        ("populated", "0") => Some(false),
        ("populated", "1") => Some(true),
        _ => None,
    })
}

/// SIGKILL everything in `cgroup` (`cgroup.kill`, Linux 5.14). A cgroup that
/// is gone has nothing to kill.
///
/// # Errors
/// When `cgroup.kill` exists but can't be written.
pub fn kill(cgroup: &Path) -> Result<(), String> {
    let file = cgroup.join("cgroup.kill");
    match std::fs::write(&file, "1") {
        Ok(()) => Ok(()),
        Err(e) if e.kind() == std::io::ErrorKind::NotFound && !cgroup.exists() => Ok(()),
        Err(e) => Err(format!("{}: {e}", file.display())),
    }
}

/// Settle one record: `true` once its cgroup is empty, removed and the
/// record gone. A populated cgroup is killed and is not quiet yet.
///
/// # Errors
/// When the cgroup can't be read, killed or removed, or the record removed.
pub fn settle(record: &Record) -> Result<bool, String> {
    if populated(&record.cgroup)? {
        kill(&record.cgroup)?;
        return Ok(false);
    }
    match std::fs::remove_dir(&record.cgroup) {
        Ok(()) => {}
        Err(e) if e.kind() == std::io::ErrorKind::NotFound => {}
        Err(e) => return Err(format!("{}: {e}", record.cgroup.display())),
    }
    std::fs::remove_file(&record.path).map_err(|e| format!("{}: {e}", record.path.display()))?;
    Ok(true)
}

/// `bbp slot-hold --cgroup`: return once `stdin` is at end of file and
/// `cgroup` is empty; the slot lock, the caller's stdout, stays open until
/// then. At `deadline` the cgroup is killed, and the hold ends as soon as it
/// is empty, whether or not stdin was closed.
///
/// # Errors
/// When the cgroup can't be read or killed.
pub fn hold(
    cgroup: &Path,
    deadline: Option<SystemTime>,
    stdin: impl std::io::Read + Send + 'static,
) -> Result<(), String> {
    let (closed, done) = std::sync::mpsc::channel();
    std::thread::spawn(move || {
        let mut stdin = stdin;
        // Nothing is written; end of file means the caller is done or gone.
        let _ = std::io::copy(&mut stdin, &mut std::io::sink());
        let _ = closed.send(());
    });
    let mut stdin_closed = false;
    let mut killed = false;
    loop {
        stdin_closed |= done.try_recv().is_ok();
        if !killed && deadline.is_some_and(|d| SystemTime::now() >= d) {
            kill(cgroup)?;
            killed = true;
        }
        if (stdin_closed || killed) && !populated(cgroup)? {
            return Ok(());
        }
        std::thread::sleep(POLL);
    }
}

#[cfg(test)]
#[allow(clippy::expect_used)]
mod tests {
    use super::*;

    #[test]
    fn only_paths_below_the_cgroup_root_are_session_cgroups() {
        assert!(checked(Path::new("/sys/fs/cgroup/lel/t/c/1")).is_ok());
        for bad in ["/sys/fs/cgroup/", "/sys/fs/cgroup/../x", "/tmp/cg", "lel/t"] {
            assert!(checked(Path::new(bad)).is_err(), "{bad}");
        }
    }

    #[test]
    fn populated_is_read_from_cgroup_events() {
        assert_eq!(parse_populated("populated 1\nfrozen 0\n"), Some(true));
        assert_eq!(parse_populated("frozen 0\npopulated 0\n"), Some(false));
        assert_eq!(parse_populated("frozen 0\n"), None);
    }

    #[test]
    fn a_missing_cgroup_is_empty_and_has_nothing_to_kill() {
        let gone = Path::new("/sys/fs/cgroup/lel-test-never-made-0");
        assert_eq!(populated(gone), Ok(false));
        assert_eq!(kill(gone), Ok(()));
    }

    #[test]
    fn records_are_keyed_by_path_and_refuse_a_bad_path() {
        let dir = std::env::temp_dir().join(format!("bbp-cgroups-{}", std::process::id()));
        let task = TaskId::from("t");
        assert_eq!(records(&dir, &task), Ok(Vec::new()));
        assert!(open(&dir, &task, Path::new("/tmp/not-a-cgroup")).is_err());
        let records_dir = dir_of(&dir, &task);
        std::fs::create_dir_all(&records_dir).expect("records dir");
        std::fs::write(records_dir.join("x"), "/etc").expect("record");
        let err = records(&dir, &task).expect_err("a record naming /etc");
        assert!(err.contains("not a cgroup"), "{err}");
        assert_ne!(
            key(Path::new("/sys/fs/cgroup/lel/a/1")),
            key(Path::new("/sys/fs/cgroup/lel/b/1"))
        );
        let _ = std::fs::remove_dir_all(&dir);
    }
}
