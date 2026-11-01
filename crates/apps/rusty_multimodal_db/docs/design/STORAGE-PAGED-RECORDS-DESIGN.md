# Paged Records: Record Bodies on Disk in Log-Structured Segments (Proposed)

- Status: **Proposed**, draft 3 (2026-11-01)
- Date: 2026-11-01
- Deciders: baileyrd
- Related: `ADR-0138` decision 2 (storage before features), `ADR-0139`
  (this design's decision record, which amends ADR-0138's codec clause),
  `ADR-0065`/`ADR-0070` (backup and restore), `ADR-0066` (migration to a
  fresh path), `ADR-0136` (chunked snapshots), root `docs/adr/0002`
  (dev-only oracles)
- Supersedes: nothing

## 1. Purpose and scope

The engine keeps every record body in RAM and rewrites whole-store images
to stay compact. This design moves record bodies into append-only segment
files on disk, with an in-RAM key directory, and makes compaction work one
segment at a time.

The scope is the engine (`crates/libs/storage/rusty_multimodal_db_engine`).
It also covers the `rusty_multimodal_db` server where the server touches
storage: its adapters, `Backup`, snapshots, crash tests and codec use.
Other consumers move in their own rollout PRs (§10).

## 2. Non-goals

- SQL, more than one writer process per store, or replication changes.
- Persisted derived indexes, including full-text. They get their own design
  (phase 6), and phase M decides when it starts.
- Moving edges into segments. Edge blobs and edge logs stay as they are,
  with #1355's rollback and poisoning.
- A record cache. Reads rely on the OS page cache until a gate shows
  otherwise (§12, open question 3).
- A pluggable storage port. There is one real implementation, so there is
  no trait to swap it for.

## 3. Context and terminology

What the engine does today:

- `GenericMmapStore` holds `records: HashMap<R::Id, Box<R>>`
  (`generic/mmap_store.rs:463`) and persists the set as one bincode image,
  `<path>.records`.
- Writes append to `<path>.inserts`. `open` folds that log back in,
  fingerprints every record, and rewrites the image if the set changed
  (`mmap_store.rs:659-744`). `compact()` does the same and also rewrites
  the slot file gaplessly (`:977-1023`). Both are O(records).
- Layers are built over the record set at open. `Indexed`, `Scanned`,
  `Reversed` and `MmapScanned` take `records: &[R]`. `NameIndex::new` and
  `Ordered::new` read every record back through `all_ids()` + `get()`
  (`store.rs:1788`, `:2417`).

What that costs:

- **Writes and compaction are O(store).** This is the "SQLite-class" gap in
  ADR-0138.
- **Store size is capped by RAM** wherever bodies dominate.
- **It is not remind-me's main RAM cost.** The engine `CHANGELOG.md` (0.2.0)
  gives 110 MiB at 15,000 memories, measured after the full-text rewrite.
  At about 800 B per memory, bodies are about 12 MiB (11%). Phase M (§10)
  measures every consumer before any RAM target is set.

Terms:

- **Segment:** an append-only file of entries, `<path>.seg-NNNNNN`.
- **Active segment:** the highest-numbered segment, the only one that
  accepts appends.
- **Sealed segment:** any other segment. It is synced and never written
  again.
- **Entry:** one framed `put` (a record body) or `tombstone` (a removed id).
- **Key directory:** the in-RAM map from id to the location of its newest
  `put`.
- **Live:** a `put` the key directory points at, or a tombstone that must be
  kept (§7.5). Every other entry is **dead**.
- **Slot file:** today's mmap'd fixed-width field file. It is authoritative
  for its field (§7.6).
- **Poisoned:** the heap's state after a failed sync or a failed rollback.
  Writes are refused until reopen (§8.1).
- **Deferred sync / group commit:** `LogSync::Deferred` writes plus one
  later `commit()` sync (`GroupCommit`, `store.rs:140-151`).

## 4. Requirements

- **PAGED-FR-001** Record bodies live on disk. RAM holds the key directory,
  the derived indexes and slot-file pages.
- **PAGED-FR-002** No write, open or compaction step rewrites the whole
  record set. Puts and removes are O(record). Compaction works one segment
  at a time.
- **PAGED-FR-003** After a clean close, open builds the key directory from
  hint files without reading bodies.
