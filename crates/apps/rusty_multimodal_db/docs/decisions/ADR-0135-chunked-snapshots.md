# ADR-0135: Chunked Snapshots — Lifting the 8 MiB Cap (Proposal)

- Status: **Proposed — design only, no code. Forks for the owner at the end.**
- Date: 2026-09-30
- Deciders: baileyrd
- Related: `ADR-0067` (`FetchSnapshot` and the 8 MiB ceiling), `ADR-0065`
  (`Backup`, which copies a table's files under the write lock),
  `ADR-0118`/`0123` (`replica_refresh`), `ADR-0131` (the change log and
  `SnapshotAt`), `ADR-0134` (the grouped log sync), `docs/FUTURE-GROWTH.md`.
- Supersedes/Superseded by: none. If accepted: protocol 35 → 36, append-only.

## Context

A standby bootstraps from a snapshot, and `replica_refresh --follow`
(`ADR-0131`) tails the change log from that snapshot's position. So a table
larger than `MAX_SNAPSHOT_BYTES` (8 MiB, `protocol.rs`) cannot get a standby at
all: `FetchSnapshot` answers `TooLarge` before reading a byte.

Three things make the cap what it is, and each has to be answered:

1. **One frame.** The whole snapshot is one `Snapshot`/`SnapshotAt` response,
   and a frame is at most `MAX_FRAME_BYTES` (16 MiB). The 8 MiB leaves room for
   file names and bincode framing.
2. **One buffer.** `read_table_files` reads every file into memory on the
   server, and the client holds the same `Vec` before it writes anything.
3. **One lock.** The read runs inside the table's exclusive section
   (`fetch_snapshot` is `with_exclusive(|_| read_table_files(base))`, and under
   `ChangeLogged` also inside the log's lock) so the files and the log position
   agree. Writers wait for the whole read.

Raising the constant fixes none of the three. A frame over 16 MiB is refused; a
1 GiB buffer on both ends is not a design; and a lock held while bytes cross a
network is held for as long as the slowest client takes.

## What holding the lock costs, measured

The one part that cannot be designed away is that a consistent copy has to be
taken while no write is applying, because the table is a set of files that are
mutated in place (a slot file is updated with `mmap`, the insert log and edge
logs are appended to) and only agree with each other between writes.
`Backup` already accepts this: it copies the files into a directory under the
write lock (`copy_table_files`).

On this host (a 4-core VM, ext4), copying a 512 MiB file took **3.0–3.1 s**
(`cp`, no `fsync`; about 170 MiB/s, the page cache throttling on dirty pages),
so a locked copy stalls writers about **6 s per GiB**. Streaming *under* the
lock would cost the same plus the network: a slow standby would set the stall.
So the lock is held for a local copy and never for a transfer.

## Options

1. **Chunk the response but keep the lock (rejected).** Stream `Chunk`
   frames from inside the exclusive section. Fixes the frame and the buffer,
   makes the lock hold time the client's download time. A standby on a slow link
   could stall a table's writers for minutes.
2. **Stage, then stream (recommended).** Under the lock, copy the table's files
   to a staging directory and note the log position; release the lock; serve
   the staged files in chunks to the client by a snapshot handle; delete the
   staging directory when the client says it is done, the connection ends, or an
   idle timer fires. The lock is held for a local copy (the cost above), the
   transfer takes as long as it takes, and nothing in memory is larger than a
   chunk.
3. **Fuzzy copy plus log replay.** Read the files without the lock, in chunks,
   from a noted log position, and let the standby's idempotent replay
   (`ADR-0131`) repair whatever changed underneath. No lock and no staging.
   Rejected for now: the files must agree with *each other* (the slot file, the
   record blob and its fingerprint, the insert log, the edge logs) and a copy
   taken across writes need not; the standby's verification reopen would refuse
   it or, worse, accept a torn one. It is a research direction (it would need a
   crash-consistent read of each file), not a phase.
4. **Per-file fetch with no staging.** The standby asks for one file at a time.
   Same consistency problem as 3, plus a round trip per file.

## Decision (proposed): option 2

**Wire (protocol 36, appended, `Replication` token only, like `FetchSnapshot`):**

- `Request::BeginSnapshot` (40) → `Response::SnapshotManifest` (28):
  `{ snapshot: u64, position: Option<(u64, u64)>, files: Vec<(String, u64, [u8; 32])> }`
  — an unguessable handle bound to this connection, the change-log
  `(epoch, seq)` the copy agrees with (`None` for a table with no log, as
  `Snapshot` today), and per file its name, its length and its SHA-256 (the
  crate already depends on `sha2`).
- `Request::FetchChunk { snapshot, file: u32, offset: u64, len: u32 }` (41) →
  `Response::Chunk { bytes }` (29). `len` is at most `MAX_CHUNK_BYTES` (4 MiB,
  well under the frame cap); a chunk that runs past the file's end is short, and one that
  starts at or past it is empty. Chunks may be asked in any order, so a client can resume.
- `Request::EndSnapshot { snapshot }` (42) → `Ok`; frees the staging copy.
- `ErrorCode::NoSnapshot` (17): the handle is unknown, expired, or another
  connection's. `Busy` (15) when the table already has a staged snapshot.
- Below 36: the three requests are `Malformed` (rule 3). `FetchSnapshot` is
  unchanged: it still answers a table of 8 MiB or less in one frame, and
  `TooLarge` above, so an old client and a small table behave as before.

**Server:**

- `SERVER_SNAPSHOT_DIR` (required to enable it; unset, `BeginSnapshot` is
  `Unsupported`), and `SERVER_SNAPSHOT_MAX_MB` (default 4096, the new ceiling
  for `BeginSnapshot`; `MAX_SNAPSHOT_BYTES` remains the legacy request's).
  Staging needs disk equal to the table: the ceiling and the free-space check
  (refuse `TooLarge`/`Storage` before copying) are what keep it bounded.
- One staged snapshot per table at a time (`Busy` otherwise), an idle timeout
  (default 300 s) that frees it, deletion on `EndSnapshot` and on connection
  close, and a sweep of stale staging directories at startup (a crash leaves
  them).
- `ConnectionStore::begin_snapshot` does the staged copy inside the exclusive
  section and, with a log, inside the log's lock so the position agrees, exactly
  as `fetch_snapshot_at` does now; `read_chunk` and `end_snapshot` touch only the
  staging directory and take no table lock.

**Client:** `SchemaDrivenClient::fetch_snapshot_chunked(sink)` streams each file
into a caller-supplied writer (no whole-file buffer), checks each length and
SHA-256 against the manifest, and returns the position. `replica_refresh`
writes into its staging directory as chunks arrive, so its memory is one chunk;
it uses this when the server negotiated 36 or more and `FetchSnapshot` below.
The Python client gains the three requests.

## Interaction with the rest

- **The change log (`ADR-0131`/`0134`):** unchanged. The position is taken at
  the copy, exactly as now; the standby tails from it. `ADR-0134`'s grouped sync
  does not change what the position means.
- **Backup:** `Backup` and the staging copy are the same operation with a
  different destination; they should share `copy_table_files`.
- **Retention:** a large snapshot takes long to download and the log keeps
  growing; if the download outlasts the log's retention the standby gets `Gone`
  on its first `FetchSince` and must start over. `SERVER_CHANGE_LOG_RETAIN_MB`
  has to exceed the write volume of one download (documented, not enforced).

## Costs and limits, stated plainly

- **A write stall proportional to the table**, about 6 s per GiB on the
  measured disk: the same property `Backup` has, now on a path a standby can
  trigger repeatedly. Mitigations, none built: a filesystem clone (`FICLONE`) is
  O(1) where the filesystem supports it (this host's ext4 does not); the ceiling
  can be set lower; a standby should not bootstrap in a write burst.
- **Disk for the staging copy** equal to the table.
- **Only one connection's handle**: a dropped connection loses its snapshot and
  the standby starts over. Resuming across connections is not proposed.
- Not addressed: a table so large that even one local copy is unacceptable.
  That needs option 3 or a different design (a log-structured store).

## Before it ships (if accepted)

- Tests: a table over 8 MiB round-trips through `BeginSnapshot`/`FetchChunk`
  to an identical, reopenable directory; a corrupted chunk fails the SHA-256; a
  handle from another connection, an expired one, and one past `EndSnapshot` are
  `NoSnapshot`; the staging directory is gone after `EndSnapshot`, a dropped
  connection and an idle timeout; writers blocked during the copy proceed the
  moment it ends, not when the download does.
- Crash trials: a kill during the staging copy leaves no directory a restart
  serves from and a swept one; a kill mid-download leaves the standby's staging
  directory unrenamed (`replica_refresh` already installs by rename).
- Golden wire vectors for the three requests, two responses and the new code;
  `SERVER-002` 0.25.0.
- A benchmark of the write stall against table size (1, 4, 16 GiB), so the
  ceiling default is a measurement.

## Forks for the owner

- Option 2, or leave the cap (a table over 8 MiB then has no standby).
- The default ceiling (proposed 4 GiB, about 25 s of write stall on the measured
  disk) and whether staging should require a filesystem with clone support.
- Whether the legacy `FetchSnapshot` should stay for small tables (proposed: yes,
  nothing changes for a client below 36).

## Consequences (if accepted)

- Positive: a standby for a table of any size the ceiling allows; the client's
  memory is one chunk; per-file integrity checks the snapshot never had.
- Negative: a protocol bump, a staging directory to manage (limits, sweeps),
  a write stall that grows with the table, and a new class of leftover state
  after a crash.
- Not changed: the change log's format, `FetchSince`, promotion.
