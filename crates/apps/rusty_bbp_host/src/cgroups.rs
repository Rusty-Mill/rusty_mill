//! Agent sessions contained by a cgroup instead of a process group
//! (Let Em Loose D82, mux-hosted-agents-design.md §3; build defaults D86).
//!
//! A mux-hosted agent is forked by the mux, not by its holder, so it can't
//! join the holder's process group. It joins a cgroup that orch names
//! (`lel-jail` writes itself into it), and a cgroup can't be left with
//! `setsid` or `setpgid`. Three pieces replace the process-group ones:
//!
//! - [`open`] claims the path for one task (`mod/cgroup-owners/<key>`,
//!   written whole and renamed in under one lock), records the intent as `mod/<task>.cgroups/<key>`,
//!   creates the cgroup, also exclusively, and only then marks the record
//!   `created`. No crash leaves a cgroup this session made without a record,
//!   and no two sessions share a path. It runs under the task's cgroup lock
//!   and refuses an ended task.
//! - [`hold`] is `bbp slot-hold --cgroup`: it keeps the slot until stdin is
//!   closed and the cgroup is known to be empty, and from a deadline on
//!   writes `cgroup.kill`. An error is reported and retried, never taken
//!   for emptiness. It never signals a process group.
//! - [`settle`] is what `bbp quiesce` does with each record, under the same
//!   lock: a populated cgroup is killed, an empty one removed with its claim
//!   and record. Only a `created` record of the claim's own task touches the
//!   cgroup. A bare intent, left by a crash before the mark, can't tell a
//!   cgroup this session made from one that was already there, so it is
//!   dropped with its claim and the cgroup is left alone: fail closed.

use std::fs::File;
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

/// Which task owns each session cgroup path, one file per path.
fn owners(dir: &Path) -> PathBuf {
    dir.join("mod").join("cgroup-owners")
}

/// Take `task`'s cgroup lock, waiting for it: [`open`] holds it from its
/// liveness check through creating the cgroup, and `bbp quiesce` while it
/// settles the records, so neither sees the other half done.
///
/// # Errors
/// When the lock file can't be created or locked.
pub fn lock(dir: &Path, task: &TaskId) -> Result<File, String> {
    lock_file(&task_dir(dir, "mod", task).with_extension("cgroups.lock"))
}

/// The exclusive lock on `path`, waiting for it.
fn lock_file(path: &Path) -> Result<File, String> {
    if let Some(parent) = path.parent() {
        std::fs::create_dir_all(parent).map_err(|e| format!("{}: {e}", parent.display()))?;
    }
    let at = |e: std::io::Error| format!("{}: {e}", path.display());
    let file = File::options()
        .create(true)
        .truncate(false)
        .write(true)
        .open(path)
        .map_err(at)?;
    file.lock().map_err(at)?;
    Ok(file)
}

/// A session cgroup path: absolute, below [`CGROUP_ROOT`], without `..`,
/// and UTF-8 with no control character and no name that starts or ends in
/// whitespace, so the path recorded is the path read back, byte for byte.
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
    let text = path.to_str().ok_or("a cgroup path must be UTF-8")?;
    let odd_name = text
        .split('/')
        .any(|name| name.trim() != name || name.chars().any(char::is_control));
    if odd_name {
        return Err(format!(
            "{text:?}: cgroup names may not have control characters or start or end in whitespace"
        ));
    }
    Ok(path.to_path_buf())
}

/// The record's file name: a digest of the path, so any path maps to one
/// file and two sessions never share one.
fn key(cgroup: &Path) -> String {
    Sha256::of(cgroup.as_os_str().as_encoded_bytes()).hex()
}

/// Claim `cgroup` for `task`, record it, then create it. A path some
/// session already claimed, or a cgroup that already exists, is refused:
/// each session gets a path no earlier one used.
///
/// # Errors
/// When the path is not a session cgroup, the task has ended, the path is
/// claimed or exists, or the claim, record or directory can't be written.
pub fn open(dir: &Path, task: &TaskId, cgroup: &Path) -> Result<PathBuf, String> {
    open_with(dir, task, cgroup, || {})
}

