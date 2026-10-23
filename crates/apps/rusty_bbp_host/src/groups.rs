//! The harness groups a task's moderators started, recorded so that their
//! cleanup can be established without the moderator that started them.
//!
//! Every harness runs in the process group of a `bbp slot-hold` holder
//! ([`crate::slots::holder`]). Before anyone joins that group the moderator
//! records it as `mod/<task>.groups/<holder pid>`, holding the holder's start
//! time. A record is removed only once its group is seen empty, so a group
//! never outlives its record.
//!
//! The group is the cleanup boundary, as it is for the moderator's own kill:
//! a process that leaves it with `setsid` or `setpgid` is outside it.
//!
//! A group id is the holder's pid, and a pid is reused once its process and
//! group are gone. So no member is signalled by number. A member is pinned by
//! a pidfd and checked to be in the group while the holder is proven alive,
//! which reserves the group id for this group alone; a signal then reaches
//! that member or, if it has exited since, nobody. With the holder gone the
//! group's members can no longer be told from a later group with the same
//! id: they are left alone, and the group is not quiet until none remains.
//!
//! The records directory doubles as the task's cleanup capability: the first
//! moderator creates it before any harness has run. A task whose harnesses
//! ran without it (under an older `bbp`) cannot have its cleanup established.

use std::path::{Path, PathBuf};
#[cfg(target_os = "linux")]
use std::time::{Duration, Instant};

use rusty_bbp::TaskId;

use crate::moderator::task_dir;

/// Where `task`'s harness groups are recorded.
fn dir_of(dir: &Path, task: &TaskId) -> PathBuf {
    task_dir(dir, "mod", task).with_extension("groups")
}

/// Whether a moderator ever started a harness for `task`: each one gets a
/// turn log, under every `bbp` version. An entry that cannot be read is an
/// error, never "no harness ran".
fn launched_any(dir: &Path, task: &TaskId) -> Result<bool, String> {
    let agents = task_dir(dir, "agents", task);
    let entries = match std::fs::read_dir(&agents) {
        Ok(entries) => entries,
        Err(e) if e.kind() == std::io::ErrorKind::NotFound => return Ok(false),
        Err(e) => return Err(format!("{}: {e}", agents.display())),
    };
    for entry in entries {
        let entry = entry.map_err(|e| format!("{}: {e}", agents.display()))?;
        if entry.file_name().to_string_lossy().starts_with("turn-") {
            return Ok(true);
        }
    }
    Ok(false)
}

/// Make `task`'s groups recordable, once, under the moderator lock. `false`
/// when harnesses already ran unrecorded: the task stays without the
/// capability, so [`records`] refuses it rather than call it quiet. Only
/// Linux records groups.
///
/// # Errors
/// When the task's directories cannot be read or created.
pub fn enable(dir: &Path, task: &TaskId) -> Result<bool, String> {
    if !cfg!(target_os = "linux") {
        return Ok(false);
    }
    let groups = dir_of(dir, task);
    if groups.is_dir() {
        return Ok(true);
    }
    if launched_any(dir, task)? {
        eprintln!(
            "bbp mod: task {} ran harnesses before groups were recorded; it cannot be quiesced",
            task.0
        );
        return Ok(false);
    }
    std::fs::create_dir_all(&groups).map_err(|e| format!("{}: {e}", groups.display()))?;
    Ok(true)
}

/// One recorded group: its holder's pid, which is the group id, and the
/// holder's start time.
#[derive(Debug, Clone, PartialEq, Eq)]
#[cfg_attr(not(target_os = "linux"), allow(dead_code))]
pub struct Record {
    path: PathBuf,
    holder: u32,
    start: u64,
}

/// Record the group `holder` leads, before anyone joins it.
///
/// # Errors
/// When the holder has already exited or the record cannot be written.
pub fn record(dir: &Path, task: &TaskId, holder: u32) -> Result<PathBuf, String> {
    let start = crate::slots::process_start(holder).ok_or("slot holder exited at start")?;
    let path = dir_of(dir, task).join(holder.to_string());
    rusty_atomic_file::write(&path, start.to_string().as_bytes())
        .map_err(|e| format!("{}: {e}", path.display()))?;
    Ok(path)
}

