//! Fair admission: which of the moderators waiting on a slot directory takes
//! the next free slot.
//!
//! A moderator that wants a slot of a class holds one ticket for it: a lock
//! on `<dir>/queue/<id>.lock`, held while it waits, beside its record in
//! `<id>.json`. A ticket whose lock is free belongs to a waiter that died and
//! is removed. A moderator that takes a slot records who holds it in
//! `<class>-<n>.owner`; that record counts only while the slot is held.
//! Every decision is taken under `<dir>/admit.lock`, so tickets, owners and
//! the slots themselves are seen and changed by one moderator at a time.
//!
//! Tickets are served in rank order: when the ticket was taken, less its
//! claim's `priority_secs`. A rank never changes, so a ticket can be passed
//! only by tickets taken at most the largest priority difference after it:
//! with every holder bounded by its wall limit, no waiter starves. Walking
//! the queue in that order, each ticket takes a free slot when it fits; one
//! that does not fit leaves the slot to the next, so no ticket waits behind
//! another that cannot run. A slot left to an earlier ticket is reserved for
//! it until its moderator comes back for it.
//!
//! For agent slots a ticket fits when:
//!
//! - **Ceiling.** Its parent's slots, held or just given in this walk, are
//!   fewer than the parent's `agents`. Every slot counts once, toward the one
//!   parent of the task holding it, so a parent's tasks share its ceiling and
//!   never add up past it.
//! - **Leases.** No other task's write lease overlaps its own, whether that
//!   task holds a slot or waits ahead of it. Harnesses of one task share
//!   their writes; two tasks share none. An earlier waiter keeps its paths
//!   while it waits, so later conflicting tickets cannot starve it.
//!
//! Validator slots are served in rank order alone.

use crate::slots::{Class, Slot, Slots};
use rusty_serde::{Deserialize, Serialize};
use std::fs::{File, OpenOptions, TryLockError};
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicU64, Ordering};

/// What a moderator's tasks ask of admission, beyond a slot.
#[derive(Clone, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct Claim {
    /// The parent whose ceiling the task's agent slots count toward.
    #[rusty_serde(default)]
    pub parent: Option<Parent>,
    /// How many seconds earlier than its arrival a ticket is ranked.
    #[rusty_serde(default)]
    pub priority_secs: u64,
    /// The task's leased write paths: absolute, compared by component.
    #[rusty_serde(default)]
    pub write: Vec<String>,
}

/// A parent and how many agent slots its tasks may hold at once.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct Parent {
    pub id: String,
    pub agents: u32,
}

impl Claim {
    /// Read a claim file: the JSON form of [`Claim`].
    ///
    /// # Errors
    /// When the file cannot be read or parsed, or the claim is invalid.
    pub fn read_file(path: &Path) -> Result<Claim, String> {
        let text = std::fs::read_to_string(path).map_err(|e| format!("{}: {e}", path.display()))?;
        let claim: Claim =
            rusty_serde::json::from_str(&text).map_err(|e| format!("{}: {e}", path.display()))?;
        claim.validate()?;
        Ok(claim)
    }

    fn validate(&self) -> Result<(), String> {
        if let Some(parent) = &self.parent {
            if parent.id.trim().is_empty() || parent.agents == 0 {
                return Err("a claim's parent needs an id and at least one agent".into());
            }
        }
        if self.priority_secs > MAX_PRIORITY_SECS {
            return Err(format!("priority_secs must be at most {MAX_PRIORITY_SECS}"));
        }
        if !self.write.iter().all(|p| Path::new(p).is_absolute()) {
            return Err("a claim's write paths must be absolute".into());
        }
        Ok(())
    }

    fn overlaps(&self, other: &Claim) -> bool {
        self.write.iter().any(|a| {
            other.write.iter().any(|b| {
                let (a, b) = (Path::new(a), Path::new(b));
                a.starts_with(b) || b.starts_with(a)
            })
        })
    }
}

/// A day: more precedence than that is no queue at all.
const MAX_PRIORITY_SECS: u64 = 86_400;

/// Who waits for, or holds, a slot.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
struct Record {
    task: String,
    rank_ms: u64,
    claim: Claim,
}

impl Record {
    fn same_parent(&self, other: &Record) -> bool {
        matches!((&self.claim.parent, &other.claim.parent), (Some(a), Some(b)) if a.id == b.id)
    }

    fn conflicts(&self, other: &Record) -> bool {
        self.task != other.task && self.claim.overlaps(&other.claim)
    }
}

/// A moderator's place in the queue for one class. Dropping it leaves the
/// queue; a moderator that dies leaves it too, as its lock is released.
#[derive(Debug)]
pub struct Ticket {
    lock: Option<File>,
    stem: PathBuf,
}

