//! A redo journal for crash-atomic batches across several stores, and the
//! durable sequence counters that ride on it (`rusty_remind_me`'s ADR-0023
//! §3b and §3c).
//!
//! Each store's insert log makes one write durable, but a batch touching
//! several stores (a memory, its tags and its outbox entry) could land in
//! some and not others if the process dies between them. The journal
//! closes that gap the way `rusty_multimodal_db`'s server journal does (its
//! ADR-0025), written fresh here because an app crate cannot be a
//! dependency of this one:
//!
//! 1. [`Journal::commit`] writes the whole batch as one entry and `fsync`s
//!    it, **before** any store sees it.
//! 2. The caller then applies it to each store, with the stores' own syncs
//!    deferred if it likes.
//! 3. Once every store's files are synced, [`Journal::checkpoint`] empties
//!    the journal.
//!
//! [`Journal::open`] hands back every batch committed since the last
//! checkpoint, oldest first, for the caller to apply again. A batch is a
//! list of whole-value puts and deletes keyed by id, so applying one twice
//! leaves what applying it once did: replay needs no record of what already
//! landed.
//!
//! # Sequences
//!
//! Named `u64` counters, for ids that must never be issued twice (outbox
//! ids, chunk ids). [`Journal::allocate`] issues the next value in memory;
//! every committed entry carries all the counters, so a value becomes
//! durable with the batch that uses it. A value issued but never committed
//! may be issued again after a crash, which is safe: nothing durable holds
//! it. A checkpoint keeps the counters, so they never go backwards.
//!
//! # Format
//!
//! `ENGJRNL\0`, a `u32` LE format version, then entries of `[u32 LE
//! len][u32 LE CRC-32 of the payload][payload]`, the payload being a
//! [`crate::codec`]-encoded entry. An incomplete last entry, or a last
//! entry whose checksum fails (its length landed but its bytes did not),
//! is a torn tail: it was never acknowledged, so open drops it and
//! truncates it away. A bad checksum or an undecodable payload anywhere
//! before the last entry is corruption, and open refuses the journal.

use crate::codec;
use crate::durability::sync_parent_dir;
use serde::{Deserialize, Serialize};
use std::collections::BTreeMap;
use std::fmt;
use std::fs::{File, OpenOptions};
use std::io::{self, Write};
use std::path::{Path, PathBuf};

/// The journal file's magic.
pub const JOURNAL_MAGIC: &[u8; 8] = b"ENGJRNL\0";
/// The format this build writes and reads; any other is refused.
pub const JOURNAL_VERSION: u32 = 1;

const HEADER_LEN: usize = 12;
const FRAME_LEN: usize = 8;

/// One store write within a batch.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct Change {
    /// Which store: a name the caller chooses and resolves on replay.
    pub store: String,
    /// The record's id, encoded however the caller encodes it.
    pub key: Vec<u8>,
    /// The whole new record, or `None` to delete it.
    pub value: Option<Vec<u8>>,
}

/// Writes that land together or not at all.
#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct Batch {
    pub changes: Vec<Change>,
}

impl Batch {
    pub fn put(&mut self, store: &str, key: Vec<u8>, value: Vec<u8>) -> &mut Self {
        self.changes.push(Change {
            store: store.to_string(),
            key,
            value: Some(value),
        });
        self
    }

    pub fn delete(&mut self, store: &str, key: Vec<u8>) -> &mut Self {
        self.changes.push(Change {
            store: store.to_string(),
            key,
            value: None,
        });
        self
    }

    pub fn is_empty(&self) -> bool {
        self.changes.is_empty()
    }
}

/// What one journal entry holds.
#[derive(Debug, Clone, Default, Serialize, Deserialize)]
struct Entry {
    /// Every counter's last issued value when the entry was written.
    sequences: BTreeMap<String, u64>,
    batch: Batch,
}

/// Everything that can go wrong with a journal.
#[derive(Debug)]
pub enum JournalError {
    Io(PathBuf, io::Error),
    /// Not a journal, another format version, or a corrupt entry.
    Format(PathBuf, String),
    /// A batch too large for a `u32` frame, or one that does not encode.
    Encode(String),
    /// An earlier commit failed part-way, so the file may end in a partial
    /// entry that a later append would bury. Refused until reopened.
    Failed(String),
}

