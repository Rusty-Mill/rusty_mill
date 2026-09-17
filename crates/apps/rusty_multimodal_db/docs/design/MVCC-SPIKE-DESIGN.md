# MVCC Spike Design (Accepted)

- Status: **Accepted** — owner picked this scope and mechanism
  interactively, 2026-09-16, alongside `ADR-0071`. Authorizes the
  spike only; full `src/server/**` wiring is a separate, later unit.
- Date: 2026-09-16
- Related: `ADR-0071` (the full decision record — read that first for
  Context/Decision-drivers/Considered-options; this document is the
  concrete implementation shape for the spike phase only), `ADR-0033`/
  `docs/design/SERVER-SESSION-SNAPSHOT-ISOLATION-DESIGN.md` (the
  isolation mechanism this one is an alternative to, not a
  replacement of), `ADR-0052` (Compact — the GC trigger this design
  reuses), `GENERIC-SCHEMA-DESIGN`'s four validation spikes in
  `src/generic_spike/` (the precedent for gating novel,
  storage-adjacent machinery behind the `research` feature before
  promoting it to a real library or server-facing unit).

## Purpose and scope

Prove the core mechanism `ADR-0071` proposes — versioned writes via a
side log, snapshot-consistent reads, write-write conflict detection at
commit, and log reclamation bounded to what no open snapshot needs —
in a small, isolated harness, before committing to wiring it into the
real `src/server/**` session machinery, protocol, and production
domains.

**In scope:**