/// [`open`], calling `after_record` once the record is written and before
/// the cgroup is created: the seam the interleaving tests pause in.
#[doc(hidden)]
pub fn open_with(
    dir: &Path,
    task: &TaskId,
    cgroup: &Path,
    after_record: impl FnOnce(),
) -> Result<PathBuf, String> {
    let cgroup = checked(cgroup)?;
    let _lock = lock(dir, task)?;
    if crate::open_driver(dir, task)?.state.state.terminal() {
        return Err(format!("task {} has ended", task.0));
    }
    let key = key(&cgroup);
    let owner = claim(dir, task, &cgroup, &key)?;
    let records = dir_of(dir, task);
    let record = records.join(&key);
    let written = std::fs::create_dir_all(&records)
        .map_err(|e| format!("{}: {e}", records.display()))
        .and_then(|()| {
            cgroup
                .to_str()
                .ok_or_else(|| "non-UTF-8 cgroup path".to_owned())
        })
        .and_then(|text| {
            rusty_atomic_file::write(&record, text.as_bytes())
                .map_err(|e| format!("{}: {e}", record.display()))
        });
    if let Err(e) = written {
        return Err(undo(e, None, &owner));
    }
    after_record();
    if let Err(e) = create(&cgroup) {
        return Err(undo(e, Some(&record), &owner));
    }
    // Only now is the cgroup known to be this session's own.
    let text = format!("{}\n{CREATED}\n", cgroup.display());
    if let Err(e) = rusty_atomic_file::write(&record, text.as_bytes()) {
        // Ours and empty: nothing has been told its path yet.
        let e = format!("{}: {e}", record.display());
        let e = match std::fs::remove_dir(&cgroup) {
            Ok(()) => e,
            Err(rm) => format!("{e} (and removing {}: {rm})", cgroup.display()),
        };
        return Err(undo(e, Some(&record), &owner));
    }
    Ok(cgroup)
}

/// The record's second line once [`open`] made the cgroup itself.
const CREATED: &str = "created";

/// The claim file for `cgroup`, created only if no task has claimed it, an
/// ancestor of it or a cgroup below it: `cgroup.kill` and `populated` reach
/// the whole subtree, so two sessions may never nest. Every task's claims
/// are made under one lock, so the check and the claim can't interleave
/// with another task's.
fn claim(dir: &Path, task: &TaskId, cgroup: &Path, key: &str) -> Result<PathBuf, String> {
    let owners = owners(dir);
    std::fs::create_dir_all(&owners).map_err(|e| format!("{}: {e}", owners.display()))?;
    let _all = lock_file(&owners.with_extension("lock"))?;
    for (path, by) in claims(&owners)? {
        if path == cgroup {
            return Err(format!(
                "{} already exists: task {by} owns it",
                cgroup.display()
            ));
        }
        if path.starts_with(cgroup) || cgroup.starts_with(&path) {
            return Err(format!(
                "{} overlaps {}, which task {by} owns",
                cgroup.display(),
                path.display()
            ));
        }
    }
    let draft = drafts(dir).join(key);
    let text = format!("{}\n{}", cgroup.display(), task.0);
    write_synced(&draft, &text).map_err(|e| undo(e, None, &draft))?;
    // The scan found no claim on this path, and claims are only made under
    // this lock, so nothing can be at `owner`: the rename publishes a
    // whole claim or none.
    let owner = owners.join(key);
    std::fs::rename(&draft, &owner)
        .map_err(|e| undo(format!("{}: {e}", owner.display()), None, &draft))?;
    Ok(owner)
}

/// Where a claim is written before it is published: outside the scanned
/// directory, so a crash mid-write leaves nothing a scan reads. A draft is
/// only touched under the owners lock, so one left behind is stale.
fn drafts(dir: &Path) -> PathBuf {
    dir.join("mod").join("cgroup-owners.drafts")
}

