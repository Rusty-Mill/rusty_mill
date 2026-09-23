//! The batch journal behind crash-atomic transactions (`SERVER-001`
//! FR-025, ADR-0025, `docs/design/SERVER-TRANSACTION-SESSION-DESIGN.md`
//! Part B): a redo log of *intended* writes that a domain adapter
//! (`DogConnectionStore::with_journal` and its `Order`/`Employee`
//! siblings) appends and `fsync`s **before** applying a batch, replays on
//! the next open, and checkpoints by size.
//!
//! # Why redo suffices, and why it lives here
//!
//! Every operation this protocol can batch is an idempotent overwrite of
//! a fixed-width slot keyed by a record id that is never deleted at
//! runtime, for a field whose type the schema fixes (the "no runtime
//! deletion, fixed schema" invariant `ADR-0013` already relied on).
//! Re-applying an applied operation changes nothing; applying one that
//! never landed produces the state it would have. So a log of intended
//! writes, durable before the first slot write and replayed in order,
//! restores a batch's all-or-nothing outcome from *any* intermediate
//! state — no prior values recorded, no slot file changed. The journal
//! is the adapter's, not the store's, because the adapter is the only
//! layer that both sees a batch (`ConnectionStore::apply_transaction`)
//! and knows how to apply one operation to its store; a store knows
//! neither `TransactionOp` nor `FieldRef`.
//!
//! # Format and discipline
//!
//! `TXNJRNL\0`, a `u32` LE format version, then entries of
//! `[u32 LE len][crate::codec(Vec<TransactionOp>)]` — `STORAGE-018`'s
//! encoding, so a journal written by one build replays under the next.
//! `BatchJournal::append` writes an entry and `sync_data`s (a journaled
//! adapter goes through `CommitGroup` instead, which separates the
//! write from the `fsync` so one `fsync` can cover several batches —
//! `SERVER-001` FR-027, ADR-0026, `docs/design/SERVER-JOURNAL-GROUP-COMMIT-DESIGN.md`);
//! a crash
//! before that returns leaves a *torn tail* — an entry whose length
//! prefix or payload is incomplete — which `BatchJournal::open` drops
//! (it was never acknowledged) and truncates away. A complete entry that
//! does not decode is corruption, not a torn tail, and is a
//! [`JournalError::Format`]. The journal is never truncated except by a
//! checkpoint that first makes the store's own files durable
//! ([`CheckpointFlush`]) — `src/durability/hybrid.rs`'s discipline with
//! the batch as the unit and the `.mmap` files as the snapshot — so it
//! always holds exactly the batches applied since the last moment every
//! slot write was known to be on disk.

use super::protocol::{ErrorCode, TransactionOp, WriteOp};
use crate::durability::DurabilityError;
use std::collections::BTreeMap;
use std::fmt;
use std::fs::{File, OpenOptions};
use std::io::{self, Read, Seek, SeekFrom, Write};
use std::path::Path;
use std::sync::{Condvar, Mutex, MutexGuard};
use std::thread::Thread;

/// The journal file's magic.
pub const JOURNAL_MAGIC: &[u8; 8] = b"TXNJRNL\0";
/// The journal format this build writes and reads. Version 2
/// (`WBJ-FR-001`, ADR-0063) adds a leading kind byte per entry — `0` a
/// `TransactionOp` batch (every entry before this round), `1` a
/// `WriteOp` batch (`Request::WriteBatch`'s atomic mode,
/// `Memory`/`Entity`/`Relation` only). Strict version match on open, as
/// before this round — a version-1 journal left over from an older
/// build is refused, not silently upgraded; the journal is expected to
/// be small and frequently checkpointed, unlike a long-lived blob.
pub const JOURNAL_FORMAT_VERSION: u32 = 2;
/// After an append leaves the journal larger than this, the adapter
/// checkpoints: flushes the store, then truncates the journal
/// (`JRN-FR-004`). A constant chosen without measurement; the journaled
/// bench row in `benches/server.rs` is where it gets revisited.
pub const JOURNAL_CHECKPOINT_BYTES: u64 = 1 << 20;

const HEADER_LEN: u64 = 12;

/// Version-2 entry kind bytes (`WBJ-FR-001`, ADR-0063).
const KIND_TRANSACTION: u8 = 0;
const KIND_WRITE: u8 = 1;

/// One journal entry about to be appended — borrowed, chosen by the
/// caller's own typed [`CommitGroup::commit`]/[`CommitGroup::
/// commit_write`] (`WBJ-FR-001`, ADR-0063).
pub(crate) enum JournalEntry<'a> {
    Transaction(&'a [TransactionOp]),
    Write(&'a [WriteOp]),
}

/// One journal entry replayed from disk, oldest first (`WBJ-FR-001`,
/// ADR-0063): `Transaction` (kind `0`, every entry before this round —
/// `Request::Transaction`'s field-update batches, every adapter);
/// `Write` (kind `1` — `Request::WriteBatch`'s runtime writes,
/// `Memory`/`Entity`/`Relation`'s atomic mode only).
pub(crate) enum JournaledBatch {
    Transaction(Vec<TransactionOp>),
    Write(Vec<WriteOp>),
}

/// Everything that can go wrong opening, appending to, or replaying a
/// batch journal.
#[derive(Debug)]
pub enum JournalError {
    Io(io::Error),
    /// Not a batch journal, an unknown format version, an entry that is
    /// complete but does not decode, or a batch too large to frame.
    Format(String),
    Codec(bincode::Error),
    /// A journaled batch could not be re-applied on open — impossible
    /// for a batch the adapter validated before journaling, so this
    /// means the store's files are not the ones the journal belongs to.
    Replay {
        batch: usize,
        index: usize,
        code: ErrorCode,
    },
    /// The store's own flush failed during the checkpoint that precedes
    /// truncation on open.
    Durability(DurabilityError),
}

impl fmt::Display for JournalError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            JournalError::Io(e) => write!(f, "batch journal I/O: {e}"),
            JournalError::Format(what) => write!(f, "batch journal format: {what}"),
            JournalError::Codec(e) => write!(f, "batch journal encoding: {e}"),
            JournalError::Replay { batch, index, code } => write!(
                f,
                "batch journal replay: batch {batch}, operation {index} failed with {code:?}"
            ),
            JournalError::Durability(e) => write!(f, "batch journal checkpoint: {e}"),
        }
    }
}