/// Every group recorded for `task`. Run with the moderator lock held.
///
/// # Errors
/// When the task's harnesses ran unrecorded, so no record can prove them
/// gone, or a record cannot be read.
pub fn records(dir: &Path, task: &TaskId) -> Result<Vec<Record>, String> {
    let groups = dir_of(dir, task);
    let entries = match std::fs::read_dir(&groups) {
        Ok(entries) => entries,
        Err(e) if e.kind() == std::io::ErrorKind::NotFound => {
            if launched_any(dir, task)? {
                return Err(format!(
                    "task {} ran harnesses before their groups were recorded; \
                     their cleanup cannot be established",
                    task.0
                ));
            }
            return Ok(Vec::new());
        }
        Err(e) => return Err(format!("{}: {e}", groups.display())),
    };
    entries
        .map(|entry| {
            let path = entry.map_err(|e| e.to_string())?.path();
            let holder = path
                .file_name()
                .and_then(|n| n.to_str())
                .and_then(|n| n.parse().ok())
                .ok_or_else(|| format!("unexpected group record {}", path.display()))?;
            let start = std::fs::read_to_string(&path)
                .map_err(|e| format!("{}: {e}", path.display()))?
                .trim()
                .parse()
                .map_err(|_| format!("unreadable group record {}", path.display()))?;
            Ok(Record {
                path,
                holder,
                start,
            })
        })
        .collect()
}

/// Remove a record whose group is gone.
///
/// # Errors
/// When the file cannot be removed.
pub fn forget(record: &Path) -> Result<(), String> {
    std::fs::remove_file(record).map_err(|e| format!("{}: {e}", record.display()))
}

/// Settle one recorded group: `true` once it is empty and its record gone.
/// While its holder lives every member is killed; once the holder is gone
/// and members remain, nothing is signalled and the group is not quiet.
///
/// # Errors
/// When `/proc` or a pidfd fails, or the record cannot be removed.
#[cfg(target_os = "linux")]
pub fn settle(record: &Record) -> Result<bool, String> {
    match inspect(record.holder, record.start)? {
        Seen::Owned(group) => {
            group.kill()?;
            Ok(false)
        }
        Seen::Unowned => Ok(false),
        Seen::Empty => forget(&record.path).map(|()| true),
    }
}

/// Wait up to `within` for group `id` to have no live member but its leader,
/// which the caller keeps unreaped so the id cannot be reused meanwhile.
#[cfg(target_os = "linux")]
pub fn drained(id: u32, within: Duration) -> bool {
    let deadline = Instant::now() + within;
    loop {
        if empty(id) {
            return true;
        }
        if Instant::now() >= deadline {
            return false;
        }
        std::thread::sleep(Duration::from_millis(20));
    }
}

#[cfg(not(target_os = "linux"))]
pub fn drained(_id: u32, _within: std::time::Duration) -> bool {
    true
}

/// What [`inspect`] saw of a recorded group.
#[cfg(target_os = "linux")]
enum Seen {
    /// The holder lives: its members, pinned.
    Owned(Owned),
    /// The holder is gone and processes with the group's id remain, which
    /// may be its members or a later group's.
    Unowned,
    /// The holder is gone and no process has the group's id.
    Empty,
}

/// A group whose holder was alive when each of its members was pinned.
#[cfg(target_os = "linux")]
struct Owned {
    holder: Pidfd,
    members: Vec<Pidfd>,
}

#[cfg(target_os = "linux")]
impl Owned {
    /// `SIGKILL` every pinned member, provided the holder is still alive:
    /// then each was in this group when pinned, and its pidfd reaches it or,
    /// if it has exited since, nobody. `false` when the holder has exited,
    /// and nothing was signalled.
    fn kill(&self) -> Result<bool, String> {
        if self.holder.exited()? {
            return Ok(false);
        }
        for member in &self.members {
            member.kill()?;
        }
        Ok(true)
    }
}

