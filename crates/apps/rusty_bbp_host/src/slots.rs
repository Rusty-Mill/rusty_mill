//! Machine-wide admission: a fixed number of agent and validator slots,
//! shared by every moderator that names the same slot directory.
//!
//! A slot is an exclusive kernel lock on `<dir>/<class>-<n>.lock`. The
//! moderator hands the locked file to the process it starts (a harness's
//! stdin, a runner's stdout), so the lock lives in the open file description
//! that the moderator and the whole process tree share. It is released only
//! when the last of them has closed it: after the moderator has reaped its
//! child, or, when the moderator dies, after the child dies too. No
//! reservation table exists, so none can be left behind by a crash, and a
//! restarted moderator reclaims a slot exactly when its holder is gone.
//!
//! A harness is a launcher's program, free to replace its stdin. So the
//! moderator also hands the lock to a `bbp slot-hold` process of its own
//! ([`Slot::hold_for`]), which keeps it until the harness's process group
//! has no member left, whether or not the moderator lives that long.
//!
//! The capacity is recorded once, in `<dir>/capacity`, by the first
//! moderator to open the directory. A moderator configured with a different
//! capacity is refused, so two configurations can never share one directory
//! and together exceed either.

use std::fs::{File, OpenOptions, TryLockError};
use std::path::{Path, PathBuf};
use std::process::{Child, Command, Stdio};
use std::time::Duration;

/// What a slot admits.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Class {
    /// An agent harness, for one live turn.
    Agent,
    /// A runner, for one selected run.
    Validator,
}

impl Class {
    fn name(self) -> &'static str {
        match self {
            Self::Agent => "agent",
            Self::Validator => "validator",
        }
    }
}

/// How many processes of each class may run at once.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Capacity {
    pub agents: u32,
    pub validators: u32,
}

impl Capacity {
    fn of(self, class: Class) -> u32 {
        match class {
            Class::Agent => self.agents,
            Class::Validator => self.validators,
        }
    }

    fn text(self) -> String {
        format!("agents={} validators={}\n", self.agents, self.validators)
    }
}

/// One slot directory with its recorded capacity.
#[derive(Clone, Debug)]
pub struct Slots {
    dir: PathBuf,
    capacity: Capacity,
}

/// A held slot. Dropping it closes the moderator's handle; the slot is free
/// once every process given [`Slot::stdio`] has exited too.
#[derive(Debug)]
pub struct Slot {
    file: File,
    class: Class,
    index: u32,
}

impl Slots {
    /// Open `dir`, recording `capacity` there when it is the first to.
    ///
    /// # Errors
    /// When a class has no slots, the directory cannot be made, or it records
    /// a different capacity.
    pub fn open(dir: &Path, capacity: Capacity) -> Result<Slots, String> {
        if capacity.agents == 0 || capacity.validators == 0 {
            return Err("slot capacity must be at least one agent and one validator".into());
        }
        std::fs::create_dir_all(dir).map_err(|e| format!("{}: {e}", dir.display()))?;
        let path = dir.join("capacity");
        publish_once(&path, capacity.text().as_bytes())?;
        let recorded =
            std::fs::read_to_string(&path).map_err(|e| format!("{}: {e}", path.display()))?;
        if recorded != capacity.text() {
            return Err(format!(
                "{} records {}, not {}",
                path.display(),
                recorded.trim(),
                capacity.text().trim()
            ));
        }
        Ok(Slots {
            dir: dir.to_path_buf(),
            capacity,
        })
    }

    /// Take a free slot of `class`, or `None` when every one is held.
    ///
    /// # Errors
    /// When a slot file cannot be opened or locked for another reason than
    /// being held.
    pub fn try_acquire(&self, class: Class) -> Result<Option<Slot>, String> {
        for index in 0..self.capacity.of(class) {
            let path = self.dir.join(format!("{}-{index}.lock", class.name()));
            let file = OpenOptions::new()
                .create(true)
                .truncate(false)
                .read(true)
                .write(true)
                .open(&path)
                .map_err(|e| format!("{}: {e}", path.display()))?;
            match file.try_lock() {
                Ok(()) => {
                    // A runner's output from an earlier hold is no one's. Not
                    // opened for append: Windows refuses to truncate such a handle.
                    file.set_len(0)
                        .map_err(|e| format!("{}: {e}", path.display()))?;
                    return Ok(Some(Slot { file, class, index }));
                }
                Err(TryLockError::WouldBlock) => continue,
                Err(TryLockError::Error(e)) => return Err(format!("{}: {e}", path.display())),
            }
        }
        Ok(None)
    }
}

impl Slot {
    /// The locked file as a child's stdio: the child then holds the slot
    /// until it and everything it started have exited. Read, it is empty.
    ///
    /// # Errors
    /// When the descriptor cannot be duplicated.
    pub fn stdio(&self) -> Result<Stdio, String> {
        self.file
            .try_clone()
            .map(Stdio::from)
            .map_err(|e| format!("slot: {e}"))
    }