impl std::error::Error for JournalError {
    fn source(&self) -> Option<&(dyn std::error::Error + 'static)> {
        match self {
            JournalError::Io(e) => Some(e),
            JournalError::Codec(e) => Some(e),
            JournalError::Durability(e) => Some(e),
            JournalError::Format(_) | JournalError::Replay { .. } => None,
        }
    }
}

impl From<io::Error> for JournalError {
    fn from(e: io::Error) -> Self {
        JournalError::Io(e)
    }
}

impl From<bincode::Error> for JournalError {
    fn from(e: bincode::Error) -> Self {
        JournalError::Codec(e)
    }
}

impl From<DurabilityError> for JournalError {
    fn from(e: DurabilityError) -> Self {
        JournalError::Durability(e)
    }
}

/// What a checkpoint needs from the store an adapter holds exclusively:
/// force every slot write so far to disk, so the journal entries before
/// it can be dropped (`JRN-FR-004`). Implemented for exactly the three
/// exclusive store types the adapters reach through `with_exclusive` —
/// a server-layer trait, so no storage module changes.
pub trait CheckpointFlush {
    fn checkpoint_flush(&self) -> Result<(), DurabilityError>;
}

impl CheckpointFlush for crate::durability::mmap_store::MmapAgeStore {
    fn checkpoint_flush(&self) -> Result<(), DurabilityError> {
        self.flush()
    }
}

#[cfg(feature = "research")]
impl CheckpointFlush for crate::generic::order_customer::OrderProductionStack {
    fn checkpoint_flush(&self) -> Result<(), DurabilityError> {
        crate::generic::store::Flush::flush(self)
    }
}

#[cfg(feature = "research")]
impl CheckpointFlush for crate::generic_spike::employee_impl::EmployeeProductionStack {
    fn checkpoint_flush(&self) -> Result<(), DurabilityError> {
        crate::generic::store::Flush::flush(self)
    }
}

/// `RMD-FR-007` — not `research`-gated, matching `Reminder`'s own
/// front-door status (`ADR-0036`): `ReminderProductionStack` is
/// `GenericMmapStore` under an `Ordered` wrap since `ADR-0080` (no
/// relation, so no `Symmetric`/`Reversed` layer), and `Ordered`
/// forwards `Flush` to the core.
impl CheckpointFlush for crate::generic::reminder::ReminderProductionStack {
    fn checkpoint_flush(&self) -> Result<(), DurabilityError> {
        crate::generic::store::Flush::flush(self)
    }
}

/// `REL-FR-004` — front-door like `Reminder` (`ADR-0058`):
/// `RelationProductionStack` is `GenericMmapStore` directly.
impl CheckpointFlush for crate::generic::relation::RelationProductionStack {
    fn checkpoint_flush(&self) -> Result<(), DurabilityError> {
        crate::generic::store::Flush::flush(self)
    }
}

/// `MEM-FR-006` — front-door like `Reminder` (`ADR-0048`):
/// `MemoryProductionStack` is `GenericMmapStore` directly.
impl CheckpointFlush for crate::generic::memory::MemoryProductionStack {
    fn checkpoint_flush(&self) -> Result<(), DurabilityError> {
        crate::generic::store::Flush::flush(self)
    }
}

/// `ENT-FR-006` — not `research`-gated, matching `Reminder`'s own
/// front-door status (`ADR-0037`): `EntityProductionStack` is
/// `NameIndex<MultiSymmetric<GenericMmapStore<..>, ..>, ..>` (ADR-0039,
/// ADR-0040), every layer of which forwards `Flush` generically.
impl CheckpointFlush for crate::generic::entity::EntityProductionStack {
    fn checkpoint_flush(&self) -> Result<(), DurabilityError> {
        crate::generic::store::Flush::flush(self)
    }
}

/// One adapter's journal file, held open for appends. See the module
/// docs for the format and the discipline.
pub(crate) struct BatchJournal {
    file: File,
    len: u64,
}