impl fmt::Display for JournalError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            JournalError::Io(path, e) => write!(f, "journal {}: {e}", path.display()),
            JournalError::Format(path, why) => write!(f, "journal {}: {why}", path.display()),
            JournalError::Encode(why) => write!(f, "journal entry: {why}"),
            JournalError::Failed(why) => write!(
                f,
                "journal refuses writes until it is reopened, after an earlier failure: {why}"
            ),
        }
    }
}

impl std::error::Error for JournalError {}

/// An open journal, and what it held.
pub struct Opened {
    pub journal: Journal,
    /// Batches committed since the last checkpoint, oldest first, for the
    /// caller to apply again before anything else.
    pub replay: Vec<Batch>,
}

/// The journal itself. One writer: callers serialize access.
pub struct Journal {
    path: PathBuf,
    file: File,
    sequences: BTreeMap<String, u64>,
    bytes: u64,
    entries: u64,
    failed: Option<String>,
}

impl Journal {
    /// Open the journal at `path`, creating it if it does not exist.
    ///
    /// # Errors
    ///
    /// [`JournalError::Format`] for a file that is not a journal of this
    /// version or has a corrupt entry before its last; I/O errors as
    /// [`JournalError::Io`].
    pub fn open(path: &Path) -> Result<Opened, JournalError> {
        let io = |e| JournalError::Io(path.to_path_buf(), e);
        let raw = match std::fs::read(path) {
            Ok(raw) => raw,
            Err(e) if e.kind() == io::ErrorKind::NotFound => Vec::new(),
            Err(e) => return Err(io(e)),
        };
        if raw.len() < HEADER_LEN {
            // Absent, or a creation that died before its header was whole
            // (the header is written and synced before anything else).
            if !header().starts_with(&raw) {
                return Err(format_err(path, "not a journal (bad header)"));
            }
            let bytes = write_fresh(path, &Entry::default())?;
            return Ok(Opened {
                journal: Self::append_to(path, BTreeMap::new(), bytes, 1)?,
                replay: Vec::new(),
            });
        }
        if raw[..8] != JOURNAL_MAGIC[..] {
            return Err(format_err(path, "not a journal (bad magic)"));
        }
        let version = u32::from_le_bytes(raw[8..12].try_into().expect("four bytes"));
        if version != JOURNAL_VERSION {
            return Err(format_err(
                path,
                &format!("format version {version}; this build reads {JOURNAL_VERSION}"),
            ));
        }

        let (entries, whole) = parse_entries(path, &raw)?;
        if whole < raw.len() {
            truncate(path, whole as u64)?;
        }
        let mut sequences = BTreeMap::new();
        let mut replay = Vec::new();
        for entry in &entries {
            merge_max(&mut sequences, &entry.sequences);
        }
        let count = entries.len() as u64;
        for entry in entries {
            if !entry.batch.is_empty() {
                replay.push(entry.batch);
            }
        }
        Ok(Opened {
            journal: Self::append_to(path, sequences, whole as u64, count)?,
            replay,
        })
    }

    fn append_to(
        path: &Path,
        sequences: BTreeMap<String, u64>,
        bytes: u64,
        entries: u64,
    ) -> Result<Self, JournalError> {
        let file = OpenOptions::new()
            .append(true)
            .open(path)
            .map_err(|e| JournalError::Io(path.to_path_buf(), e))?;
        Ok(Self {
            path: path.to_path_buf(),
            file,
            sequences,
            bytes,
            entries,
            failed: None,
        })
    }

    /// Issue the next value of the counter `name`, starting at 1.
    ///
    /// Durable once a [`Journal::commit`] after this returns.
    pub fn allocate(&mut self, name: &str) -> u64 {
        let next = self.sequences.get(name).copied().unwrap_or(0) + 1;
        self.sequences.insert(name.to_string(), next);
        next
    }

    /// The last value issued for `name`, or 0.
    pub fn last(&self, name: &str) -> u64 {
        self.sequences.get(name).copied().unwrap_or(0)
    }

    /// Never issue `floor` or anything below it for `name` again: for a
    /// store copied in with ids already in use. Durable with the next
    /// commit, like [`Journal::allocate`].
    pub fn raise_to(&mut self, name: &str, floor: u64) {
        let slot = self.sequences.entry(name.to_string()).or_insert(0);
        *slot = (*slot).max(floor);
    }