- **PAGED-FR-004** An I/O or checksum error on a read is never reported as
  "not found".
- **PAGED-FR-005** Durability is no weaker than today. A failed append rolls
  back or poisons, and a failed sync poisons (#1355's rule).
- **PAGED-FR-006** Old-layout stores are never migrated silently. Migration
  writes to a fresh path and leaves the source untouched.
- **PAGED-FR-007** No runtime dependency on `bincode`. bincode 1.3.3 stays
  only as a dev-dependency oracle, as root ADR-0002 allows.
- **PAGED-FR-008** Each phase leaves `main` green, and each consumer moves to
  segments in its own PR.

## 5. Considered options

| Aspect | A. B+tree page store | B. LSM tree | C. Log-structured segments |
|---|---|---|---|
| Shape | Pages, buffer pool, WAL | Memtable, sorted runs, levels | Append-only segments, key directory |
| Writes | Page plus WAL | Sequential, plus compaction amplification | One sequential append |
| New code | Large | Large | Small. #1355's insert-log writer already does framing, rollback and poisoning |
| Fit | Needs a page layer under the layers | Duplicates the in-RAM `Ordered` sort | The layers already sit on an id-to-record lookup |

**Chosen: C.** Range and order queries come from the derived `Ordered`
index, not the primary store, so A and B would duplicate work. C's limit is
that the key directory stays in RAM.

Prior art, and what this design takes from each:

- **#1355's `insert_log` writer** (`write_entry`, `roll_back`, `poison`,
  `Appender`): reused as the segment writer.
- **`journal.rs`** (CRC framing, `crc32`, a torn-tail rule): its rule is
  adopted (§8.3).
- **`rusty_journal`**: it is async on `rusty_tokio`, so it is not reused.
  Its rollover creates the new segment before syncing the old one; §8.2
  syncs the old segment first, so that a sealed segment is always durable
  before anything relies on it.
- **agent-cloud's segment log** (ADR-0138, #773): it lives outside this repo.
  This design replaces the need for it, and ac-store migrates last.

The repo holds three copies of CRC-32: the engine's `journal.rs:446`,
`rusty_journal` and the rleval gzip code. Phase 1 makes the engine copy
`pub(crate)` in `durability::crc` and uses slicing-by-8 for speed. Merging
the other two is a separate change.

## 6. Codec: bincode replaced without a format migration

This departs from ADR-0138 decision 2, which planned one combined format
migration. ADR-0139 records the amendment.

Every engine and server encoding goes through `codec.rs` (the server reaches
it through `pub(crate) use`, `lib.rs:145`). Golden vectors pin the bytes
(`BINENC-FR-004`). A first-party `serde` serializer and deserializer for the
same fixint little-endian format changes no file and no wire frame.

It must match bincode 1.3.3, with the crate's options, both on the bytes it
writes and on the inputs it accepts:

- `is_human_readable()` is `false`, `u128` is 16 bytes, and `char` is UTF-8.
- A sequence or map of unknown length is an encode error.
- Decoding rejects:
  - a `bool` of 2;
  - an `Option` tag of 2;
  - an out-of-range enum index;
  - invalid UTF-8 or an invalid `char`;
  - trailing bytes.
- `deserialize_any` is unsupported.
- Allocations are bounded by the remaining input.
- Struct sequences keep exact lengths, so `#[serde(default)]` fields still
  decode.

Other effects:

- `codec::Error` replaces `bincode::Error` in the engine's public API.
- The server drops `bincode = "1"` (`Cargo.toml:369`).
- The `deny.toml` entry for RUSTSEC-2025-0141 stays, because `ignore` is per
  advisory. Its reason becomes "dev-only oracle".
- The root ADR-0009 bincode row is updated.

## 7. Proposed shape

### 7.1 Where it sits

`GenericMmapStore` keeps its type and API. Internally its bodies are an enum:

```rust
enum Bodies<R: Record> {
    /// Today's layout: map + `<path>.records` + `<path>.inserts`.
    Map(MapBodies<R>),
    /// This design.
    Segments(SegmentHeap<R>),
}
```

Which one a store uses is decided by how it is opened (§7.2), never by a
type parameter. `SegmentHeap` is `pub(crate)`. `Map` is removed in the
release after the default flips (phase 5).