impl BatchJournal {
    /// Open (or create) the journal at `path` and return every complete
    /// entry in it, oldest first, for the caller to replay. A torn tail
    /// is dropped and truncated away; a bad magic or version, or a
    /// complete entry that does not decode, is an error (`JRN-FR-003`).
    pub(crate) fn open(path: &Path) -> Result<(Self, Vec<JournaledBatch>), JournalError> {
        let mut file = OpenOptions::new()
            .read(true)
            .write(true)
            .create(true)
            .truncate(false)
            .open(path)?;
        let mut bytes = Vec::new();
        file.read_to_end(&mut bytes)?;

        if bytes.is_empty() {
            file.write_all(JOURNAL_MAGIC)?;
            file.write_all(&JOURNAL_FORMAT_VERSION.to_le_bytes())?;
            file.sync_all()?;
            // `RVL-FR-003` (ADR-0112): the new file's directory entry too,
            // so the journal itself survives a power loss (`ADR-0092`).
            crate::durability::sync_parent_dir(path)?;
            return Ok((
                Self {
                    file,
                    len: HEADER_LEN,
                },
                Vec::new(),
            ));
        }
        if bytes.len() < HEADER_LEN as usize || &bytes[..8] != JOURNAL_MAGIC {
            return Err(JournalError::Format("not a batch journal".into()));
        }
        let version = u32::from_le_bytes([bytes[8], bytes[9], bytes[10], bytes[11]]);
        if version != JOURNAL_FORMAT_VERSION {
            return Err(JournalError::Format(format!(
                "format version {version}, this build reads {JOURNAL_FORMAT_VERSION}"
            )));
        }

        let mut entries = Vec::new();
        let mut pos = HEADER_LEN as usize;
        // Stops at the first incomplete entry — a torn tail, or exactly the
        // end of the file. Each entry: `[kind: u8][len: u32 LE][payload]`.
        while let Some(&kind) = bytes.get(pos) {
            let Some(len_bytes) = bytes.get(pos + 1..pos + 5) else {
                break; // torn tail: the kind byte landed, the length did not
            };
            let len = u32::from_le_bytes([len_bytes[0], len_bytes[1], len_bytes[2], len_bytes[3]])
                as usize;
            let Some(payload) = bytes.get(pos + 5..pos + 5 + len) else {
                break; // torn tail: the length prefix landed, the payload did not
            };
            let batch = match kind {
                KIND_TRANSACTION => {
                    let ops: Vec<TransactionOp> = crate::codec::decode(payload).map_err(|e| {
                        JournalError::Format(format!("entry at byte {pos} does not decode: {e}"))
                    })?;
                    JournaledBatch::Transaction(ops)
                }
                KIND_WRITE => {
                    let ops: Vec<WriteOp> = crate::codec::decode(payload).map_err(|e| {
                        JournalError::Format(format!("entry at byte {pos} does not decode: {e}"))
                    })?;
                    JournaledBatch::Write(ops)
                }
                other => {
                    return Err(JournalError::Format(format!(
                        "entry at byte {pos} has unknown kind {other}"
                    )))
                }
            };
            entries.push(batch);
            pos += 5 + len;
        }
        let complete = pos as u64;
        if complete != bytes.len() as u64 {
            file.set_len(complete)?;
            file.sync_all()?;
        }
        file.seek(SeekFrom::Start(complete))?;
        Ok((
            Self {
                file,
                len: complete,
            },
            entries,
        ))
    }

    /// Append one batch and force it to disk (`JRN-FR-002`): when this
    /// returns `Ok`, the batch is durable. Tests write journals with it;
    /// a journaled adapter goes through [`CommitGroup`], which separates
    /// the two halves.
    #[cfg(test)]
    pub(crate) fn append(&mut self, batch: &[TransactionOp]) -> Result<(), JournalError> {
        self.append_unsynced(JournalEntry::Transaction(batch))?;
        self.file.sync_data()?;
        Ok(())
    }

    /// [`append`] for a decided `WriteOp` batch (`WBJ-FR-001`, ADR-0063)
    /// — tests only; a journaled adapter goes through [`CommitGroup::
    /// commit_write`].
    #[cfg(test)]
    pub(crate) fn append_write(&mut self, batch: &[WriteOp]) -> Result<(), JournalError> {
        self.append_unsynced(JournalEntry::Write(batch))?;
        self.file.sync_data()?;
        Ok(())
    }

    /// Write one entry without syncing (`GRP-FR-001`): the caller owns
    /// the `fsync` — see [`CommitGroup`]. `len` advances only if the
    /// whole entry was written. The kind byte is `entry`'s own variant
    /// (`WBJ-FR-001`).
    pub(crate) fn append_unsynced(&mut self, entry: JournalEntry<'_>) -> Result<(), JournalError> {
        let (kind, payload) = match entry {
            JournalEntry::Transaction(batch) => (KIND_TRANSACTION, crate::codec::encode(batch)?),
            JournalEntry::Write(batch) => (KIND_WRITE, crate::codec::encode(batch)?),
        };
        let len = u32::try_from(payload.len())
            .map_err(|_| JournalError::Format("batch too large to journal".into()))?;
        self.file.write_all(&[kind])?;
        self.file.write_all(&len.to_le_bytes())?;
        self.file.write_all(&payload)?;
        self.len += 5 + payload.len() as u64;
        Ok(())
    }

    /// A second handle on the same open file description, so a
    /// [`CommitGroup`] leader can `sync_data` without holding the lock
    /// that guards appends.
    fn sync_handle(&self) -> Result<File, JournalError> {
        Ok(self.file.try_clone()?)
    }

    /// Whether the journal has grown past [`JOURNAL_CHECKPOINT_BYTES`].
    pub(crate) fn needs_checkpoint(&self) -> bool {
        self.len > JOURNAL_CHECKPOINT_BYTES
    }

    /// Drop every entry — only after the store's own files are known
    /// durable (see [`CheckpointFlush`]).
    pub(crate) fn truncate(&mut self) -> Result<(), JournalError> {
        self.file.set_len(HEADER_LEN)?;
        self.file.seek(SeekFrom::Start(HEADER_LEN))?;
        self.file.sync_all()?;
        self.len = HEADER_LEN;
        Ok(())
    }