/// Pin group `id`, recorded with its holder's `start`. The holder is pinned
/// first and checked to be the recorded process; each member is pinned, then
/// checked to be in the group ([`Owned::kill`] checks the holder outlived
/// that check).
#[cfg(target_os = "linux")]
fn inspect(id: u32, start: u64) -> Result<Seen, String> {
    let holder = Pidfd::open(id)?.filter(|_| crate::slots::process_start(id) == Some(start));
    let Some(holder) = holder else {
        return unowned(Path::new(PROC), id);
    };
    let mut members = Vec::new();
    for pid in members_of(Path::new(PROC), id)? {
        let Some(member) = Pidfd::open(pid)? else {
            continue;
        };
        if in_group(Path::new(PROC), pid, id)? {
            members.push(member);
        }
    }
    Ok(Seen::Owned(Owned { holder, members }))
}

#[cfg(target_os = "linux")]
const PROC: &str = "/proc";

/// What is left of group `id` once its holder is gone, as `proc` lists it:
/// [`Seen::Empty`] only after two looks find no member (a scan can miss a
/// member that forked as its parent exited, as the holder's own does).
#[cfg(target_os = "linux")]
fn unowned(proc: &Path, id: u32) -> Result<Seen, String> {
    if !members_of(proc, id)?.is_empty() {
        return Ok(Seen::Unowned);
    }
    std::thread::sleep(Duration::from_millis(100));
    Ok(if members_of(proc, id)?.is_empty() {
        Seen::Empty
    } else {
        Seen::Unowned
    })
}

/// Whether no live process but the leader has group id `id`; a scan that
/// fails counts as no.
#[cfg(target_os = "linux")]
fn empty(id: u32) -> bool {
    members_of(Path::new(PROC), id).is_ok_and(|m| m.is_empty())
}

/// The live processes in group `id` other than its leader, as `proc` lists
/// them. Any process that cannot be read, short of having exited, is an
/// error: never absent on a guess.
#[cfg(target_os = "linux")]
fn members_of(proc: &Path, id: u32) -> Result<Vec<u32>, String> {
    let entries = std::fs::read_dir(proc).map_err(|e| format!("{}: {e}", proc.display()))?;
    let mut members = Vec::new();
    for entry in entries {
        let entry = entry.map_err(|e| format!("{}: {e}", proc.display()))?;
        let Some(pid) = entry
            .file_name()
            .to_str()
            .and_then(|n| n.parse::<u32>().ok())
        else {
            continue;
        };
        if pid != id && in_group(proc, pid, id)? {
            members.push(pid);
        }
    }
    Ok(members)
}

/// Whether `pid` is a live (not zombie) process in group `id`. A process
/// that exited while being read is not; any other failure to read or parse
/// its stat is an error.
#[cfg(target_os = "linux")]
fn in_group(proc: &Path, pid: u32, id: u32) -> Result<bool, String> {
    let path = proc.join(pid.to_string()).join("stat");
    let stat = match std::fs::read_to_string(&path) {
        Ok(stat) => stat,
        Err(e) if exited(&e) => return Ok(false),
        Err(e) => return Err(format!("{}: {e}", path.display())),
    };
    let malformed = || format!("{}: malformed stat", path.display());
    let (_, rest) = stat.rsplit_once(") ").ok_or_else(malformed)?;
    let state = rest.chars().next().ok_or_else(malformed)?;
    let group = crate::slots::stat_pgrp(&stat).ok_or_else(malformed)?;
    Ok(state != 'Z' && group == id)
}

/// Whether a `/proc` read failed because the process is gone: its
/// directory went (`ENOENT`) or it exited mid-read (`ESRCH`).
#[cfg(target_os = "linux")]
fn exited(e: &std::io::Error) -> bool {
    e.kind() == std::io::ErrorKind::NotFound || e.raw_os_error() == Some(3)
}

/// A pidfd: one process, never a later one with its pid.
#[cfg(target_os = "linux")]
struct Pidfd(i32);