### 7.2 Opening a segment store

```rust
pub struct HeapOptions {
    /// Seal the active segment at this size. Default 64 MiB.
    /// Valid range 64 KiB ..= 2 GiB; not persisted, may change between opens.
    pub segment_bytes: u32,
}

impl GenericMmapStore<R, I, S> {
    pub fn create_segments(path: &Path, lock: &DirLock, opts: HeapOptions)
        -> Result<Self, DurabilityError>;
    pub fn open_segments(path: &Path, lock: &DirLock, opts: HeapOptions)
        -> Result<Self, DurabilityError>;
}
```

- `lock` is the consumer's directory lock, taken before the call. Requiring
  it is a type-level proof that the caller excludes other writers, so the
  heap takes no lock of its own.
- The `rusty_multimodal_db` server locks its data directory with its own
  `DataDirLock` (`server/data_lock.rs:60`), not the engine's `DirLock`. Both
  are the same `try_lock` on a file in the directory. The server's phase 4
  PR replaces `DataDirLock` with `DirLock::acquire(dir, LOCK_FILE_NAME)`,
  keeping the file name and the `Held` message, so one type proves
  exclusion everywhere.
- An out-of-range `segment_bytes` returns `DurabilityError::InvalidOptions`.
- `open_segments` on an old-layout store returns
  `DurabilityError::NeedsMigration { path }`.
- `open`/`open_portable` on a segment store return
  `DurabilityError::LayoutMismatch { path }`.

Each `open_*`/`create_*_production_stack` helper gains a `_segments`
variant with the same extra parameters. A consumer opts in by calling them.
The existing helpers and their defaults do not change until phase 5.

### 7.3 Files

All files are flat siblings of `<path>`. The server's prefix-matched
`table_files`, its `is_plain_file_name` check and restore therefore work
unchanged, and the wire protocol does not change.

| File | Contents |
|---|---|
| `<path>` | Slot file, unchanged |
| `<path>.<field>.slots` etc. | `MmapScanned` slot files, unchanged |
| `<path>.edges`, `<path>.<label>.edges`, their `.inserts` | Edge blobs and logs, unchanged |
| `<path>.seg-NNNNNN` | Segments, numbered from 1 |
| `<path>.hint-NNNNNN` | Key list for one segment (§7.4) |

Open deletes leftover `*.tmp` files with the store's prefix before anything
else, and `Backup` skips any that remain. Segments are copied for backup,
never hard-linked: a hard link shares blocks with the live file, so bit rot
in the original would also be in the backup.

### 7.4 Formats

All integers are little-endian. `W` is
`<R::Id as MmapFieldValue>::BYTE_WIDTH`.

**Segment header.** This is the 28-byte tagged header
(`encode_tagged_image`, `record_blob.rs:118-130`): magic `MMDBSEG\0`, `u32`
format version 1, `u64` first sequence number of this segment, and `u64`
schema-tag hash. A 16-byte store UUID follows; the UUID is fixed at
`create_segments` and copied by migration and backup. Open refuses a
segment whose magic, version, tag or UUID does not match, and the error
names the file (`DurabilityError::ForeignSegment { path }`).

**Entry.**

| Field | Width | Notes |
|---|---|---|
| `kind` | 1 | 0 = `put`, 1 = `tombstone` |
| `id` | W | Record id |
| `seq` | 8 | Assigned once per write, increasing |
| `len` | 4 | Payload length; 0 for a tombstone |
| `crc` | 4 | CRC-32 over every field above and the payload |
| payload | `len` | `codec`-encoded `R` (put only) |

With the id in the header, a scan builds the key directory without decoding
payloads. A record larger than `segment_bytes` gets a segment of its own.

**Hint.** A tagged header (magic `MMDBHNT\0`, version 1, the segment number,
the tag hash), then:

| Field | Width |
|---|---|
| `segment_len` | 8 |
| `entries` | 8 |
| rows: `kind`, `id`, `seq`, `offset` (u32), `len` (u32) | 1 + W + 8 + 4 + 4 each |
| `crc` over the hint body | 4 |