    /// Make `batch`, and every counter's current value, durable. When this
    /// returns `Ok`, the batch survives a crash: apply it to the stores
    /// next.
    ///
    /// # Errors
    ///
    /// On any failure the journal refuses later commits until reopened
    /// ([`JournalError::Failed`]), since the file may now end in a partial
    /// entry.
    pub fn commit(&mut self, batch: &Batch) -> Result<(), JournalError> {
        if let Some(why) = &self.failed {
            return Err(JournalError::Failed(why.clone()));
        }
        let frame = frame(&Entry {
            sequences: self.sequences.clone(),
            batch: batch.clone(),
        })?;
        #[cfg(feature = "test-support")]
        let injected_prefix = crate::test_support::take_fault(|fault| {
            matches!(fault, crate::test_support::Fault::JournalWritePrefix(_))
        });
        #[cfg(feature = "test-support")]
        let written =
            if let Some(crate::test_support::Fault::JournalWritePrefix(prefix)) = injected_prefix {
                self.file
                    .write_all(&frame[..prefix.min(frame.len())])
                    .and_then(|()| Err(io::Error::other("injected journal partial write")))
            } else {
                self.file.write_all(&frame).and_then(|()| {
                    if crate::test_support::take_fault(|fault| {
                        fault == crate::test_support::Fault::JournalSync
                    })
                    .is_some()
                    {
                        Err(io::Error::other("injected journal sync failure"))
                    } else {
                        self.file.sync_data()
                    }
                })
            };
        #[cfg(not(feature = "test-support"))]
        let written = self
            .file
            .write_all(&frame)
            .and_then(|()| self.file.sync_data());
        if let Err(e) = written {
            self.failed = Some(e.to_string());
            return Err(JournalError::Io(self.path.clone(), e));
        }
        self.bytes += frame.len() as u64;
        self.entries += 1;
        Ok(())
    }

    /// Empty the journal, keeping the counters. Call only once every store
    /// a replayed or committed batch touched has synced its own files:
    /// after this, those batches are no longer replayed.
    ///
    /// # Errors
    ///
    /// I/O errors, after which the old journal is still in place and still
    /// replays.
    pub fn checkpoint(&mut self) -> Result<(), JournalError> {
        if let Some(why) = &self.failed {
            return Err(JournalError::Failed(why.clone()));
        }
        let entry = Entry {
            sequences: self.sequences.clone(),
            batch: Batch::default(),
        };
        let bytes = write_fresh(&self.path, &entry)?;
        let reopened = Self::append_to(&self.path, self.sequences.clone(), bytes, 1)?;
        *self = reopened;
        Ok(())
    }

    /// The file's size in bytes, header included: what to checkpoint on.
    pub fn bytes(&self) -> u64 {
        self.bytes
    }

    /// Entries in the file, the checkpoint's own included.
    pub fn entries(&self) -> u64 {
        self.entries
    }
}

fn header() -> [u8; HEADER_LEN] {
    let mut header = [0u8; HEADER_LEN];
    header[..8].copy_from_slice(JOURNAL_MAGIC);
    header[8..].copy_from_slice(&JOURNAL_VERSION.to_le_bytes());
    header
}

fn format_err(path: &Path, why: &str) -> JournalError {
    JournalError::Format(path.to_path_buf(), why.to_string())
}

fn frame(entry: &Entry) -> Result<Vec<u8>, JournalError> {
    let payload = codec::encode(entry).map_err(|e| JournalError::Encode(e.to_string()))?;
    let len = u32::try_from(payload.len())
        .map_err(|_| JournalError::Encode(format!("{} bytes is too large", payload.len())))?;
    let mut frame = Vec::with_capacity(FRAME_LEN + payload.len());
    frame.extend_from_slice(&len.to_le_bytes());
    frame.extend_from_slice(&crc32(&payload).to_le_bytes());
    frame.extend_from_slice(&payload);
    Ok(frame)
}

/// Every whole entry after the header, and where the whole ones end.
fn parse_entries(path: &Path, raw: &[u8]) -> Result<(Vec<Entry>, usize), JournalError> {
    let mut entries = Vec::new();
    let mut at = HEADER_LEN;
    while at < raw.len() {
        let rest = &raw[at..];
        if rest.len() < FRAME_LEN {
            break;
        }
        let len = u32::from_le_bytes(rest[..4].try_into().expect("four bytes")) as usize;
        let crc = u32::from_le_bytes(rest[4..8].try_into().expect("four bytes"));
        let Some(payload) = rest.get(FRAME_LEN..FRAME_LEN + len) else {
            break;
        };
        let end = at + FRAME_LEN + len;
        if crc32(payload) != crc {
            if end == raw.len() {
                break;
            }
            return Err(format_err(
                path,
                &format!("corrupt entry at byte {at}: checksum mismatch"),
            ));
        }
        let entry: Entry = codec::decode(payload)
            .map_err(|e| format_err(path, &format!("corrupt entry at byte {at}: {e}")))?;
        entries.push(entry);
        at = end;
    }
    Ok((entries, at))
}