    /// The journal's size in bytes, header included.
    #[cfg(test)]
    pub(crate) fn len_bytes(&self) -> u64 {
        self.len
    }
}

/// A batch's turn to apply, handed to the closure [`CommitGroup::commit`]
/// runs under the caller's own exclusive section.
#[derive(Debug, Clone, Copy)]
pub(crate) struct Turn {
    /// `GRP-FR-004`: whether the journal is past
    /// [`JOURNAL_CHECKPOINT_BYTES`] *and* this batch is the last one
    /// appended — the only moment a checkpoint is safe, since every
    /// journaled entry is then applied. The closure answers by flushing
    /// the store and returning `true`; the group truncates.
    pub(crate) checkpoint_due: bool,
    /// `ADR-0072`'s `MVCC2-FR-010`: how many journal entries have been
    /// appended since the last truncate, as of this batch's turn —
    /// including this batch itself. A caller flushing MVCC history right
    /// before a checkpoint's truncate records this as
    /// `journal_entries_reflected`, so a later open knows how many
    /// leading replayed entries the flush already covers, whether or not
    /// the truncate that was meant to follow actually completed.
    pub(crate) journal_entries: u64,
}

/// Why [`CommitGroup::commit`] failed: the journal (the batch was never
/// applied) or the caller's own apply closure.
#[derive(Debug)]
pub(crate) enum CommitError<E> {
    Journal(JournalError),
    Apply(E),
}

impl<E: fmt::Display> fmt::Display for CommitError<E> {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            CommitError::Journal(e) => write!(f, "journal: {e}"),
            CommitError::Apply(e) => write!(f, "apply: {e}"),
        }
    }
}

impl<E> From<JournalError> for CommitError<E> {
    fn from(e: JournalError) -> Self {
        CommitError::Journal(e)
    }
}

#[cfg(test)]
type SyncHook = Box<dyn Fn() -> io::Result<()> + Send + Sync>;

struct GroupState {
    journal: BatchJournal,
    /// The sequence of the last entry written.
    appended: u64,
    /// `ADR-0072`'s `MVCC2-FR-010`: entries appended since the journal
    /// was last truncated — unlike `appended`, this resets to `0` on a
    /// successful [`BatchJournal::truncate`], so a caller flushing MVCC
    /// history right before a checkpoint's truncate can record exactly
    /// how many journal entries are reflected in that flush (`Turn::
    /// journal_entries`), regardless of the journal's lifetime total.
    since_checkpoint: u64,
    /// Every sequence `<= durable` has had an `fsync` return after its
    /// bytes were written (monotone: a leader covers every earlier entry).
    durable: u64,
    /// Every sequence in `(durable_at_failure, failed_upto]` had its
    /// `fsync` fail; those batches are refused, never applied.
    failed_upto: u64,
    /// A leader is inside `sync_data`.
    syncing: bool,
    /// The sequence whose turn it is to apply (`GRP-FR-003`).
    next_apply: u64,
    /// Batches parked waiting for their turn, by sequence — each is
    /// unparked individually when its turn comes, so releasing a turn
    /// wakes exactly one thread rather than every waiter.
    turn_waiters: BTreeMap<u64, Thread>,
}

/// The group-commit discipline behind a journaled adapter (`SERVER-001`
/// FR-027, ADR-0026, `docs/design/SERVER-JOURNAL-GROUP-COMMIT-DESIGN.md`):
/// append under this group's own mutex, taking a sequence; wait until
/// the journal is durable through that sequence — the first waiter that
/// finds no `fsync` in flight becomes the *leader* and `sync_data`s once
/// for everyone appended so far, the rest wait on a condvar
/// (`GRP-FR-002`); then apply in sequence order through a turn gate
/// (`GRP-FR-003`), so replay order equals apply order; then checkpoint
/// only when nothing later has been appended (`GRP-FR-004`). No timer,
/// no delay, no thread: a lone batch is its own leader and pays exactly
/// one `fsync`. The store's exclusive section is the caller's and is
/// held only inside the apply closure — never across an `fsync`
/// (`GRP-FR-001`).
pub(crate) struct CommitGroup {
    sync_handle: File,
    state: Mutex<GroupState>,
    /// Signalled when `durable` or `failed_upto` advances.
    durable: Condvar,
    #[cfg(test)]
    sync_hook: Mutex<Option<SyncHook>>,
}

/// Releases a batch's apply turn — on the normal path and on unwind, so
/// a panic inside an apply closure cannot strand every later batch.
struct TurnGuard<'a> {
    group: &'a CommitGroup,
}

impl Drop for TurnGuard<'_> {
    fn drop(&mut self) {
        if let Ok(mut state) = self.group.state.lock() {
            state.next_apply += 1;
            let next = state.next_apply;
            if let Some(waiter) = state.turn_waiters.remove(&next) {
                waiter.unpark();
            }
        }
    }
}

fn poisoned() -> JournalError {
    JournalError::Io(io::Error::other("commit group lock poisoned"))
}

fn sync_failed() -> JournalError {
    JournalError::Io(io::Error::other(
        "the fsync covering this batch failed; nothing was applied",
    ))
}

impl CommitGroup {
    /// [`BatchJournal::open`] plus the group state. Replay is the
    /// caller's (it knows how to apply an operation); once every replayed
    /// batch is applied and the store flushed, [`CommitGroup::truncate`].
    pub(crate) fn open(path: &Path) -> Result<(Self, Vec<JournaledBatch>), JournalError> {
        let (journal, batches) = BatchJournal::open(path)?;
        let sync_handle = journal.sync_handle()?;
        Ok((
            Self {
                sync_handle,
                state: Mutex::new(GroupState {
                    journal,
                    appended: 0,
                    since_checkpoint: 0,
                    durable: 0,
                    failed_upto: 0,
                    syncing: false,
                    next_apply: 1,
                    turn_waiters: BTreeMap::new(),
                }),
                durable: Condvar::new(),
                #[cfg(test)]
                sync_hook: Mutex::new(None),
            },
            batches,
        ))
    }