/// Writes `text` to `path`, replacing a stale file, and syncs it.
fn write_synced(path: &Path, text: &str) -> Result<(), String> {
    let at = |e: std::io::Error| format!("{}: {e}", path.display());
    if let Some(parent) = path.parent() {
        std::fs::create_dir_all(parent).map_err(|e| format!("{}: {e}", parent.display()))?;
    }
    let mut file = File::create(path).map_err(at)?;
    std::io::Write::write_all(&mut file, text.as_bytes())
        .and_then(|()| file.sync_all())
        .map_err(at)
}

/// Every claim: its path and the task that holds it. A claim file is the
/// path, a newline, then the task. Claims are dropped without the owners
/// lock (by `settle` and a failed open), so one that goes during the scan
/// is skipped: it no longer owns anything.
fn claims(owners: &Path) -> Result<Vec<(PathBuf, String)>, String> {
    let entries = std::fs::read_dir(owners).map_err(|e| format!("{}: {e}", owners.display()))?;
    let mut found = Vec::new();
    for entry in entries {
        let path = entry.map_err(|e| e.to_string())?.path();
        let text = match std::fs::read_to_string(&path) {
            Ok(text) => text,
            Err(e) if e.kind() == std::io::ErrorKind::NotFound => continue,
            Err(e) => return Err(format!("{}: {e}", path.display())),
        };
        let (cgroup, by) = text
            .split_once('\n')
            .ok_or_else(|| format!("cgroup claim {}: no task", path.display()))?;
        found.push((PathBuf::from(cgroup), by.to_owned()));
    }
    Ok(found)
}

/// Creates the cgroup itself exclusively: one that exists already, made
/// outside this protocol, is refused rather than shared.
fn create(cgroup: &Path) -> Result<(), String> {
    if let Some(parent) = cgroup.parent() {
        std::fs::create_dir_all(parent).map_err(|e| format!("{}: {e}", parent.display()))?;
    }
    std::fs::create_dir(cgroup).map_err(|e| format!("{}: {e}", cgroup.display()))
}

/// A failed open takes back its record, then its claim, and says so if it
/// can't: a record or claim left behind is still safe, as it names a path
/// only its own task can settle.
fn undo(error: String, record: Option<&Path>, owner: &Path) -> String {
    let left: Vec<String> = record
        .into_iter()
        .chain([owner])
        .filter_map(|p| {
            std::fs::remove_file(p)
                .err()
                .map(|e| format!("{}: {e}", p.display()))
        })
        .collect();
    if left.is_empty() {
        return error;
    }
    format!("{error} (and undoing it: {})", left.join("; "))
}

/// One recorded session cgroup.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Record {
    path: PathBuf,
    cgroup: PathBuf,
    /// Its claim file, and the task that must own it for this record to
    /// touch the cgroup.
    owner: PathBuf,
    task: String,
    /// Whether [`open`] got as far as creating the cgroup itself.
    created: bool,
}

impl Record {
    /// The cgroup it records.
    #[must_use]
    pub fn cgroup(&self) -> &Path {
        &self.cgroup
    }
}

/// Every session cgroup recorded for `task`. Run with [`lock`] held.
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
            let bad = |e: String| format!("cgroup record {}: {e}", path.display());
            let mut lines = text.split_terminator('\n');
            let cgroup = checked(Path::new(lines.next().unwrap_or_default())).map_err(bad)?;
            let created = match (lines.next(), lines.next()) {
                (None, None) => false,
                (Some(CREATED), None) => true,
                _ => return Err(bad("not a path and an optional `created`".into())),
            };
            let owner = owners(dir).join(key(&cgroup));
            Ok(Record {
                path,
                cgroup,
                owner,
                task: task.0.clone(),
                created,
            })
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