impl Drop for Ticket {
    fn drop(&mut self) {
        // Closed first: Windows may keep the name of an open file it deletes.
        drop(self.lock.take());
        remove(&self.stem);
    }
}

/// A slot directory and the claim one moderator's task makes on it.
#[derive(Clone, Debug)]
pub struct Admission {
    slots: Slots,
    task: String,
    claim: Claim,
}

impl Admission {
    /// Admission to `slots` for `task` under `claim`.
    ///
    /// # Errors
    /// When the claim is invalid or the task id is blank.
    pub fn new(slots: Slots, task: String, claim: Claim) -> Result<Admission, String> {
        if task.trim().is_empty() {
            return Err("admission needs a task id".into());
        }
        claim.validate()?;
        Ok(Admission { slots, task, claim })
    }

    /// Take a slot of `class` if this task's turn has come, or wait in the
    /// queue with `ticket`, which is taken on the first call and given up
    /// with the slot.
    ///
    /// # Errors
    /// When a file of the slot directory cannot be read or written.
    pub fn admit(&self, class: Class, ticket: &mut Option<Ticket>) -> Result<Option<Slot>, String> {
        self.admit_at(class, ticket, unix_ms()?)
    }

    fn admit_at(
        &self,
        class: Class,
        ticket: &mut Option<Ticket>,
        now_ms: u64,
    ) -> Result<Option<Slot>, String> {
        let _turn = self.lock()?;
        let mine = match ticket {
            Some(t) => t.stem.clone(),
            None => {
                let t = self.ticket(class, now_ms)?;
                let stem = t.stem.clone();
                *ticket = Some(t);
                stem
            }
        };
        let held = self.held(class)?;
        let queue = self.queue(class)?;
        let Some(order) = queue.iter().position(|(stem, _)| *stem == mine) else {
            return Err(format!("{}: own ticket missing", mine.display()));
        };
        let free = (self.slots.capacity(class) as usize).saturating_sub(held.len());
        let owners: Vec<&Record> = held.iter().flatten().collect();
        if !served(class, &owners, &queue, free, order) {
            return Ok(None);
        }
        let Some(slot) = self.slots.try_acquire(class)? else {
            // Taken outside admission; wait for the next.
            return Ok(None);
        };
        let record = &queue[order].1;
        let owner = self.owner_path(class, slot.index());
        let json = rusty_serde::json::to_string(record).map_err(|e| e.to_string())?;
        rusty_atomic_file::write(&owner, json.as_bytes())
            .map_err(|e| format!("{}: {e}", owner.display()))?;
        *ticket = None;
        Ok(Some(slot))
    }

    /// Hold `<dir>/admit.lock` until the returned file is dropped.
    fn lock(&self) -> Result<File, String> {
        let path = self.slots.dir().join("admit.lock");
        let file = OpenOptions::new()
            .create(true)
            .truncate(false)
            .write(true)
            .open(&path)
            .map_err(|e| format!("{}: {e}", path.display()))?;
        file.lock()
            .map_err(|e| format!("{}: {e}", path.display()))?;
        Ok(file)
    }

    fn ticket(&self, class: Class, now_ms: u64) -> Result<Ticket, String> {
        static SEQ: AtomicU64 = AtomicU64::new(0);
        let queue = self.slots.dir().join("queue");
        std::fs::create_dir_all(&queue).map_err(|e| format!("{}: {e}", queue.display()))?;
        let seq = SEQ.fetch_add(1, Ordering::Relaxed);
        let stem = queue.join(format!(
            "{}-{now_ms}-{}-{seq}",
            class.name(),
            std::process::id()
        ));
        let lock = with_ext(&stem, "lock");
        let file = OpenOptions::new()
            .create_new(true)
            .write(true)
            .open(&lock)
            .map_err(|e| format!("{}: {e}", lock.display()))?;
        file.try_lock()
            .map_err(|e| format!("{}: {e}", lock.display()))?;
        let ticket = Ticket {
            lock: Some(file),
            stem,
        };
        let record = Record {
            task: self.task.clone(),
            rank_ms: now_ms.saturating_sub(self.claim.priority_secs * 1000),
            claim: self.claim.clone(),
        };
        let json = rusty_serde::json::to_string(&record).map_err(|e| e.to_string())?;
        let path = with_ext(&ticket.stem, "json");
        rusty_atomic_file::write(&path, json.as_bytes())
            .map_err(|e| format!("{}: {e}", path.display()))?;
        Ok(ticket)
    }