    /// Drop every entry — only after the store's own files are known
    /// durable, and only from a context where nothing is in flight (the
    /// replay at open).
    pub(crate) fn truncate(&self) -> Result<(), JournalError> {
        self.lock()?.journal.truncate()
    }

    fn lock(&self) -> Result<MutexGuard<'_, GroupState>, JournalError> {
        self.state.lock().map_err(|_| poisoned())
    }

    /// The test hook, run by a leader after it takes the lead and before
    /// it reads how far to sync — the slot a commit delay would occupy
    /// (ADR-0026 option (b), declined). `Ok(())` outside tests.
    fn pre_sync_hook(&self) -> io::Result<()> {
        #[cfg(test)]
        {
            if let Ok(hook) = self.sync_hook.lock() {
                if let Some(hook) = hook.as_ref() {
                    hook()?;
                }
            }
        }
        Ok(())
    }

    /// `GRP-FR-001`–`004`: append → group `fsync` → ordered apply →
    /// quiescent checkpoint. `apply` runs under the caller's exclusive
    /// section with this batch's [`Turn`] and returns whether it flushed
    /// the store (only meaningful when `checkpoint_due`); the group then
    /// truncates the journal if nothing was appended meanwhile. A journal
    /// or `fsync` failure is [`CommitError::Journal`] with `apply` never
    /// called (`GRP-FR-005`); an `fsync` failure fails every batch the
    /// leader was covering, and the next batch tries again.
    pub(crate) fn commit<E>(
        &self,
        batch: &[TransactionOp],
        apply: impl FnOnce(Turn) -> Result<bool, E>,
    ) -> Result<(), CommitError<E>> {
        self.commit_entry(JournalEntry::Transaction(batch), apply)
    }

    /// [`commit`] for `Request::WriteBatch`'s atomic mode (`WBJ-FR-002`,
    /// ADR-0063) — `Memory`/`Entity`/`Relation` only. `ops` is the raw,
    /// already-`prepare_write`-validated wire batch; replay re-runs the
    /// identical `prepare_write`/`apply_prepared` pair in order, which is
    /// what makes a `ReplaceIf` guard inside it safe to re-evaluate on
    /// replay — a crash before `apply` here means the store is exactly
    /// the pre-apply state the guard was (or will be) evaluated against,
    /// so replay reconstructs the identical decision, not a stale one.
    pub(crate) fn commit_write<E>(
        &self,
        ops: &[WriteOp],
        apply: impl FnOnce(Turn) -> Result<bool, E>,
    ) -> Result<(), CommitError<E>> {
        self.commit_entry(JournalEntry::Write(ops), apply)
    }

    fn commit_entry<E>(
        &self,
        entry: JournalEntry<'_>,
        apply: impl FnOnce(Turn) -> Result<bool, E>,
    ) -> Result<(), CommitError<E>> {
        let mut state = self.lock().map_err(CommitError::Journal)?;
        state
            .journal
            .append_unsynced(entry)
            .map_err(CommitError::Journal)?;
        state.appended += 1;
        state.since_checkpoint += 1;
        let seq = state.appended;

        // `GRP-FR-002`: wait for durability through `seq`, leading when
        // no sync is in flight. The failure check comes first: a later
        // successful sync must not rescue a batch whose own sync failed.
        let durable = loop {
            if state.failed_upto >= seq {
                break Err(sync_failed());
            }
            if state.durable >= seq {
                break Ok(());
            }
            if state.syncing {
                state = self.durable.wait(state).map_err(|_| poisoned())?;
                continue;
            }
            state.syncing = true;
            drop(state);
            let hooked = self.pre_sync_hook();
            // Read how far to sync as late as possible: everything appended
            // before the `sync_data` starts is covered by it.
            state = self.lock()?;
            let upto = state.appended;
            drop(state);
            let synced = hooked.and_then(|()| self.sync_handle.sync_data());
            state = self.lock()?;
            match synced {
                Ok(()) => state.durable = state.durable.max(upto),
                Err(_) => state.failed_upto = state.failed_upto.max(upto),
            }
            state.syncing = false;
            self.durable.notify_all();
        }
        .map_err(CommitError::Journal);

        // `GRP-FR-003`: the turn gate — taken on failure too, so the
        // sequence after a refused batch still gets its turn.
        // Parked per sequence and unparked by the exact predecessor: a
        // condvar here would wake every waiter on every release — a herd
        // quadratic in the group size, measured as the difference between
        // ~3k and ~30k batches/s at 32 connections.
        while state.next_apply != seq {
            state.turn_waiters.insert(seq, std::thread::current());
            drop(state);
            std::thread::park();
            state = self.lock().map_err(CommitError::Journal)?;
        }
        let checkpoint_due =
            durable.is_ok() && state.journal.needs_checkpoint() && state.appended == seq;
        let journal_entries = state.since_checkpoint;
        drop(state);
        let _turn = TurnGuard { group: self };

        durable?;
        let flushed = apply(Turn {
            checkpoint_due,
            journal_entries,
        })
        .map_err(CommitError::Apply)?;
        if flushed {
            // `GRP-FR-004`: truncate only if still quiescent — an entry
            // appended during the apply is not yet applied.
            let mut state = self.lock().map_err(CommitError::Journal)?;
            if state.appended == seq {
                let _ = state.journal.truncate();
                state.since_checkpoint = 0;
            }
        }
        Ok(())
    }

    /// Run `hook` before every `sync_data` (a returned error is the
    /// sync's failure) — the design's held-leader, grouping, and failure
    /// criteria are driven through it.
    #[cfg(test)]
    pub(crate) fn set_sync_hook(&self, hook: SyncHook) {
        if let Ok(mut slot) = self.sync_hook.lock() {
            *slot = Some(hook);
        }
    }

    /// The sequence of the last entry appended.
    #[cfg(test)]
    pub(crate) fn appended(&self) -> u64 {
        self.state.lock().map(|s| s.appended).unwrap_or(0)
    }

    /// The journal's size in bytes, header included.
    #[cfg(test)]
    pub(crate) fn len_bytes(&self) -> u64 {
        self.state
            .lock()
            .map(|s| s.journal.len_bytes())
            .unwrap_or(0)
    }

    /// `ADR-0072`'s `MVCC2-FR-010`: how many entries have been appended
    /// since the last successful truncate, right now — the count an
    /// adapter records as `journal_entries_reflected` when it flushes
    /// MVCC history for a reason other than a live commit's own
    /// checkpoint (`MvccState::open`'s own baseline-seeding flush, which
    /// has no [`Turn`] to read `journal_entries` from).
    pub(crate) fn entries_since_checkpoint(&self) -> u64 {
        self.state.lock().map(|s| s.since_checkpoint).unwrap_or(0)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::server::protocol::ScanValue;
    use crate::test_support::fresh_temp_dir;
    use uuid::Uuid;

    fn op(id: u128, age: u32) -> TransactionOp {
        TransactionOp {
            id: Uuid::from_u128(id),
            field: 1,
            value: ScanValue::U32(age),
        }
    }

    fn write_op(id: u128) -> WriteOp {
        WriteOp::Delete {
            id: Uuid::from_u128(id),
        }
    }

    /// `WBJ-FR-001` (ADR-0063): a kind-`1` (`WriteOp`) entry round-trips
    /// distinctly from a kind-`0` (`TransactionOp`) entry in the same
    /// journal, in append order, each decoded as its own variant.
    #[test]
    fn a_write_batch_entry_round_trips_alongside_a_transaction_entry() {
        let dir = fresh_temp_dir("journal_write_kind").unwrap();
        let path = dir.join("txn.journal");
        {
            let (mut journal, _) = BatchJournal::open(&path).unwrap();
            journal.append(&[op(1, 30)]).unwrap();
            journal.append_write(&[write_op(2), write_op(3)]).unwrap();
        }
        let (_, entries) = BatchJournal::open(&path).unwrap();
        assert_eq!(entries.len(), 2);
        assert!(matches!(&entries[0], JournaledBatch::Transaction(ops) if ops.len() == 1));
        assert!(matches!(&entries[1], JournaledBatch::Write(ops) if ops.len() == 2));
    }

    #[test]
    fn a_fresh_journal_is_a_header_and_replays_nothing() {
        let dir = fresh_temp_dir("journal_fresh").unwrap();
        let path = dir.join("txn.journal");
        let (journal, entries) = BatchJournal::open(&path).unwrap();
        assert!(entries.is_empty());
        assert_eq!(journal.len_bytes(), HEADER_LEN);
        let bytes = std::fs::read(&path).unwrap();
        assert_eq!(&bytes[..8], JOURNAL_MAGIC);
        assert_eq!(&bytes[8..12], &JOURNAL_FORMAT_VERSION.to_le_bytes());
        assert_eq!(bytes.len(), HEADER_LEN as usize);
    }

    #[test]
    fn appended_batches_replay_in_order_and_a_torn_tail_is_dropped() {
        let dir = fresh_temp_dir("journal_replay").unwrap();
        let path = dir.join("txn.journal");
        {
            let (mut journal, _) = BatchJournal::open(&path).unwrap();
            journal.append(&[op(1, 30), op(2, 40)]).unwrap();
            journal.append(&[op(3, 50)]).unwrap();
        }
        // A crash mid-append: a length prefix promising more than landed.
        {
            let mut file = OpenOptions::new().append(true).open(&path).unwrap();
            file.write_all(&[0x40, 0x00, 0x00, 0x00, 0xaa, 0xbb])
                .unwrap();
        }
        let torn_len = std::fs::metadata(&path).unwrap().len();
        let (journal, entries) = BatchJournal::open(&path).unwrap();
        assert_eq!(entries.len(), 2);
        let JournaledBatch::Transaction(entry0) = &entries[0] else {
            panic!("expected a Transaction batch");
        };
        let JournaledBatch::Transaction(entry1) = &entries[1] else {
            panic!("expected a Transaction batch");
        };
        assert_eq!(entry0.len(), 2);
        assert_eq!(entry0[1].id, Uuid::from_u128(2));
        assert_eq!(entry1[0].value, ScanValue::U32(50));
        // The torn tail was truncated away.
        assert!(journal.len_bytes() < torn_len);
        assert_eq!(std::fs::metadata(&path).unwrap().len(), journal.len_bytes());
    }

    #[test]
    fn a_foreign_file_and_an_unknown_version_are_format_errors() {
        let dir = fresh_temp_dir("journal_format").unwrap();
        let foreign = dir.join("foreign.journal");
        std::fs::write(&foreign, b"DOGBLOB\0\x02\x00\x00\x00").unwrap();
        assert!(matches!(
            BatchJournal::open(&foreign).map(|_| ()),
            Err(JournalError::Format(_))
        ));
        let future = dir.join("future.journal");
        let mut bytes = JOURNAL_MAGIC.to_vec();
        bytes.extend_from_slice(&3u32.to_le_bytes());
        std::fs::write(&future, bytes).unwrap();
        assert!(matches!(
            BatchJournal::open(&future).map(|_| ()),
            Err(JournalError::Format(_))
        ));
    }

    fn open_group(label: &str) -> (CommitGroup, std::path::PathBuf) {
        let dir = fresh_temp_dir(label).unwrap();
        let path = dir.join("txn.journal");
        let (group, entries) = CommitGroup::open(&path).unwrap();
        assert!(entries.is_empty());
        (group, path)
    }

    /// A hook that blocks its first caller until `release` is dropped or
    /// sent to, counts every call, and (optionally) fails the first call.
    fn holding_hook(
        fail_first: bool,
    ) -> (
        SyncHook,
        std::sync::mpsc::Receiver<()>,
        std::sync::mpsc::Sender<()>,
        std::sync::Arc<std::sync::atomic::AtomicU64>,
    ) {
        use std::sync::atomic::{AtomicBool, AtomicU64, Ordering};
        use std::sync::{mpsc, Arc, Mutex};
        let (entered_tx, entered_rx) = mpsc::channel();
        let (release_tx, release_rx) = mpsc::channel::<()>();
        let release_rx = Mutex::new(release_rx);
        let first = AtomicBool::new(true);
        let count = Arc::new(AtomicU64::new(0));
        let count_in = Arc::clone(&count);
        let hook: SyncHook = Box::new(move || {
            count_in.fetch_add(1, Ordering::SeqCst);
            if first.swap(false, Ordering::SeqCst) {
                let _ = entered_tx.send(());
                if let Ok(rx) = release_rx.lock() {
                    let _ = rx.recv();
                }
                if fail_first {
                    return Err(io::Error::other("disk on fire"));
                }
            }
            Ok(())
        });
        (hook, entered_rx, release_tx, count)
    }

    fn wait_until(what: &str, mut cond: impl FnMut() -> bool) {
        for _ in 0..5_000 {
            if cond() {
                return;
            }
            std::thread::sleep(std::time::Duration::from_millis(1));
        }
        panic!("timed out waiting for {what}");
    }

    /// `GRP-FR-003` (design criterion 3): under many concurrent
    /// committers, the order applies ran in is exactly the order the
    /// journal will replay them in.
    #[test]
    fn commits_apply_in_journal_order_across_threads() {
        use std::sync::{Arc, Mutex};
        let (group, path) = open_group("journal_group_order");
        let group = Arc::new(group);
        let applied = Arc::new(Mutex::new(Vec::new()));
        let threads: Vec<_> = (0..8u32)
            .map(|t| {
                let group = Arc::clone(&group);
                let applied = Arc::clone(&applied);
                std::thread::spawn(move || {
                    for i in 0..40u32 {
                        let v = t * 1_000 + i;
                        group
                            .commit(&[op(v as u128, v)], |_turn| {
                                applied.lock().unwrap().push(v);
                                Ok::<bool, ()>(false)
                            })
                            .unwrap();
                    }
                })
            })
            .collect();
        for t in threads {
            t.join().unwrap();
        }
        drop(group);
        let (_, entries) = BatchJournal::open(&path).unwrap();
        let journaled: Vec<u32> = entries
            .iter()
            .map(|batch| {
                let JournaledBatch::Transaction(ops) = batch else {
                    unreachable!()
                };
                match ops[0].value {
                    ScanValue::U32(v) => v,
                    _ => unreachable!(),
                }
            })
            .collect();
        assert_eq!(journaled.len(), 320);
        assert_eq!(*applied.lock().unwrap(), journaled);
    }

    /// `GRP-FR-002` (design criterion 2): while one leader is held before
    /// its `sync_data`, two more batches append; the leader's single sync
    /// covers all three.
    #[test]
    fn a_held_leader_syncs_once_for_every_batch_appended_meanwhile() {
        use std::sync::atomic::{AtomicU64, Ordering};
        use std::sync::Arc;
        let (group, _path) = open_group("journal_group_held");
        let group = Arc::new(group);
        let (hook, entered, release, syncs) = holding_hook(false);
        group.set_sync_hook(hook);
        let applied = Arc::new(AtomicU64::new(0));
        let spawn = |v: u32| {
            let group = Arc::clone(&group);
            let applied = Arc::clone(&applied);
            std::thread::spawn(move || {
                group.commit(&[op(v as u128, v)], |_turn| {
                    applied.fetch_add(1, Ordering::SeqCst);
                    Ok::<bool, ()>(false)
                })
            })
        };
        let a = spawn(1);
        entered.recv().unwrap();
        let b = spawn(2);
        let c = spawn(3);
        wait_until("both followers to append", || group.appended() == 3);
        assert_eq!(
            applied.load(Ordering::SeqCst),
            0,
            "nothing applies before the sync"
        );
        release.send(()).unwrap();
        for t in [a, b, c] {
            t.join().unwrap().unwrap();
        }
        assert_eq!(syncs.load(Ordering::SeqCst), 1, "three batches, one sync");
        assert_eq!(applied.load(Ordering::SeqCst), 3);
    }

    /// `GRP-FR-005` (design criterion 5): a failed sync refuses every
    /// batch it covered with nothing applied; the next batch syncs again
    /// and succeeds — and the turn gate advanced past the refused ones.
    #[test]
    fn a_failed_sync_refuses_its_group_and_the_next_batch_succeeds() {
        use std::sync::atomic::{AtomicU64, Ordering};
        use std::sync::Arc;
        let (group, _path) = open_group("journal_group_failed");
        let group = Arc::new(group);
        let (hook, entered, release, syncs) = holding_hook(true);
        group.set_sync_hook(hook);
        let applied = Arc::new(AtomicU64::new(0));
        let spawn = |v: u32| {
            let group = Arc::clone(&group);
            let applied = Arc::clone(&applied);
            std::thread::spawn(move || {
                group.commit(&[op(v as u128, v)], |_turn| {
                    applied.fetch_add(1, Ordering::SeqCst);
                    Ok::<bool, ()>(false)
                })
            })
        };
        let a = spawn(1);
        entered.recv().unwrap();
        let b = spawn(2);
        let c = spawn(3);
        wait_until("both followers to append", || group.appended() == 3);
        release.send(()).unwrap();
        for t in [a, b, c] {
            assert!(matches!(
                t.join().unwrap(),
                Err(CommitError::Journal(JournalError::Io(_)))
            ));
        }
        assert_eq!(applied.load(Ordering::SeqCst), 0);
        group
            .commit(&[op(4, 4)], |_turn| {
                applied.fetch_add(1, Ordering::SeqCst);
                Ok::<bool, ()>(false)
            })
            .unwrap();
        assert_eq!(applied.load(Ordering::SeqCst), 1);
        assert_eq!(syncs.load(Ordering::SeqCst), 2);
    }

    /// `GRP-FR-004` (design criterion 4): a batch whose turn finds the
    /// journal past the threshold is offered the checkpoint, but if
    /// another batch appended during its apply the truncation is
    /// withheld; the later batch, quiescent at its own turn, checkpoints.
    #[test]
    fn a_checkpoint_is_deferred_until_no_later_batch_is_appended() {
        use std::sync::atomic::{AtomicBool, Ordering};
        use std::sync::{mpsc, Arc};
        let (group, _path) = open_group("journal_group_checkpoint");
        let group = Arc::new(group);
        let big: Vec<TransactionOp> = (0..4096).map(|i| op(i as u128, i as u32)).collect();
        while group.len_bytes() <= JOURNAL_CHECKPOINT_BYTES {
            group.commit(&big, |_turn| Ok::<bool, ()>(false)).unwrap();
        }
        let past = group.len_bytes();
        let n = group.appended();

        let (entered_tx, entered_rx) = mpsc::channel();
        let (release_tx, release_rx) = mpsc::channel::<()>();
        let a_saw_due = Arc::new(AtomicBool::new(false));
        let a = {
            let group = Arc::clone(&group);
            let a_saw_due = Arc::clone(&a_saw_due);
            std::thread::spawn(move || {
                group.commit(&[op(1, 1)], |turn| {
                    a_saw_due.store(turn.checkpoint_due, Ordering::SeqCst);
                    entered_tx.send(()).unwrap();
                    release_rx.recv().unwrap();
                    Ok::<bool, ()>(true) // "flushed" — but a later append will withhold the truncate
                })
            })
        };
        entered_rx.recv().unwrap();
        let b_saw_due = Arc::new(AtomicBool::new(false));
        let (b_entered_tx, b_entered_rx) = mpsc::channel();
        let (b_release_tx, b_release_rx) = mpsc::channel::<()>();
        let b = {
            let group = Arc::clone(&group);
            let b_saw_due = Arc::clone(&b_saw_due);
            std::thread::spawn(move || {
                group.commit(&[op(2, 2)], |turn| {
                    b_saw_due.store(turn.checkpoint_due, Ordering::SeqCst);
                    b_entered_tx.send(()).unwrap();
                    b_release_rx.recv().unwrap();
                    Ok::<bool, ()>(turn.checkpoint_due)
                })
            })
        };
        wait_until("b to append", || group.appended() == n + 2);
        release_tx.send(()).unwrap();
        a.join().unwrap().unwrap();
        assert!(
            a_saw_due.load(Ordering::SeqCst),
            "a was quiescent at its turn"
        );
        // b now holds the turn inside its apply; nothing has truncated.
        b_entered_rx.recv().unwrap();
        assert!(
            group.len_bytes() > past,
            "a must not truncate: b appended during a's apply"
        );
        b_release_tx.send(()).unwrap();
        b.join().unwrap().unwrap();
        assert!(b_saw_due.load(Ordering::SeqCst));
        assert_eq!(group.len_bytes(), HEADER_LEN, "b checkpointed");
    }

    #[test]
    fn checkpoint_threshold_and_truncate() {
        let dir = fresh_temp_dir("journal_checkpoint").unwrap();
        let path = dir.join("txn.journal");
        let (mut journal, _) = BatchJournal::open(&path).unwrap();
        let big: Vec<TransactionOp> = (0..4096).map(|i| op(i as u128, i as u32)).collect();
        assert!(!journal.needs_checkpoint());
        while !journal.needs_checkpoint() {
            journal.append(&big).unwrap();
        }
        assert!(journal.len_bytes() > JOURNAL_CHECKPOINT_BYTES);
        journal.truncate().unwrap();
        assert!(!journal.needs_checkpoint());
        assert_eq!(journal.len_bytes(), HEADER_LEN);
        let (_, entries) = BatchJournal::open(&path).unwrap();
        assert!(entries.is_empty());
    }
}