- A new module, gated behind the existing `research` Cargo feature
  (see `Cargo.toml`'s `research = []`), alongside `src/generic_spike/`'s
  own precedent for exploratory/validation code that is not shipped
  production capability.
- A minimal, in-memory-or-file-backed harness (the spike's own choice;
  it does not need to reuse `SlotFile` verbatim, only to prove the
  *mechanism* `SlotFile` would eventually need) modeling one table's
  worth of `(RecordId, FieldRef) -> value` state, with:
  - a monotonically increasing per-table `TxnId` counter;
  - a `last_write_txn` stamp per key, checked in place of a separate
    read-set map;
  - a side, append-only version log recording superseded values keyed
    by the txn id that superseded them;
  - a snapshot handle (`snapshot_txn: TxnId`) that can read a
    key's value *as of* that snapshot, whether or not it is the
    current value;
  - a write-write conflict check at commit time: a snapshot's staged
    writes are rejected atomically, as a whole, if any target key's
    `last_write_txn` has advanced past the snapshot's own `snapshot_txn`
    since it was taken;
  - a GC step, given the current minimum open-snapshot `TxnId` (or
    "none," meaning everything is reclaimable) as an explicit input —
    **not** real session tracking; that belongs to the later
    full-implementation unit — that prunes every version-log entry
    whose `superseded_by_txn` is at or below that minimum.
- Tests proving the acceptance criteria below.

**Out of scope (see `ADR-0071`'s own "Explicitly out of scope"):**

- Any change to `src/server/**`, the wire protocol, `PROTOCOL_VERSION`,
  or any shipped binary.
- Any change to `SlotFile`'s real on-disk format, `src/generic/**`, or
  `src/production.rs`.
- Multi-table atomicity, staged non-field-update operations, real
  session/connection wiring, or persistence of the txn counter across
  process restarts (the spike may keep it in memory; the full unit
  decides the real persistence mechanism).
- Performance measurement/benchmarking — this is a correctness spike,
  not a `benches/` addition.

## Requirements

- `SPIKE-FR-001` — **Gated, non-production module.** Lives under
  `src/generic_spike/` (or a clearly-named sibling module under the
  same `research`-gated tree — implementation's choice, following
  existing naming conventions in that directory) with `#[cfg(feature
  = "research")]` (or the module-level equivalent already used by its
  siblings). Not referenced from any non-`research`-gated code path.
- `SPIKE-FR-002` — **Versioned write.** A write to `(id, field)`
  advances the table's `TxnId` counter, appends the value it is about
  to replace (with the new txn id as `superseded_by_txn`) to the
  version log — skipped if this is the key's first-ever write, nothing
  to supersede — then overwrites the current value and its
  `last_write_txn` stamp.
- `SPIKE-FR-003` — **Snapshot-consistent read.** Given a
  `snapshot_txn`, reading `(id, field)` returns the current value if
  its `last_write_txn <= snapshot_txn`; otherwise it walks that key's
  version-log entries and returns the `old_value` of the entry with
  the smallest `superseded_by_txn` that is still strictly greater than
  `snapshot_txn`. A `snapshot_txn` taken before any write to a key
  that is later created must see "not present," not a spurious value.
- `SPIKE-FR-004` — **Write-write conflict at commit.** A snapshot
  stages one or more writes locally (not yet applied); committing
  checks, for every staged key, whether its live `last_write_txn`
  exceeds the snapshot's own `snapshot_txn` — if any does, the whole
  commit is rejected and nothing is applied; otherwise every staged
  write applies per `SPIKE-FR-002`, atomically as a set (this spike
  may use a single-threaded harness with an explicit critical section
  rather than real concurrency — proving the *logic*, not the lock,
  is the point; a real concurrent stress test belongs to the full
  implementation unit).
- `SPIKE-FR-005` — **Bounded GC.** Given an explicit "minimum open
  snapshot" input, GC removes every version-log entry with
  `superseded_by_txn` at or below it, and leaves every entry above it
  untouched. Confirmed by: a value still reachable from an open
  snapshot older than the GC boundary is unaffected; a value reachable
  only by snapshots at or below the boundary is gone, and a read
  attempting to use an already-GC'd snapshot for a GC'd key must fail
  cleanly (a typed error, not silent wrong data or a panic) rather
  than pretend correctness it cannot provide.
- `SPIKE-FR-006` — **No production wiring.** Zero change to any file
  outside the new spike module (plus this design doc, `ADR-0071`, and
  the routine roadmap/status/traceability updates below) — no
  `Cargo.toml` dependency change, no `SlotFile`/`src/generic/**`/
  `src/server/**` edit.

## Proposed shape

```rust
// src/generic_spike/mvcc_spike.rs (new), #[cfg(feature = "research")]

pub type TxnId = u64;

#[derive(Clone, Copy, PartialEq, Eq, Hash)]
pub struct Key { pub id: RecordId, pub field: FieldRef }

struct VersionLogEntry { old_value: ScanValue, superseded_by_txn: TxnId }

pub struct MvccTable {
    next_txn: TxnId,
    current: HashMap<Key, (ScanValue, TxnId /* last_write_txn */)>,
    log: HashMap<Key, Vec<VersionLogEntry>>, // oldest..newest, append-only
}

pub struct Snapshot { snapshot_txn: TxnId, staged: HashMap<Key, ScanValue> }

impl MvccTable {
    pub fn begin_snapshot(&self) -> Snapshot { /* snapshot_txn = self.next_txn (last committed) */ }

    pub fn read(&self, snap: &Snapshot, key: Key) -> Option<ScanValue> {
        // SPIKE-FR-003: current value if last_write_txn <= snap.snapshot_txn,
        // else the log entry with the smallest superseded_by_txn > snap.snapshot_txn.
    }

    pub fn stage(&self, snap: &mut Snapshot, key: Key, value: ScanValue) { /* local only */ }

    pub fn commit(&mut self, snap: Snapshot) -> Result<(), MvccConflict> {
        // SPIKE-FR-004: check every staged key's live last_write_txn against
        // snap.snapshot_txn first; on any conflict, apply nothing, return Err.
        // Otherwise apply every staged write (SPIKE-FR-002: log the superseded
        // value, bump next_txn once per commit or once per op — implementation
        // picks and states which, either is defensible for a spike) and return Ok.
    }

    pub fn gc(&mut self, min_open_snapshot: Option<TxnId>) {
        // SPIKE-FR-005: prune log entries with superseded_by_txn <= min_open_snapshot
        // (or all entries, if min_open_snapshot is None).
    }
}
```

Exact field/method names, and whether a commit bumps `next_txn` once
per commit or once per staged op, are implementation's call — this
sketch fixes the *shape and guarantees*, not the byte-for-byte API.

## Acceptance criteria

1. A snapshot taken before a write to `(id, field)` still reads the
   pre-write value after that write commits elsewhere — proving real
   time-travel, not just "read your own writes."
2. Two sequential writes to the same key, with a snapshot taken
   between them, reads the value that was current *at* the snapshot,
   not the first or the last — proving the version-log chain-walk
   picks the correct interval, not just "the previous one."
3. A snapshot that stages a write to a key another commit has since
   changed is rejected whole at commit, with nothing applied — proving
   write-write conflict detection.
4. A snapshot that stages a write to a key nothing else has touched
   commits normally — no false conflicts.
5. `gc` with a `min_open_snapshot` at or above every commit that
   touched a key removes that key's now-unreachable log entries; `gc`
   with a `min_open_snapshot` below a still-needed entry leaves it
   untouched, and a read using a snapshot at or above that boundary
   still succeeds correctly afterward.
6. A read using a snapshot whose needed version has already been
   GC'd (i.e., the harness deliberately mis-uses it, simulating a bug
   this spike wants to catch, not a real code path) fails with a
   distinct, typed error — never silent wrong data, never a panic.
7. `cargo test -p rusty_multimodal_db --features research` covers all
   of the above; `cargo test -p rusty_multimodal_db --all-features`
   (i.e., without `research`) is unaffected — the spike module is
   invisible to a default/production build.
8. `cargo clippy -p rusty_multimodal_db --all-features -- -D warnings`
   and `cargo fmt -p rusty_multimodal_db -- --check` clean.

## Verification plan

- Unit tests inside the new spike module covering acceptance criteria
  1–6 directly, following `src/generic_spike/`'s own existing test
  style and naming conventions.
- No integration test, no `benches/` addition, no change to any
  existing test file — this unit touches nothing outside its own new
  module (plus docs).

## Traceability

- → `ADR-0071`'s "Acceptance and implementation," phase 1. Registers
  as `MVCC-SPIKE` in `docs/roadmap/ROADMAP.md` and
  `docs/PROJECT-STATUS.md`; no `SERVER-001`/`STORAGE-*` version bump
  (this is a spike, not a shipped unit) — full implementation, once
  authorized, registers its own.

## Open questions

- Whether the eventual full-implementation unit reuses this spike's
  in-memory harness shape directly inside `src/generic/**`, or finds a
  materially different approach once it has to answer to `SlotFile`'s
  real fixed-width mmap constraints — left to that unit, not
  prejudged here.
- Whether `next_txn` should be persisted per-commit or per-op in the
  real implementation (this spike may pick either, stating which, for
  its own harness only).

## Change history

- 2026-09-16: Initial proposal and acceptance, same session, following
  the owner's interactive scope pick (real MVCC only, single-table)
  and mechanism pick (side version-log + Compact-triggered GC) —
  `ADR-0071`. Implementation (this spike) delegated to Codex via
  `codex-build`, independently reviewed by Claude before merge.