/// Settle one record, with [`lock`] held: `true` once its cgroup is empty
/// and removed, then its claim and the record. A populated cgroup is killed
/// and is not quiet yet. Only a `created` record whose task holds the claim
/// touches the cgroup. A bare intent drops its claim and leaves the cgroup
/// alone, as it can't show the cgroup is its own (an empty one it made just
/// before a crash is left behind). A record whose task doesn't hold the
/// claim is only dropped.
///
/// # Errors
/// When the claim or cgroup can't be read, killed or removed, or the record
/// removed.
pub fn settle(record: &Record) -> Result<bool, String> {
    let ours = owns(record)?;
    if ours && record.created {
        if populated(&record.cgroup)? {
            kill(&record.cgroup)?;
            return Ok(false);
        }
        remove_tree(&record.cgroup)?;
    }
    if ours {
        remove(&record.owner, |p| std::fs::remove_file(p))?;
    }
    remove(&record.path, |p| std::fs::remove_file(p))?;
    Ok(true)
}

/// Whether the record's task holds the claim on its cgroup.
fn owns(record: &Record) -> Result<bool, String> {
    match std::fs::read_to_string(&record.owner) {
        Ok(text) => Ok(text.split_once('\n')
            == Some((
                record.cgroup.to_str().unwrap_or_default(),
                record.task.as_str(),
            ))),
        Err(e) if e.kind() == std::io::ErrorKind::NotFound => Ok(false),
        Err(e) => Err(format!("{}: {e}", record.owner.display())),
    }
}

/// Removes an empty cgroup and the empty cgroups below it, deepest first:
/// a session may make its own children, which keep the root from going.
/// Only for an owned session, which no other session nests in or under.
fn remove_tree(cgroup: &Path) -> Result<(), String> {
    let entries = match std::fs::read_dir(cgroup) {
        Ok(entries) => entries,
        Err(e) if e.kind() == std::io::ErrorKind::NotFound => return Ok(()),
        Err(e) => return Err(format!("{}: {e}", cgroup.display())),
    };
    for entry in entries {
        let entry = entry.map_err(|e| format!("{}: {e}", cgroup.display()))?;
        let kind = entry
            .file_type()
            .map_err(|e| format!("{}: {e}", entry.path().display()))?;
        if kind.is_dir() {
            remove_tree(&entry.path())?;
        }
    }
    remove(cgroup, |p| std::fs::remove_dir(p))
}

/// `how(path)`, where a path that is already gone is done.
fn remove(path: &Path, how: impl Fn(&Path) -> std::io::Result<()>) -> Result<(), String> {
    match how(path) {
        Ok(()) => Ok(()),
        Err(e) if e.kind() == std::io::ErrorKind::NotFound => Ok(()),
        Err(e) => Err(format!("{}: {e}", path.display())),
    }
}

/// What the holder needs of a cgroup; a seam for failure injection.
pub trait Watched {
    /// Whether any process is in it.
    ///
    /// # Errors
    /// When that can't be established.
    fn populated(&self) -> Result<bool, String>;
    /// SIGKILL everything in it.
    ///
    /// # Errors
    /// When the kill can't be written.
    fn kill(&self) -> Result<(), String>;
}

impl Watched for Path {
    fn populated(&self) -> Result<bool, String> {
        populated(self)
    }

    fn kill(&self) -> Result<(), String> {
        kill(self)
    }
}