    /// One entry per held slot of `class`: its owner, or `None` for a slot
    /// taken outside admission, which holds capacity and nothing else.
    fn held(&self, class: Class) -> Result<Vec<Option<Record>>, String> {
        self.slots
            .held(class)?
            .into_iter()
            .map(|index| read_record(&self.owner_path(class, index)))
            .collect()
    }

    /// The live tickets of `class` in rank order. Stale ones are removed.
    fn queue(&self, class: Class) -> Result<Vec<(PathBuf, Record)>, String> {
        let dir = self.slots.dir().join("queue");
        let prefix = format!("{}-", class.name());
        let mut queue = Vec::new();
        for entry in std::fs::read_dir(&dir).map_err(|e| format!("{}: {e}", dir.display()))? {
            let path = entry.map_err(|e| format!("{}: {e}", dir.display()))?.path();
            let live = path.extension().is_some_and(|x| x == "lock")
                && path
                    .file_name()
                    .and_then(|n| n.to_str())
                    .is_some_and(|n| n.starts_with(&prefix));
            if !live {
                continue;
            }
            let stem = path.with_extension("");
            if !locked(&path)? {
                remove(&stem);
                continue;
            }
            // Its waiter is leaving the queue as we look.
            if let Some(record) = read_record(&with_ext(&stem, "json"))? {
                queue.push((stem, record));
            }
        }
        queue.sort_by(|(a, x), (b, y)| (x.rank_ms, a).cmp(&(y.rank_ms, b)));
        Ok(queue)
    }

    fn owner_path(&self, class: Class, index: u32) -> PathBuf {
        self.slots
            .dir()
            .join(format!("{}-{index}.owner", class.name()))
    }
}

/// Whether the ticket at `order` in `queue` takes one of `free` slots, with
/// `owners` holding the others.
fn served(
    class: Class,
    owners: &[&Record],
    queue: &[(PathBuf, Record)],
    free: usize,
    order: usize,
) -> bool {
    let mut given: Vec<&Record> = Vec::new();
    for (index, (_, record)) in queue.iter().enumerate().take(order + 1) {
        if given.len() == free {
            return false;
        }
        let ahead = queue[..index].iter().map(|(_, r)| r);
        let fits = match class {
            Class::Validator => true,
            Class::Agent => {
                under_ceiling(record, owners.iter().copied().chain(given.iter().copied()))
                    && !owners
                        .iter()
                        .copied()
                        .chain(ahead)
                        .any(|other| record.conflicts(other))
            }
        };
        if fits {
            if index == order {
                return true;
            }
            given.push(record);
        }
    }
    false
}

fn under_ceiling<'a>(record: &Record, taken: impl Iterator<Item = &'a Record>) -> bool {
    let Some(parent) = &record.claim.parent else {
        return true;
    };
    let count = taken.filter(|other| record.same_parent(other)).count();
    count < parent.agents as usize
}

/// Whether some open file description holds the lock on `path`.
fn locked(path: &Path) -> Result<bool, String> {
    let file = match OpenOptions::new().write(true).open(path) {
        Ok(file) => file,
        Err(e) if e.kind() == std::io::ErrorKind::NotFound => return Ok(false),
        Err(e) => return Err(format!("{}: {e}", path.display())),
    };
    match file.try_lock() {
        Ok(()) => Ok(false),
        Err(TryLockError::WouldBlock) => Ok(true),
        Err(TryLockError::Error(e)) => Err(format!("{}: {e}", path.display())),
    }
}

/// The record at `path`, or `None` when there is none.
fn read_record(path: &Path) -> Result<Option<Record>, String> {
    match std::fs::read_to_string(path) {
        Ok(text) => rusty_serde::json::from_str(&text)
            .map(Some)
            .map_err(|e| format!("{}: {e}", path.display())),
        Err(e) if e.kind() == std::io::ErrorKind::NotFound => Ok(None),
        Err(e) => Err(format!("{}: {e}", path.display())),
    }
}

/// Remove a ticket's files; one already gone is fine.
fn remove(stem: &Path) {
    for ext in ["json", "lock"] {
        let path = with_ext(stem, ext);
        if let Err(e) = std::fs::remove_file(&path) {
            if e.kind() != std::io::ErrorKind::NotFound {
                eprintln!("bbp mod: ticket {} not removed: {e}", path.display());
            }
        }
    }
}

fn with_ext(stem: &Path, ext: &str) -> PathBuf {
    stem.with_extension(ext)
}

fn unix_ms() -> Result<u64, String> {
    std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map_err(|e| format!("clock: {e}"))
        .map(|d| u64::try_from(d.as_millis()).unwrap_or(u64::MAX))
}

#[cfg(test)]
mod tests;