Install it as temp file, `sync_data`, rename, directory sync, the way
`EncodedRecordBlob::write` installs a blob. Open trusts a hint only if its
CRC is valid and `segment_len` equals the segment's length. Loading a hint
is O(entries in the segment).

### 7.5 Writes, reads and the key directory

Writes:

- **put:** assign the next `seq`, append, `sync_data` unless deferred, then
  update the key directory. The previous location becomes dead.
- **remove:** append a tombstone, `sync_data` unless deferred, then drop the
  id from the key directory.

The key directory maps each id to `{ segment: u32, offset: u32, len: u32 }`,
about 12 B plus the id. With W = 16 and hashbrown's overhead (one control
byte, load factor 0.5–0.875), that is about 32–64 B per record.

Accounting, in RAM per segment:

- `bytes`, `live_put_bytes` and `tombstone_bytes`, filled from hints or the
  scan at open and updated on every write.
- Dead bytes = `bytes − header − live_put_bytes − tombstone_bytes`.

Reads:

- **get:** a positional read of the entry, a CRC check and a decode. The OS
  page cache is the only cache.
- Positional reads use `FileExt::read_at` on Unix and `seek_read` on a
  per-call handle on Windows.

**Writes read the old record once.** Layers call `inner.get(id)` on replace
and delete to find old index keys. On segments, that means one decode and
several clones. `GenericProductionStore::replace_if` (`production.rs:322`)
already reads the old record once under the write lock. A new
`replace_with_old(old, new)` path generalises it, and layers take `old`
from it instead of calling `get`.

### 7.6 Slot files stay authoritative

`UpdateField::update` writes only the slot (`mmap_store.rs:1184-1188`), and
`get` overlays the slot value (`:1077-1082`). So a segment body can be stale
for a slot-backed field. That is safe because every read overlays the slot
value:

- `get` and the new `for_each_overlaid` (§7.7) decode, then overlay.
- Compaction and migration may copy stale bodies, because the slot files go
  with them.