#[cfg(target_os = "linux")]
impl Pidfd {
    /// `None` when no process has `pid`.
    fn open(pid: u32) -> Result<Option<Pidfd>, String> {
        let raw = i32::try_from(pid).map_err(|_| "pid out of range".to_owned())?;
        match rusty_libc::process::pidfd_open(raw, 0) {
            Ok(fd) => Ok(Some(Pidfd(fd))),
            Err(rusty_libc::Errno::ESRCH) => Ok(None),
            Err(e) => Err(format!("pidfd_open({pid}): {e:?}")),
        }
    }

    /// Whether the process has exited: its pidfd is readable.
    fn exited(&self) -> Result<bool, String> {
        use rusty_libc::fd::{poll, PollFd, POLLIN};
        let mut fds = [PollFd {
            fd: self.0,
            events: POLLIN,
            revents: 0,
        }];
        poll(&mut fds, 0).map_err(|e| format!("poll pidfd: {e:?}"))?;
        Ok(fds[0].revents & POLLIN != 0)
    }

    /// `SIGKILL` the process; one already gone is fine.
    fn kill(&self) -> Result<(), String> {
        match rusty_libc::process::pidfd_send_signal(self.0, rusty_libc::signal::SIGKILL) {
            Ok(()) | Err(rusty_libc::Errno::ESRCH) => Ok(()),
            Err(e) => Err(format!("pidfd_send_signal: {e:?}")),
        }
    }
}

#[cfg(target_os = "linux")]
impl Drop for Pidfd {
    fn drop(&mut self) {
        let _ = rusty_libc::fd::close(self.0);
    }
}

#[cfg(all(test, target_os = "linux"))]
#[allow(clippy::expect_used, clippy::panic)]
mod tests {
    use super::*;
    use std::os::unix::process::CommandExt;
    use std::process::{Child, Command};

    /// A process that leads a new group, like a holder.
    fn leader() -> Child {
        Command::new("sleep")
            .arg("60")
            .process_group(0)
            .spawn()
            .expect("spawn leader")
    }

    /// A process in `group`, like a harness.
    fn member(group: &Child) -> Child {
        Command::new("sleep")
            .arg("60")
            .process_group(i32::try_from(group.id()).expect("pid"))
            .spawn()
            .expect("spawn member")
    }

    fn alive(child: &mut Child) -> bool {
        child.try_wait().expect("try_wait").is_none()
    }

    fn end(child: &mut Child) {
        let _ = child.kill();
        let _ = child.wait();
    }

    fn start(child: &Child) -> u64 {
        crate::slots::process_start(child.id()).expect("start")
    }

    fn owned(seen: Seen) -> Owned {
        match seen {
            Seen::Owned(group) => group,
            Seen::Unowned => panic!("unowned"),
            Seen::Empty => panic!("empty"),
        }
    }

    #[test]
    fn a_live_holder_lets_its_members_be_killed() {
        let (mut holder, mut m) = {
            let h = leader();
            let m = member(&h);
            (h, m)
        };
        let group = owned(inspect(holder.id(), start(&holder)).expect("inspect"));
        assert!(group.kill().expect("kill"));
        m.wait().expect("reap member");
        assert!(alive(&mut holder), "the holder is left to exit by itself");
        end(&mut holder);
    }

    /// The holder exits between the check and the signal: nothing is
    /// signalled, since its members could no longer be told from a later
    /// group's.
    #[test]
    fn nothing_is_signalled_once_the_holder_exits_after_the_check() {
        let mut holder = leader();
        let mut m = member(&holder);
        let group = owned(inspect(holder.id(), start(&holder)).expect("inspect"));
        end(&mut holder);
        assert!(!group.kill().expect("kill"));
        assert!(alive(&mut m), "no member was signalled");
        assert!(matches!(
            inspect(holder.id(), 0).expect("inspect"),
            Seen::Unowned
        ));
        end(&mut m);
    }