/// `bbp slot-hold --cgroup`: return once `stdin` is at end of file and
/// `cgroup` is known to be empty; the slot lock, the caller's stdout, stays
/// open until then. From `deadline` on the cgroup is killed at every look
/// that doesn't find it empty, and the hold ends as soon as it is, whether
/// or not stdin was closed. An error is passed to `report` (once until it
/// changes) and the look retried: uncertainty never frees the slot.
pub fn hold<C: Watched + ?Sized>(
    cgroup: &C,
    deadline: Option<SystemTime>,
    stdin: impl std::io::Read + Send + 'static,
    report: &mut dyn FnMut(&str),
) {
    let (closed, done) = std::sync::mpsc::channel();
    std::thread::spawn(move || {
        let mut stdin = stdin;
        // Nothing is written; end of file means the caller is done or gone.
        let _ = std::io::copy(&mut stdin, &mut std::io::sink());
        let _ = closed.send(());
    });
    let mut stdin_closed = false;
    let mut last: Option<String> = None;
    let mut tell = |e: String| {
        if last.as_ref() != Some(&e) {
            report(&e);
            last = Some(e);
        }
    };
    loop {
        stdin_closed |= done.try_recv().is_ok();
        let past = deadline.is_some_and(|d| SystemTime::now() >= d);
        if stdin_closed || past {
            match cgroup.populated() {
                Ok(false) => return,
                Ok(true) => {}
                Err(e) => tell(e),
            }
            if past {
                if let Err(e) = cgroup.kill() {
                    tell(e);
                }
            }
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
        assert!(checked(Path::new("/sys/fs/cgroup/lel/my session")).is_ok());
        for bad in [
            "/sys/fs/cgroup/",
            "/sys/fs/cgroup/../x",
            "/tmp/cg",
            "lel/t",
            "/sys/fs/cgroup/bbp-test/session ",
            "/sys/fs/cgroup/ lel/s",
            "/sys/fs/cgroup/lel/s\r",
            "/sys/fs/cgroup/lel/a\nb",
        ] {
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

    use std::sync::atomic::{AtomicBool, AtomicUsize, Ordering};
    use std::sync::Arc;

    /// A cgroup whose reads fail until `empty` is set, then report it empty;
    /// `kill` fails its first `kill_fails` calls.
    #[derive(Default)]
    struct Flaky {
        empty: AtomicBool,
        looks: AtomicUsize,
        kills: AtomicUsize,
        kill_fails: usize,
        reads_fail: bool,
    }

    impl Watched for Flaky {
        fn populated(&self) -> Result<bool, String> {
            self.looks.fetch_add(1, Ordering::SeqCst);
            if self.empty.load(Ordering::SeqCst) {
                return Ok(false);
            }
            if self.reads_fail {
                return Err("cgroup.events: injected read failure".into());
            }
            Ok(true)
        }

        fn kill(&self) -> Result<(), String> {
            let n = self.kills.fetch_add(1, Ordering::SeqCst);
            if n < self.kill_fails {
                return Err("cgroup.kill: injected denial".into());
            }
            Ok(())
        }
    }

    /// stdin that never reaches end of file: the caller keeps it open.
    struct Open(std::sync::mpsc::Receiver<()>);

    impl std::io::Read for Open {
        fn read(&mut self, _: &mut [u8]) -> std::io::Result<usize> {
            let _ = self.0.recv();
            Ok(0)
        }
    }

    /// A slot lock taken here, and whether some other open of it is refused.
    fn slot(tag: &str) -> (PathBuf, File) {
        let path = std::env::temp_dir().join(format!("bbp-cg-slot-{}-{tag}", std::process::id()));
        let file = File::options()
            .create(true)
            .truncate(false)
            .write(true)
            .open(&path)
            .expect("slot");
        file.lock().expect("lock");
        (path, file)
    }

    fn held(path: &Path) -> bool {
        let file = File::options().write(true).open(path).expect("slot");
        matches!(file.try_lock(), Err(std::fs::TryLockError::WouldBlock))
    }

    fn wait_for(what: &str, done: impl Fn() -> bool) {
        let end = std::time::Instant::now() + Duration::from_secs(10);
        while !done() {
            assert!(
                std::time::Instant::now() < end,
                "timed out waiting for {what}"
            );
            std::thread::sleep(Duration::from_millis(10));
        }
    }

    /// Runs `hold` on a thread that owns the slot lock, as `bbp slot-hold`
    /// owns it on its stdout; the agent keeps no copy. Returns the reports.
    fn holding(
        cg: Arc<Flaky>,
        deadline: Option<SystemTime>,
        stdin: impl std::io::Read + Send + 'static,
        lock: File,
    ) -> std::thread::JoinHandle<Vec<String>> {
        std::thread::spawn(move || {
            let mut reports = Vec::new();
            hold(&*cg, deadline, stdin, &mut |e| reports.push(e.to_owned()));
            drop(lock);
            reports
        })
    }

    #[test]
    fn a_failed_look_keeps_the_slot_until_the_cgroup_is_known_empty() {
        let (path, lock) = slot("look");
        let cg = Arc::new(Flaky {
            reads_fail: true,
            ..Flaky::default()
        });
        let holder = holding(cg.clone(), None, std::io::empty(), lock);
        wait_for("several failed looks", || {
            cg.looks.load(Ordering::SeqCst) >= 5
        });
        assert!(!holder.is_finished(), "the hold ended on an error");
        assert!(held(&path), "the slot was freed on an error");
        cg.empty.store(true, Ordering::SeqCst);
        let reports = holder.join().expect("holder");
        // A parallel test's fork may hold a copy of the lock until it execs.
        wait_for("the slot to be freed", || !held(&path));
        assert_eq!(
            reports,
            ["cgroup.events: injected read failure"],
            "reported once"
        );
        assert_eq!(cg.kills.load(Ordering::SeqCst), 0, "no deadline, no kill");
        let _ = std::fs::remove_file(&path);
    }

    #[test]
    fn a_failed_kill_is_retried_and_the_slot_kept_while_members_remain() {
        let (path, lock) = slot("kill");
        let cg = Arc::new(Flaky {
            kill_fails: 3,
            ..Flaky::default()
        });
        // The caller still holds stdin open; only the deadline applies.
        let (_caller, open) = std::sync::mpsc::channel();
        let past = SystemTime::now() - Duration::from_secs(1);
        let holder = holding(cg.clone(), Some(past), Open(open), lock);
        wait_for("the kill to succeed", || {
            cg.kills.load(Ordering::SeqCst) >= 5
        });
        assert!(!holder.is_finished(), "the hold ended with members left");
        assert!(held(&path), "the slot was freed with members left");
        cg.empty.store(true, Ordering::SeqCst);
        let reports = holder.join().expect("holder");
        // A parallel test's fork may hold a copy of the lock until it execs.
        wait_for("the slot to be freed", || !held(&path));
        assert_eq!(reports, ["cgroup.kill: injected denial"]);
        let _ = std::fs::remove_file(&path);
    }

    #[test]
    fn a_record_whose_task_does_not_own_the_path_never_touches_the_cgroup() {
        let base = std::env::temp_dir().join(format!("bbp-cg-own-{}", std::process::id()));
        let cgroup = base.join("cg");
        std::fs::create_dir_all(&cgroup).expect("cg");
        std::fs::write(cgroup.join("cgroup.events"), "populated 1\n").expect("events");
        let owner = base.join("owner");
        std::fs::write(&owner, format!("{}\nA", cgroup.display())).expect("owner");
        let path = base.join("record");
        std::fs::write(&path, "x").expect("record");
        let record = Record {
            path: path.clone(),
            cgroup: cgroup.clone(),
            owner: owner.clone(),
            task: "B".into(),
            created: true,
        };
        assert_eq!(settle(&record), Ok(true));
        assert!(!path.exists(), "the stray record is dropped");
        assert!(
            !cgroup.join("cgroup.kill").exists(),
            "A's session was not killed"
        );
        assert!(
            cgroup.is_dir() && owner.exists(),
            "A's cgroup and claim stay"
        );
        let _ = std::fs::remove_dir_all(&base);
    }

    #[test]
    fn an_intent_never_touches_the_cgroup_even_with_its_own_claim() {
        let base = std::env::temp_dir().join(format!("bbp-cg-intent-{}", std::process::id()));
        let cgroup = base.join("cg");
        std::fs::create_dir_all(&cgroup).expect("cg");
        std::fs::write(cgroup.join("cgroup.events"), "populated 1\n").expect("events");
        let owner = base.join("owner");
        std::fs::write(&owner, format!("{}\nA", cgroup.display())).expect("owner");
        let path = base.join("record");
        std::fs::write(&path, "x").expect("record");
        let record = Record {
            path: path.clone(),
            cgroup: cgroup.clone(),
            owner: owner.clone(),
            task: "A".into(),
            created: false,
        };
        assert_eq!(settle(&record), Ok(true));
        assert!(
            !path.exists() && !owner.exists(),
            "intent and claim are dropped"
        );
        assert!(
            !cgroup.join("cgroup.kill").exists(),
            "the cgroup was not killed"
        );
        assert!(cgroup.is_dir(), "nor removed");
        let _ = std::fs::remove_dir_all(&base);
    }

    #[test]
    fn a_record_is_a_path_and_an_optional_created_mark() {
        let dir = std::env::temp_dir().join(format!("bbp-cg-rec-{}", std::process::id()));
        let task = TaskId::from("t");
        let records_dir = dir_of(&dir, &task);
        std::fs::create_dir_all(&records_dir).expect("records dir");
        std::fs::write(records_dir.join("a"), "/sys/fs/cgroup/lel/a\n").expect("a");
        std::fs::write(records_dir.join("b"), "/sys/fs/cgroup/lel/b\ncreated\n").expect("b");
        let mut got: Vec<(PathBuf, bool)> = records(&dir, &task)
            .expect("records")
            .into_iter()
            .map(|r| (r.cgroup, r.created))
            .collect();
        got.sort();
        assert_eq!(
            got,
            [
                (PathBuf::from("/sys/fs/cgroup/lel/a"), false),
                (PathBuf::from("/sys/fs/cgroup/lel/b"), true)
            ]
        );
        std::fs::write(records_dir.join("c"), "/sys/fs/cgroup/lel/c\nmaybe\n").expect("c");
        assert!(records(&dir, &task).is_err());
        let _ = std::fs::remove_dir_all(&dir);
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

    #[test]
    fn a_claim_is_published_whole_and_a_stale_draft_blocks_nothing() {
        let dir = std::env::temp_dir().join(format!("bbp-cg-draft-{}", std::process::id()));
        let cgroup = Path::new("/sys/fs/cgroup/lel/draft/1");
        let key = key(cgroup);
        // A crash mid-claim leaves a partial draft, never a partial claim.
        std::fs::create_dir_all(drafts(&dir)).expect("drafts");
        std::fs::write(drafts(&dir).join(&key), "/sys/fs/cgroup/lel/dr").expect("draft");
        std::fs::write(drafts(&dir).join("other"), "").expect("draft");
        let other = Path::new("/sys/fs/cgroup/lel/draft/2");
        claim(&dir, &TaskId::from("a"), other, &super::key(other)).expect("unrelated claim");
        let owner = claim(&dir, &TaskId::from("b"), cgroup, &key).expect("claim over a draft");
        assert_eq!(
            std::fs::read_to_string(&owner).expect("claim"),
            format!("{}\nb", cgroup.display())
        );
        assert!(!drafts(&dir).join(&key).exists(), "the draft was renamed");
        assert_eq!(claims(&owners(&dir)).expect("claims").len(), 2);
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn a_claim_dropped_during_a_scan_is_skipped() {
        let dir = std::env::temp_dir().join(format!("bbp-cg-gone-{}", std::process::id()));
        let owners = owners(&dir);
        std::fs::create_dir_all(&owners).expect("owners");
        std::fs::write(owners.join("kept"), "/sys/fs/cgroup/lel/k\nA").expect("kept");
        // Listed but unreadable as NotFound: what a claim removed by
        // another task's settle looks like between read_dir and the read.
        std::os::unix::fs::symlink(owners.join("absent"), owners.join("gone")).expect("link");
        assert_eq!(
            claims(&owners),
            Ok(vec![(
                PathBuf::from("/sys/fs/cgroup/lel/k"),
                "A".to_owned()
            )])
        );
        let _ = std::fs::remove_dir_all(&dir);
    }
}