fn truncate(path: &Path, len: u64) -> Result<(), JournalError> {
    let io = |e| JournalError::Io(path.to_path_buf(), e);
    let file = OpenOptions::new().write(true).open(path).map_err(io)?;
    file.set_len(len).map_err(io)?;
    file.sync_all().map_err(io)
}

/// Replace the journal with a header and `entry`, atomically. Returns the
/// new file's length.
fn write_fresh(path: &Path, entry: &Entry) -> Result<u64, JournalError> {
    let io = |e| JournalError::Io(path.to_path_buf(), e);
    let mut contents = header().to_vec();
    contents.extend_from_slice(&frame(entry)?);
    let staging = staging_path(path);
    let mut file = File::create(&staging).map_err(io)?;
    file.write_all(&contents).map_err(io)?;
    file.sync_all().map_err(io)?;
    drop(file);
    std::fs::rename(&staging, path).map_err(io)?;
    #[cfg(feature = "test-support")]
    if crate::test_support::take_fault(|fault| {
        fault == crate::test_support::Fault::CheckpointAfterRename
    })
    .is_some()
    {
        return Err(io(io::Error::other(
            "injected checkpoint post-rename failure",
        )));
    }
    sync_parent_dir(path).map_err(io)?;
    Ok(contents.len() as u64)
}

fn staging_path(path: &Path) -> PathBuf {
    let mut staging = path.as_os_str().to_owned();
    staging.push(".tmp");
    PathBuf::from(staging)
}

fn merge_max(into: &mut BTreeMap<String, u64>, from: &BTreeMap<String, u64>) {
    for (name, value) in from {
        let slot = into.entry(name.clone()).or_insert(0);
        *slot = (*slot).max(*value);
    }
}

/// CRC-32 (IEEE 802.3, reflected), as zlib computes it.
fn crc32(bytes: &[u8]) -> u32 {
    const TABLE: [u32; 256] = {
        let mut table = [0u32; 256];
        let mut n = 0;
        while n < 256 {
            let mut c = n as u32;
            let mut k = 0;
            while k < 8 {
                c = if c & 1 != 0 {
                    0xEDB8_8320 ^ (c >> 1)
                } else {
                    c >> 1
                };
                k += 1;
            }
            table[n] = c;
            n += 1;
        }
        table
    };
    !bytes.iter().fold(!0u32, |c, b| {
        TABLE[((c ^ u32::from(*b)) & 0xFF) as usize] ^ (c >> 8)
    })
}