    /// Hold this slot in a `bbp slot-hold` process, in a process group of
    /// its own, for as long as process group `group` has a member: however
    /// that group's processes treat their stdio, and whether or not the
    /// caller outlives them.
    ///
    /// # Errors
    /// When the holder cannot be started.
    pub fn hold_for(&self, bbp: &Path, group: u32) -> Result<Child, String> {
        let mut command = Command::new(bbp);
        command
            .args(["slot-hold", "--group", &group.to_string()])
            .stdin(self.stdio()?)
            .stdout(Stdio::null())
            .stderr(Stdio::inherit());
        #[cfg(unix)]
        std::os::unix::process::CommandExt::process_group(&mut command, 0);
        command.spawn().map_err(|e| format!("slot holder: {e}"))
    }

    /// `agent-0`, `validator-0`, ...
    pub fn name(&self) -> String {
        format!("{}-{}", self.class.name(), self.index)
    }
}

/// `bbp slot-hold`: return once process group `group` has no member left.
/// The slot lock, the caller's stdin, stays open until then.
pub fn hold_while_alive(group: i32) {
    while group_alive(group) {
        std::thread::sleep(Duration::from_millis(100));
    }
}

/// A group whose id was freed and reused within one poll keeps the slot
/// longer than needed: late admission, never oversubscription.
#[cfg(target_os = "linux")]
fn group_alive(group: i32) -> bool {
    group > 0 && rusty_libc::process::killpg(group, 0) != Err(rusty_libc::Errno::ESRCH)
}

#[cfg(not(target_os = "linux"))]
fn group_alive(_: i32) -> bool {
    false
}

/// Write `bytes` to `path` unless it already exists, without ever exposing a
/// partial file: a private temp file is linked into place, no-clobber.
fn publish_once(path: &Path, bytes: &[u8]) -> Result<(), String> {
    if path.exists() {
        return Ok(());
    }
    let tmp = path.with_file_name(format!(".capacity.{}", std::process::id()));
    std::fs::write(&tmp, bytes).map_err(|e| format!("{}: {e}", tmp.display()))?;
    let linked = std::fs::hard_link(&tmp, path);
    let _ = std::fs::remove_file(&tmp);
    match linked {
        Ok(()) => Ok(()),
        Err(e) if e.kind() == std::io::ErrorKind::AlreadyExists => Ok(()),
        Err(e) => Err(format!("{}: {e}", path.display())),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn dir(tag: &str) -> PathBuf {
        let dir = std::env::temp_dir().join(format!("bbp_slots_{}_{tag}", std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);
        dir
    }

    const TWO_AND_ONE: Capacity = Capacity {
        agents: 2,
        validators: 1,
    };

    #[test]
    fn each_class_admits_up_to_its_capacity_and_frees_on_drop() -> Result<(), String> {
        let slots = Slots::open(&dir("cap"), TWO_AND_ONE)?;
        let a = slots.try_acquire(Class::Agent)?.ok_or("first agent")?;
        let b = slots.try_acquire(Class::Agent)?.ok_or("second agent")?;
        assert_eq!((a.name(), b.name()), ("agent-0".into(), "agent-1".into()));
        assert!(slots.try_acquire(Class::Agent)?.is_none());
        let v = slots.try_acquire(Class::Validator)?.ok_or("validator")?;
        assert!(slots.try_acquire(Class::Validator)?.is_none());
        drop(a);
        assert_eq!(
            slots.try_acquire(Class::Agent)?.map(|s| s.name()),
            Some("agent-0".into())
        );
        drop(v);
        assert!(slots.try_acquire(Class::Validator)?.is_some());
        Ok(())
    }

    #[test]
    fn a_directory_keeps_the_capacity_it_was_first_opened_with() -> Result<(), String> {
        let path = dir("mismatch");
        Slots::open(&path, TWO_AND_ONE)?;
        Slots::open(&path, TWO_AND_ONE)?;
        let error = Slots::open(
            &path,
            Capacity {
                agents: 3,
                validators: 1,
            },
        )
        .err()
        .ok_or("a second capacity is refused")?;
        assert!(error.contains("records agents=2 validators=1"), "{error}");
        assert!(Slots::open(
            &dir("zero"),
            Capacity {
                agents: 0,
                validators: 1
            }
        )
        .is_err());
        Ok(())
    }

    // Inheriting a lock with a duplicated descriptor is Unix semantics; the
    // crash guarantees are proven on Linux.
    #[test]
    #[cfg(unix)]
    fn a_child_holds_its_slot_until_it_is_reaped() -> Result<(), String> {
        let slots = Slots::open(
            &dir("child"),
            Capacity {
                agents: 1,
                validators: 1,
            },
        )?;
        let slot = slots.try_acquire(Class::Agent)?.ok_or("agent")?;
        let mut child = std::process::Command::new("sleep")
            .arg("30")
            .stdin(slot.stdio()?)
            .spawn()
            .map_err(|e| e.to_string())?;
        // The moderator's own handle is gone, as after a crash: the child
        // still holds the slot.
        drop(slot);
        assert!(slots.try_acquire(Class::Agent)?.is_none());
        child.kill().map_err(|e| e.to_string())?;
        child.wait().map_err(|e| e.to_string())?;
        // Another test's fork may hold a copy of the descriptor until it
        // execs, so the release is awaited, briefly.
        let deadline = std::time::Instant::now() + Duration::from_secs(5);
        while slots.try_acquire(Class::Agent)?.is_none() {
            assert!(std::time::Instant::now() < deadline, "slot never freed");
            std::thread::sleep(Duration::from_millis(10));
        }
        Ok(())
    }
}