- `position_index` stays in `GenericMmapStore`. `compact()` keeps its
  gapless slot rewrite, and it rebuilds `position_index` immediately after
  the slot rewrite (#1355's ordering).

### 7.7 Open and index build

1. Delete stale temp files.
2. Read each segment's hint if it is trusted; otherwise scan the segment's
   entry headers and check each CRC.
3. Apply the recovery rule (§8.3).
4. Build the key directory. The highest `seq` wins for each id; on equal
   `seq`, the higher segment number wins and the other copy is dead. A
   tombstone removes its id.
5. Set the next `seq` to the larger of the active segment's first `seq` and
   the largest `seq` seen, plus 1. The active segment's first `seq` survives
   even if the tombstone that held the largest `seq` was dropped.
6. Delete the active segment's hint before the first append, so that it can
   never describe a longer file.
7. Build the layers.

Layer build: a new query trait gives every store a sequential, overlaid
pass:

```rust
pub trait ForEachRecord<R> {
    /// Every live record once, overlaid with any slot values. Order is unspecified.
    fn for_each_overlaid(&self, f: &mut dyn FnMut(R) -> Result<(), DurabilityError>)
        -> Result<(), DurabilityError>;
}
```

- The default implementation is `all_ids` + `try_get`.
- `GenericMmapStore` on segments overrides it with a sequential read of each
  segment that skips dead entries.
- `MmapScanned` wraps its inner store's pass and applies its own overlay.
- Every layer constructor that takes `records: &[R]`, plus `Ordered::new`
  and `NameIndex::new`, iterates `inner.for_each_overlaid` instead. Each
  layer therefore sees exactly what its inner stack returns.
- K layers cost K sequential passes, served from the page cache after the
  first. If PAGED-G3 fails, a single-pass builder is the follow-up
  (open question 2).

### 7.8 Compaction

A sealed segment is a candidate when its dead bytes are at least half its
size. `needs_compaction()` is true when any segment is a candidate, when
the tombstone budget below is exceeded, or when the slot file has gaps.

One compaction step works on one segment:

1. Copy each live `put` to the active segment, keeping its `seq`.
2. Copy each tombstone forward if its id is still absent from the key
   directory and an older segment survives. Drop it if this is the oldest
   segment, or if a newer `put` exists.
3. Sync (§8.2).
4. Delete the hint, then sync the directory.
5. Delete the segment, then sync the directory.

Why no record comes back:

- Only live puts are copied, so a put that a tombstone superseded is never
  carried forward.
- A tombstone is dropped only when no older segment exists that could hold
  a put for its id.
- The directory sync after each unlink makes "the older segment is gone"
  durable before a later step relies on it.

**Tombstone budget.** A cold oldest segment that stays under half dead would
force every tombstone to be copied forward forever. So when total
`tombstone_bytes` exceeds 10% of heap bytes, the oldest segment becomes a
candidate regardless of its dead fraction. Once it is compacted, the next
segment becomes the oldest, and its tombstones can be dropped.

`Compact::compact` runs under the production write lock for the whole
call, as it does today. It compacts every candidate, then rewrites the slot
file. `CompactionReport` becomes `#[non_exhaustive]` and gains
`segments_compacted` and `bytes_reclaimed`.

## 8. Errors, failure, recovery and observability

### 8.1 Append and sync failures

The segment writer is #1355's `write_entry`/`roll_back`/`poison`, moved
into `segment_log`:

- **A failed write** is truncated back to the last complete entry through a
  separate write handle, and the truncate is synced. The caller gets a
  clean error, and the entry was not written.
- **A failed truncate, or any failed `sync_data`,** poisons the heap. The
  caller gets `DurabilityError::Poisoned`, and the write's outcome is
  **unknown**: its bytes may be intact and survive reopen.
- The server's message for a poisoned error changes from "nothing was
  written" to "outcome unknown; reopen the store". `refuse_if_poisoned`
  returns the same variant on the legacy path.
- A poisoned heap refuses writes and syncs until it is reopened. Reads keep
  working.
- A sync is never retried on a fresh handle.
- Inside a deferred batch, entries appended before a failure stay appended.
  Batch atomicity remains the redo journal's job, as today (`BatchApply`).

Poisoning lives in an `AtomicBool`, and the active segment's handle sits
behind a `Mutex`. `sync` and `Flush::flush` therefore take `&self`, which
matches `Flush::flush(&self)` and `CheckpointFlush`. `Flush::flush` on a
segment store is: sync the active segment, then `msync` the slots. A redo
owner calls it before checkpointing, as today.

### 8.2 Segment lifecycle

- **Create:** `O_CREAT|O_EXCL`, write the header, `sync_data`, sync the
  directory, then accept appends.
- **Seal:** `sync_data` the active segment, write its hint (§7.4), then
  create the next segment. A sealed segment ends with its last entry, and
  nothing is appended after its final sync.
- **Clean close:** `sync_data`, then write the active segment's hint.
- **Group commit:** the heap remembers which segments were written since the
  last sync, and `sync()` syncs all of them. A deferred batch that crosses a
  seal is therefore durable when `commit()` returns.

### 8.3 Recovery rule

This is `journal.rs`'s rule, applied per segment:

- **Sealed segments** were synced before sealing. Any bad entry in one is
  `DurabilityError::Corrupt { path, offset }`, and the store refuses to
  open. The remedy is restore from backup (ADR-0065/0070).
- **The active segment:** truncate at the first bad entry (bad CRC, a short
  frame, a length past the end, or zeroes), log the number of bytes dropped,
  and open.
  - Under deferred sync, unsynced entries can reach disk out of order. The
    truncation drops only entries written after the last successful sync.
  - **Known limit:** bit rot inside already-synced data in the active segment
    reads as a tail and is dropped rather than refused. This is today's
    insert-log behaviour.
- **Highest segment with a missing or short header** (a crash during
  creation): delete it if its bytes are a prefix of a valid header;
  otherwise `Corrupt`. The previous segment stays sealed, and a fresh active
  segment is created.

### 8.4 Read errors

- `GetById` gains `try_get(&self, id) -> Result<Option<R>, DurabilityError>`,
  with a default of `Ok(self.get(id))`, so the ten existing implementors are
  unaffected.
- Segment stores implement `try_get` and implement `get` as `try_get` that
  panics on `Err`. `get` never returns `None` for an unreadable record.
- A CRC failure at read time is `Corrupt` and poisons the heap.
- Every engine-internal read uses `try_get`. That covers layers,
  `for_each_overlaid`, `replace_with_old` and compaction, and a test
  enforces it. A panic under the production write lock would poison the
  `RwLock` for good; a panic under a read guard does not.
- Callers that switch to `try_get` in their rollout PR: the server adapters,
  rusty_tick, and orch-store's legacy read-through, which treats
  `get() == None` as absent (`orch-store/src/lib.rs:240-243`).

### 8.5 Observability

The server's metrics renderer exposes, per table:

- `mmdb_heap_segments`
- `mmdb_heap_bytes`
- `mmdb_heap_dead_bytes`
- `mmdb_heap_tombstone_bytes`
- `mmdb_heap_needs_compaction` (0 or 1)
- `mmdb_heap_poisoned` (0 or 1)

## 9. Security, privacy and compatibility

- **Untrusted payloads.** Record payloads are untrusted, because the server
  accepts network writes. Recovery never searches for frames inside damaged
  data, so a crafted payload cannot steer it. The CRC is not a security
  boundary.
- **Snapshots and backup.** `MAX_SNAPSHOT_BYTES` (8 MiB) caps a table's
  total size. Tables over it already use the chunked path (ADR-0136), and
  nothing changes there.
- **API breaks:** listed in §14 with the phase that introduces each one.

## 10. Phases

Each phase is one PR that leaves `main` green.

| Phase | Ships | Format | Gates | Back out |
|---|---|---|---|---|
| M | Measure every consumer (below) | None | — | n/a |
| 0 | First-party codec (§6) | None | PAGED-T6 | Revert |
| 1 | `segment_log` (crate-private) on #1355's writer: framing, recovery rule, seal and close order, hints, `durability::crc` | None (unused code, covered by its own tests; `#[cfg_attr(not(test), allow(dead_code))]` until phase 3) | PAGED-T2 (unit) | Revert |
| 2 | `try_get`, `ForEachRecord`, `replace_with_old`; constructors iterate `for_each_overlaid` | None | PAGED-T1 (map layout), existing suites | Revert |
| 3 | `SegmentHeap`, `create_segments`/`open_segments` and the `_segments` helpers, compaction, the migration tool, server backup and metrics | New files, opt-in only | PAGED-T1–T5, T7, G1–G3 | Revert; no consumer uses it yet |
| 4 | One PR per consumer, in phase M's order: switch to the `_segments` helpers, migrate | Yes, per consumer | PAGED-T7 on that consumer's data shape; G4 for body-heavy consumers | Swap back to the untouched source |
| 5 | Default flip: the plain helpers open segments, and the old layout returns `NeedsMigration`. `Map` removed in the next minor | — | All in-repo consumers migrated | Revert before release |
| 6 | Persisted derived indexes, full-text first. Own design; timing from phase M | Later | Later | Later |

**Phase M method.**

- **Data:** a backup copy of each consumer's store. The remind-me hub is run
  in its own repo against a backup, never live.
- **Tool:** a small `examples/footprint.rs` that opens the store and reads
  `VmRSS` from `/proc/self/status` after each step: baseline, open core,
  each layer, full-text. It also records reopen-after-writes time and
  `compact()` time.
- **Output:** `docs/design/paged-records/phase-m.md`, with one row per
  consumer.
- **Decides:**
  - the rollout order in phase 4 (biggest body share first);
  - the PAGED-G4 target per consumer;
  - whether phase 6 starts before phase 4 finishes;
  - whether remind-me migrates in phase 4 at all.

## 11. Migration

```rust
pub fn migrate_to_segments<R, I, S>(
    src: &Path, dst: &Path, lock: &DirLock, opts: HeapOptions,
) -> Result<MigrationReport, DurabilityError>;
```

`src` and `dst` are store paths (`<path>` prefixes), and `lock` covers
both. The tool:

1. Refuses if any file with the `dst` prefix exists.
2. Opens `src` read-only through `read_portable_records`, which writes
   nothing.
3. Writes segments and hints under `dst`, and copies the slot files, edge
   blobs and edge logs (and the label manifest) unchanged.
4. Syncs every file and the directory.
5. Reopens `dst`, streams `for_each_overlaid`, and compares each record with
   `src`'s overlaid record. It never holds both sets in full.
6. Never writes to `src`. The operator swaps paths, and the untouched `src`
   is the rollback (ADR-0066).

A consumer's migration command:

- runs its own redo replay first, by opening and cleanly closing `src`
  through its normal path;
- refuses if its journal is not checkpointed (the server checks its data
  directory's journal);
- loops over its stores. rusty_tick, for example, loops over its users.

There is no batch mode and no migrate-on-open helper.

Rollout order comes from phase M. The consumer set is:

- rusty_tick;
- `rusty_fair_play_domain` and rusty_fair_play;
- orch-store, which has three stores per directory, including a legacy
  read-through;
- the `rusty_multimodal_db` server tables, together with its `Backup` and
  snapshot paths;
- rleval-app's `mmdb` feature;
- the remind-me hub (another repo, by libs tag);
- agent-cloud ac-store (pinned rev).

Mill Board's adapter is being retired (root ADR-0012) and is not migrated.

rusty_tick's per-batch `snapshot()` (`rusty_tick/src/store.rs:175`,
`197-207`) is a point-in-time recovery copy, so it must keep pre-write
values. Its rollout PR builds the recovery view from the batch's own `old`
values (read at `:176-183`) over the live store. That is O(batch) and needs
no engine change.

## 12. Acceptance criteria (gates)

The baseline is `main` at the phase's merge base. "No slower" means the
median of 5 runs is at most 5% slower.

- **PAGED-G1 writes** (blocks phase 3): insert, replace and delete are no
  slower than the map layout at each durability level. This is
  `benches/vs_sqlite.rs`, extended with delete and a 1,000,000-record size.
- **PAGED-G2 reads** (blocks phase 3): `get` is within 2× of SQLite with a
  working set larger than RAM. The bench calls `posix_fadvise(DONTNEED)` on
  the segment files between rounds. If G2 fails, the follow-up is a record
  cache (open question 3).
- **PAGED-G3 open** (blocks phase 3): at 15,000 and 1,000,000 records, a
  clean open is no slower than the map layout. Two stacks are measured:
  - the server `Memory` stack (`server/memory.rs`) with full-text, as the
    remind-me shape;
  - a rusty_tick-shaped stack (two `Ordered` layers over the core).
- **PAGED-G4 RAM** (blocks each body-heavy consumer's phase 4 PR): RSS
  against the target phase M sets. The reference shape is 1 KiB bodies with
  two indexes at 1,000,000 records, where RSS must be at least 4× below the
  map layout.
- **PAGED-G5** (blocks phase 3 and every phase 4 PR): PAGED-T1–T8 pass, and
  the `rusty_multimodal_db` feature-set CI job is green.

## 13. Verification plan

| ID | What | How |
|---|---|---|
| PAGED-T1 | Equivalence | One generated operation sequence against a segment store and the `AosStore` oracle, with reopens, compactions, `update`s and deletes mixed in. Every query matches, and so does every index after reopen |
| PAGED-T2 | Fault injection | New `test_support::Fault` variants: `SegmentWritePrefix(n)`, `SegmentTruncate`, `SegmentSync`, `HintInstall`, `SkipDirSync`, `ReadCrc`. Cases: failed then successful append; failed truncate (poison); failed sync (poison, outcome unknown); deferred batch crossing a seal; unsynced unlink (`SkipDirSync`) then tombstone drop; read CRC failure; equal-`seq` orphan after a compaction crash |
| PAGED-T3 | Process crash | `crash_writer` gains a segment mode with `segment_bytes = 64 KiB` and a `test-support` abort hook at named steps: mid-append, mid-seal, mid-hint-install, mid-compaction, mid-create. Each reopens to the acknowledged state |
| PAGED-T4 | Power loss | `power_loss_trial.sh` gains a filesystem argument (ext4, XFS) and a segment-store mode. Its `dm-log-writes` replay is in order, so it covers prefix crashes, not sector reordering. Run before phase 3 merges; results go in the PR |
| PAGED-T5 | Backup round trip | `Backup`, restore and reopen through the server, chunked snapshot included; temp files are skipped |
| PAGED-T6 | Codec parity | Golden vectors, plus a differential property test against bincode 1.3.3 over every persisted type in use (rusty_tick `Task`, orch-store, fair_play, rleval `SessionBlob`, the server's `Request`/`Response`/`WriteOp`/`TransactionOp`/journal entries) and adversarial inputs. `cargo tree -e normal` shows no `bincode`. remind-me runs the test on `Memory` in its own CI |
| PAGED-T7 | Migration | `dst` matches `src` record for record; `src` bytes are unchanged; an existing `dst` is refused; `open_segments` on the old layout returns `NeedsMigration`; `open` on segments returns `LayoutMismatch` |
| PAGED-T8 | Exclusion and options | A foreign segment is refused; out-of-range `segment_bytes` is refused; segment constructors cannot be called without a `&DirLock` (compile-time) |

## 14. Traceability and API changes

| Requirement | Design | Tests and gates | Phase |
|---|---|---|---|
| FR-001 | §7.3–7.5 | G4 | 3, 4 |
| FR-002 | §7.5, §7.8 | G1, T1 | 3 |
| FR-003 | §7.4, §7.7 | G3, T2, T3 | 1, 3 |
| FR-004 | §8.4 | T2 (`ReadCrc`) | 2, 3 |
| FR-005 | §8.1–8.3 | T2, T3, T4 | 1, 3 |
| FR-006 | §11 | T7 | 3, 4 |
| FR-007 | §6 | T6 | 0 |
| FR-008 | §7.1–7.2, §10 | CI on each PR | all |

| API change | Phase |
|---|---|
| `codec::Error` replaces `bincode::Error`; `DurabilityError` becomes `#[non_exhaustive]` | 0 |
| `GetById::try_get` (defaulted); `ForEachRecord`; `replace_with_old`; layer constructors read through `for_each_overlaid` | 2 |
| `HeapOptions`; `create_segments`/`open_segments` and the `_segments` helpers; `migrate_to_segments`; `DurabilityError::{Corrupt, Poisoned, NeedsMigration, LayoutMismatch, ForeignSegment, InvalidOptions}`; `CompactionReport` `#[non_exhaustive]` with new fields | 3 |
| Plain helpers open segments; `open(records, path)`, `read_portable_records` and the public `insert_log` API (`clear_interrupted_creation`) are deprecated, then removed with `Map` | 5 |

## 15. Open questions

1. **Phase M result:** the per-consumer RAM targets, the rollout order, and
   when phase 6 (persisted full-text) starts. The owner decided on
   2026-11-01 to measure first.
2. **Single-pass index build:** add one only if PAGED-G3 fails.
3. **Record cache:** add one only if PAGED-G2 fails. It would be a single
   `Mutex<LRU>` under one process-wide byte budget, never one per store.
4. **`compact_step`:** whether to release the write lock between segments.
   Only if a consumer needs it.
5. **The 10% tombstone budget:** tune it from phase 3 benches.

## 16. Change history

- **Draft 1 (2026-11-01):** first design, with a `RecordHeap` port, a
  per-store cache, a `.heap/` directory and a position-based torn-tail rule.
- **Draft 2 (2026-11-01):** folded in Elephant Goldfish review 1 and expert
  review 1:
  - flat files;
  - id and `seq` in each entry;
  - oldest-segment tombstones;
  - authoritative slots;
  - `try_get`;
  - migration to a fresh directory;
  - sync-mark recovery;
  - a shared cache;
  - one-pass builders.

  The live insert-log defect was fixed separately in #1355.
- **Draft 3 (2026-11-01):** folded in Elephant Goldfish review 2 (seven
  lenses).
  - **Dropped:** sync marks and the entry magic (they broke sealing, and a
    crafted payload could fake one); the port trait and type parameter; the
    record cache; the one-pass builders; the heap's own lock;
    migrate-on-open and the batch mode.
  - **Added:**
    - explicit opt-in and a separate default-flip phase, so `main` stays
      green;
    - `HeapOptions` and the exact constructors;
    - tombstone accounting and budget;
    - the `seq` start in segment headers, plus a tie-break;
    - "outcome unknown" on poison;
    - `&self` sync;
    - copied (not hard-linked) backups;
    - the hint format;
    - phase M's method for every consumer;
    - gates tied to phases;
    - IDs, traceability and the house template.
  - **Corrected:** the ADR-0138 citation (decision 2), the rusty_tick
    `snapshot()` approach, and code citations.