/// Read a journal file whole, for tests that inspect its bytes.
#[cfg(test)]
fn read_all(path: &Path) -> Vec<u8> {
    use std::io::Read;
    let mut raw = Vec::new();
    File::open(path).unwrap().read_to_end(&mut raw).unwrap();
    raw
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::test_support::fresh_temp_dir;

    fn journal_path(label: &str) -> PathBuf {
        fresh_temp_dir(label).unwrap().join("store.journal")
    }

    fn batch(n: u8) -> Batch {
        let mut batch = Batch::default();
        batch
            .put("memories", vec![n], vec![n, n])
            .put("outbox", vec![n], vec![n])
            .delete("tags", vec![n]);
        batch
    }

    #[test]
    fn crc32_matches_the_standard_check_value() {
        assert_eq!(crc32(b"123456789"), 0xCBF4_3926);
        assert_eq!(crc32(b""), 0);
    }

    #[test]
    fn a_new_journal_is_empty_and_created() {
        let path = journal_path("jrn_new");
        let opened = Journal::open(&path).unwrap();
        assert!(opened.replay.is_empty());
        assert!(path.exists());
        assert_eq!(opened.journal.last("outbox"), 0);
    }

    #[test]
    fn committed_batches_replay_in_order_until_a_checkpoint() {
        let path = journal_path("jrn_replay");
        let mut journal = Journal::open(&path).unwrap().journal;
        journal.commit(&batch(1)).unwrap();
        journal.commit(&batch(2)).unwrap();
        drop(journal);

        let opened = Journal::open(&path).unwrap();
        assert_eq!(opened.replay, vec![batch(1), batch(2)]);

        let mut journal = opened.journal;
        journal.checkpoint().unwrap();
        drop(journal);
        assert!(Journal::open(&path).unwrap().replay.is_empty());
    }

    #[test]
    fn sequences_are_durable_with_a_commit_and_survive_a_checkpoint() {
        let path = journal_path("jrn_seq");
        let mut journal = Journal::open(&path).unwrap().journal;
        assert_eq!(journal.allocate("outbox"), 1);
        assert_eq!(journal.allocate("outbox"), 2);
        assert_eq!(journal.allocate("chunks"), 1);
        journal.commit(&batch(1)).unwrap();
        // Issued but never committed: may come back after a crash.
        assert_eq!(journal.allocate("outbox"), 3);
        drop(journal);

        let mut journal = Journal::open(&path).unwrap().journal;
        assert_eq!(journal.last("outbox"), 2);
        assert_eq!(journal.allocate("outbox"), 3);
        journal.raise_to("chunks", 40);
        journal.raise_to("outbox", 1);
        journal.checkpoint().unwrap();
        drop(journal);

        let mut journal = Journal::open(&path).unwrap().journal;
        assert_eq!(journal.allocate("outbox"), 4, "never backwards");
        assert_eq!(journal.allocate("chunks"), 41);
    }

    #[test]
    fn a_torn_tail_is_dropped_and_truncated() {
        let path = journal_path("jrn_torn");
        let mut journal = Journal::open(&path).unwrap().journal;
        journal.commit(&batch(1)).unwrap();
        let whole = journal.bytes();
        journal.commit(&batch(2)).unwrap();
        let full = journal.bytes();
        drop(journal);

        // Every way the second entry can be cut short.
        let raw = read_all(&path);
        for cut in (whole + 1..full).step_by(3) {
            std::fs::write(&path, &raw[..cut as usize]).unwrap();
            let opened = Journal::open(&path).unwrap();
            assert_eq!(opened.replay, vec![batch(1)], "cut at {cut}");
            assert_eq!(std::fs::metadata(&path).unwrap().len(), whole);
        }
    }

    #[test]
    fn a_last_entry_whose_bytes_never_landed_is_torn_not_corrupt() {
        let path = journal_path("jrn_zeroed");
        let mut journal = Journal::open(&path).unwrap().journal;
        journal.commit(&batch(1)).unwrap();
        let whole = journal.bytes() as usize;
        journal.commit(&batch(2)).unwrap();
        drop(journal);
        let mut raw = read_all(&path);
        // Length and checksum landed, the payload came back as zeros.
        for b in &mut raw[whole + FRAME_LEN..] {
            *b = 0;
        }
        std::fs::write(&path, &raw).unwrap();
        assert_eq!(Journal::open(&path).unwrap().replay, vec![batch(1)]);
    }

    #[test]
    fn a_corrupt_entry_before_the_last_is_refused() {
        let path = journal_path("jrn_corrupt");
        let mut journal = Journal::open(&path).unwrap().journal;
        journal.commit(&batch(1)).unwrap();
        journal.commit(&batch(2)).unwrap();
        drop(journal);
        let mut raw = read_all(&path);
        raw[HEADER_LEN + FRAME_LEN] ^= 0xFF;
        std::fs::write(&path, &raw).unwrap();
        let err = Journal::open(&path).err().unwrap();
        assert!(matches!(err, JournalError::Format(..)), "{err}");
    }

    #[test]
    fn a_foreign_file_or_version_is_refused() {
        let path = journal_path("jrn_foreign");
        std::fs::write(&path, b"GENINSL\0\x02\0\0\0").unwrap();
        assert!(matches!(
            Journal::open(&path).err().unwrap(),
            JournalError::Format(..)
        ));
        let mut other = header();
        other[8] = 9;
        std::fs::write(&path, other).unwrap();
        let err = Journal::open(&path).err().unwrap();
        assert!(err.to_string().contains("version 9"), "{err}");
    }

    #[test]
    fn a_header_cut_short_at_creation_starts_fresh() {
        let path = journal_path("jrn_short");
        std::fs::write(&path, &JOURNAL_MAGIC[..5]).unwrap();
        let opened = Journal::open(&path).unwrap();
        assert!(opened.replay.is_empty());
        assert_eq!(&read_all(&path)[..HEADER_LEN], &header());
    }
}