    /// A member exits between the check and the signal and a new process
    /// joins the group: the signal goes to the pinned member, which is gone,
    /// never to whatever now has a number in the group.
    #[test]
    fn a_member_that_exits_after_the_check_is_never_replaced_as_a_target() {
        let mut holder = leader();
        let mut m = member(&holder);
        let group = owned(inspect(holder.id(), start(&holder)).expect("inspect"));
        end(&mut m);
        let mut later = member(&holder);
        assert!(group.kill().expect("kill"));
        assert!(alive(&mut later), "an unpinned process is not signalled");
        end(&mut later);
        end(&mut holder);
    }

    /// A holder that is not the recorded one does not own the group.
    #[test]
    fn a_holder_with_another_start_time_owns_nothing() {
        let mut holder = leader();
        let mut m = member(&holder);
        let seen = inspect(holder.id(), start(&holder) + 1).expect("inspect");
        assert!(matches!(seen, Seen::Unowned));
        end(&mut m);
        end(&mut holder);
        assert!(matches!(
            inspect(holder.id(), 0).expect("inspect"),
            Seen::Empty
        ));
    }

    /// A fake `/proc` with one entry per `(name, stat)`; `None` makes the
    /// entry's `stat` a directory, which no read can get through.
    fn fake_proc(tag: &str, entries: &[(&str, Option<&str>)]) -> PathBuf {
        let root = std::env::temp_dir().join(format!("bbp_groups_{}_{tag}", std::process::id()));
        let _ = std::fs::remove_dir_all(&root);
        for (name, stat) in entries {
            let dir = root.join(name);
            std::fs::create_dir_all(&dir).expect("mkdir");
            match stat {
                Some(text) => std::fs::write(dir.join("stat"), text).expect("stat"),
                None => std::fs::create_dir(dir.join("stat")).expect("unreadable stat"),
            }
        }
        root
    }

    #[test]
    fn an_unreadable_surviving_member_is_an_error_never_an_empty_group() {
        let proc = fake_proc(
            "unreadable",
            &[("1", Some("1 (init) S 0 1 1")), ("42", None)],
        );
        let seen = unowned(&proc, 7);
        assert!(seen.is_err_and(|e| e.contains("42")));
        let _ = std::fs::remove_dir_all(&proc);
    }

    #[test]
    fn a_malformed_stat_is_an_error_never_a_nonmember() {
        let proc = fake_proc("malformed", &[("42", Some("42 (sh"))]);
        assert!(unowned(&proc, 7).is_err_and(|e| e.contains("malformed")));
        let _ = std::fs::remove_dir_all(&proc);
    }

    /// Exited while being listed: its directory is gone by the time its
    /// stat is read. That alone is absence.
    #[test]
    fn only_a_process_that_exited_is_absent() {
        let proc = fake_proc(
            "races",
            &[
                ("41", Some("41 (a b) S 1 7 7")),
                ("42", Some("42 (sh) Z 1 7 7")),
            ],
        );
        assert!(
            matches!(unowned(&proc, 7), Ok(Seen::Unowned)),
            "a live member"
        );
        std::fs::remove_dir_all(proc.join("41")).expect("exit");
        assert!(
            matches!(unowned(&proc, 7), Ok(Seen::Empty)),
            "a zombie and a gone one"
        );
        let _ = std::fs::remove_dir_all(&proc);
    }

    #[test]
    fn an_unlistable_proc_is_an_error() {
        let missing = std::env::temp_dir().join(format!("bbp_groups_{}_none", std::process::id()));
        assert!(unowned(&missing, 7).is_err());
    }

    /// Whether legacy work ran cannot be decided from a turn-log directory
    /// that cannot be listed: the task is refused, not called recorded-free.
    #[test]
    fn an_unlistable_turn_log_directory_refuses_the_task() {
        let dir = std::env::temp_dir().join(format!("bbp_groups_{}_legacy", std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);
        let task = TaskId("T".into());
        let agents = task_dir(&dir, "agents", &task);
        std::fs::create_dir_all(agents.parent().expect("parent")).expect("mkdir");
        std::fs::write(&agents, "").expect("a file where the directory should be");
        assert!(records(&dir, &task).is_err());
        assert!(enable(&dir, &task).is_err());
        assert!(!dir_of(&dir, &task).exists(), "no capability is granted");
        let _ = std::fs::remove_dir_all(&dir);
    }
}
