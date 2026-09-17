# ADR-0071: Real MVCC — versioned records, a snapshot pointer, and Compact-triggered garbage collection

- Status: **Accepted as designed** (owner picked this scope and mechanism
  interactively, 2026-09-16 — see "Acceptance and implementation" below).
  Acceptance authorizes the design and a bounded validation spike;
  full production implementation across `Memory`/`Entity`/`Relation`
  remains a separate, later unit contingent on the spike's findings —
  the same two-step precedent `GENERIC-SCHEMA-DESIGN`/`ADR-0009` set
  (four validation spikes in `src/generic_spike/` before promotion to
  a real library).
- Date: 2026-09-16
- Deciders: baileyrd
- Related: `ADR-0033` (optimistic read-set validation — explicitly
  declined "real MVCC" as its option 3, naming it "the correct general
  answer... rejected this round as disproportionate," with its own
  revisit trigger: "optimistic detection proves insufficient for a
  real workload... real MVCC becomes the next question"), `ADR-0024`/
  `ADR-0025`/`ADR-0027` (the session mechanism, journal, and
  read-your-writes this design extends without replacing),
  `ADR-0052` (Compact — the only existing precedent for reclaiming
  stale on-disk state, and the operation this design's GC piggybacks
  on), `ADR-0056` (schema-tag bump — the precedent for a record-layout
  version change), `ADR-0066` (schema migration tooling — the proven
  pattern for carrying old-tagged data forward across a layout bump),
  `docs/FUTURE-GROWTH.md`'s "Path to SQLite/DuckDB parity" §2
  (Transactions).
- Supersedes/Superseded by: none. Additive: a new fourth `BeginWith`
  bit alongside the three `ADR-0024`/`ADR-0027`/`ADR-0033` already
  established; `ADR-0033`'s optimistic mechanism is untouched and
  stays available under its own bit.

## Context

`docs/FUTURE-GROWTH.md` names, as still absent from this crate's
transaction story: "a general transaction manager — real MVCC
(versioned records, a snapshot pointer, garbage collection —
`ADR-0033`'s own explicitly-declined option), multi-table atomicity,
and staged (not single-shot) transactions over inserts, links,
replacements, and deletes." These are three separable gaps, not one.
The owner picked **real MVCC only** as this round's scope — multi-table
atomicity and staging non-field-update operations inside a session
remain explicitly out of scope, future rounds of their own.

Read against the actual code (not `ADR-0033`'s own now-imprecise
phrasing, which said real MVCC would be "the first round to touch
`src/store/**`"): `src/store/**` is the crate's original, small
(~750 lines), benchmark-era code, untouched by any server-facing round
to date and not what production traffic uses. The real production
engine is `src/generic/**` (13,000+ lines: `mmap_store.rs`,
`record_blob.rs`, `slot_file.rs`, etc.) plus `src/production.rs`. A
mutable field's current value lives in a fixed-width mmap slot
(`SlotFile`'s `(id, value, COMMITTED)` layout); `write_value`
overwrites a slot's value bytes **in place** — no prior value is ever
retained anywhere. This design is, honestly and for the first time,
a change to that real storage engine's on-disk format — the thing
`ADR-0033` avoided by scoping down to optimistic read-set validation
instead.

## Decision drivers

- The owner explicitly reopened `ADR-0033`'s declined option 3 by
  direct choice, not because optimistic detection was measured
  insufficient — named plainly, since `ADR-0033`'s own revisit trigger
  assumed the latter. This ADR does not claim evidence of real
  conflict-rate problems; it proceeds on explicit owner direction.
- Keep the mechanism itself proportionate even though the *decision*
  to build it is now made: no new background/maintenance thread (this
  crate has never had one — `ADR-0052`'s Decision text: "No threshold,
  no timer, no background thread: an operator's request"). Garbage
  collection reuses the existing explicit, foreground `Compact`
  operation rather than inventing new machinery.
- Additive protocol change, not a replacement: `ADR-0033`'s optimistic
  read-set validation stays exactly as shipped, under its own bit; a
  session can pick either mechanism (or neither), matching every prior
  `BeginWith` bit's own independence (`ISO-FR-007`'s precedent).
- Single-table scope, matching `apply_transaction`'s existing scope
  exactly (`ConnectionStore` has never coordinated more than one table
  in one commit) — multi-table atomicity is a distinct, larger problem
  left for its own future round.
- De-risk a genuine storage-format first via a bounded validation
  spike before committing to a shape across all three production
  domains (`Dog`/`Order`/`Employee` for `apply_transaction`'s existing
  session-bearing adapters) — the same discipline `GENERIC-SCHEMA-DESIGN`
  applied to a comparably novel, storage-adjacent change.

## Considered options (storage mechanism for retained versions)

1. **Side version-log + Compact-triggered GC — proposed.** The primary
   slot keeps only the latest value plus an added `last_write_txn: u64`
   column (the fast, non-MVCC read path is untouched — same slot,
   same layout otherwise). Every overwrite first appends the value it
   is about to replace, tagged with the committing txn id, to a small
   per-table append-only version-log file, inside the same
   exclusive/journal section the write already uses. A snapshot read
   consults the primary slot when `last_write_txn <= snapshot_txn`
   (the common case, zero extra I/O); otherwise it walks the version
   log for that `(id, field)` to find the value whose validity
   interval covers `snapshot_txn`. Old log entries are reclaimed only
   when `Request::Compact` runs, and only past the oldest still-open
   MVCC snapshot on that table — an explicit, foreground, operator-
   triggered operation, not a background sweep. **Tradeoff, named
   plainly:** version-log growth between `Compact` runs is unbounded
   while any long-running MVCC snapshot stays open; an operator who
   never compacts and holds sessions open indefinitely can grow the
   log without bound. Accepted as the honest cost of reusing an
   existing, proven reclamation trigger instead of inventing a new one.
2. **Bounded two-version slots, no GC at all.** Each field's slot
   holds `(current value, txn) + (previous value, txn)` directly in
   the mmap layout; no side file, nothing to reclaim, ever. Rejected
   this round: a snapshot older than exactly one commit behind on a
   hot field would need to fail closed (`Conflict`) rather than serve
   a true historical read, which is a materially weaker guarantee than
   "real MVCC" implies, and the owner's chosen option (1) already
   avoids that limitation at an accepted, bounded cost.
3. **Full unbounded MVCC with a real periodic background GC thread.**
   The textbook answer. Rejected this round: it would be the first
   background/maintenance machinery of any kind this crate has ever
   shipped, a materially larger and riskier addition than reusing
   `Compact`, for a benefit (automatic, unattended reclamation) no one
   has asked for yet.

## Decision

Proposed and accepted: option 1, scoped as follows.

- **A fourth `BeginWith` bit**, `SESSION_MVCC_ISOLATION = 8`, composing
  with the existing three (`1`, `2`, `4`); `PROTOCOL_VERSION` moves to
  the next integer past whatever `SERVER-002` currently pins (27 at
  time of writing — implementation confirms the exact current value
  and bumps from there, per `ADR-0022`'s own "a flag bit is introducing
  wire meaning" rule). Unknown/below-version bit stays `Malformed`,
  matching every prior bit's own precedent.
- **A per-table monotonic commit counter** (`TxnId = u64`), persisted
  in the table's existing header/metadata alongside `SCHEMA_VERSION`
  (not a new file), advanced durably inside the same exclusive+journal
  section an ordinary commit already uses — so a crash never loses or
  duplicates a txn id relative to what's actually on disk. On reopen,
  the counter resumes from its persisted value, never from a fresh
  scan.
- **A `last_write_txn: u64` column** added to each mutable field's
  slot layout (`SlotFile`'s format) — a real, on-disk schema-tag bump
  (`ADR-0056`'s precedent), refusing an old-tagged directory to open
  under the new code without going through `ADR-0066`'s proven
  migration pattern.
- **A per-table append-only version-log file**: entries
  `(record_id, field_ref, old_value, superseded_by_txn)`, appended
  before the primary slot's in-place overwrite, inside the same
  exclusive/journal section.
- **At `Begin` with `SESSION_MVCC_ISOLATION` set**: the session
  records `snapshot_txn` = the table's latest committed txn id at that
  instant, and registers it into a small per-table in-memory
  open-snapshot set (used only by `Compact`'s GC step to find the
  oldest still-needed version — never persisted, lost harmlessly on
  restart same as every other session-scoped state today).
  Deregistered at `Commit`, `Rollback`, or disconnect.
- **Reads while such a session is open** return, per field, the value
  visible as of `snapshot_txn`: the primary value if
  `last_write_txn <= snapshot_txn` (the common, zero-extra-I/O case);
  otherwise the version-log entry for that key whose validity interval
  covers `snapshot_txn` — the entry with the smallest
  `superseded_by_txn` strictly greater than `snapshot_txn`.
- **Write-write conflict check at `Commit`**: for every `(id, field)`
  the transaction's own staged `UpdateField` batch targets, compare
  the record's *current* `last_write_txn` to the session's
  `snapshot_txn`; if it has advanced (someone else committed a change
  to that key since this session's snapshot was taken), refuse the
  whole commit atomically, applying nothing — reported as
  `Response::TransactionFailed { index: 0, code: ErrorCode::Conflict }`
  (the same sentinel-index shape `ErrorCode::Journal`/`ADR-0033`'s
  `Conflict` already established; this design reuses `Conflict`
  rather than adding a new code, since the client-visible meaning —
  "your commit lost a race" — is identical). This check needs no
  separate read-set map: the per-slot `last_write_txn` column already
  added for versioning *is* the comparison target, which is strictly
  cheaper than `ADR-0033`'s explicit tracked-read map for the writes
  a transaction itself makes (though it does not, by itself, cover
  isolation for fields the session only *read* and never wrote —
  a session wanting that combines this bit with `ADR-0033`'s
  `SESSION_SNAPSHOT_ISOLATION`, exactly as any two independent bits
  compose today).
- **`Compact`'s GC extension**: when `Request::Compact` runs on a
  table, after finding the table's current minimum open-snapshot
  `txn_id` (or "none open," meaning every entry is reclaimable), prune
  every version-log entry whose `superseded_by_txn` is at or below
  that minimum. Entries needed by any still-open snapshot are never
  touched. This is the *only* reclamation path — there is no
  automatic or background pruning in this design.
- **Explicitly out of scope, this round**: multi-table atomicity
  (`apply_transaction` stays scoped to exactly one `ConnectionStore`,
  as today); staging `Insert`/`Link`/`Replace`/`Delete` inside a
  session (still refused with `SessionOpen`, as today);
  phantom-read protection for `FilterEq`/`ScanField` (matches
  `ADR-0033`'s own identical, already-named limitation); rollout to
  `Memory`/`Entity`/`Relation` (this round validates the mechanism
  against `Dog` only, in a gated spike — see "Acceptance and
  implementation").

## Consequences

### Positive

- Real snapshot isolation: a session sees a single consistent point in
  time for every field it reads, not just the ones it happened to
  `GetById` and remember (`ADR-0033`'s own named limitation on
  `FilterEq`/`ScanField`) — though this round's write-conflict check
  still only covers the transaction's own writes, not arbitrary reads,
  unless combined with `ADR-0033`'s bit.
- Composes cleanly with everything already shipped: reuses the
  existing exclusive+journal critical section for durability, the
  existing `Compact` operation for reclamation, and `ErrorCode::Conflict`
  for the client-visible failure shape — no new lock, no new sink, no
  new background thread.
- Named, bounded, and honest about its own cost (version-log growth
  between `Compact` runs) rather than promising unbounded time-travel
  it cannot actually reclaim automatically.

### Negative / tradeoffs

- **This is the first round to change the real production storage
  format** (`src/generic/**`'s `SlotFile` layout) — a materially
  larger and more hard-to-reverse change than any prior `SERVER-001`
  round, which is exactly why implementation proceeds as a gated
  validation spike first, not a direct rollout.
- **Version-log growth is unbounded between explicit `Compact` runs**
  under a long-lived open snapshot — an accepted, named cost of
  reusing `Compact` instead of new background machinery.
- **No multi-table atomicity, no staged non-field writes** — a session
  combining `SESSION_MVCC_ISOLATION` with an attempted `Insert`/`Link`/
  `Replace`/`Delete` still gets `SessionOpen`, unchanged; these remain
  real, separately-scoped gaps.
- **Two isolation mechanisms now exist** (`ADR-0033`'s optimistic
  read-set validation and this design's MVCC), each under its own bit,
  rather than one unified answer — an accepted cost of additive,
  non-breaking protocol evolution over consolidation.

## Validation and revisit triggers

- **Not design-only-with-no-probe**, unlike most prior `SERVER-001`
  ADRs: this is new storage-format machinery, so implementation begins
  with a bounded validation spike (see below) before any production
  rollout commitment, matching `GENERIC-SCHEMA-DESIGN`'s own precedent
  for the last comparably novel change.
- Revisit if the spike finds the side-version-log's chain-walk cost
  unacceptable for a snapshot far behind the latest commit — a
  different retention shape (e.g. periodic snapshotting rather than
  per-write chaining) would need its own design.
- Revisit if multi-table atomicity or staged non-field writes are
  wanted later — each is its own future ADR, not a smaller extension
  of this one.
- Revisit the "no background GC" choice if `Compact`-triggered
  reclamation proves operationally insufficient (log growth outpaces
  how often operators actually compact) — real periodic GC becomes the
  next question, not a smaller tweak to this design.

## Acceptance and implementation

- Options offered and picked interactively, 2026-09-16: scope = real
  MVCC only, single-table (multi-table atomicity and staged
  non-field-update session ops explicitly declined this round);
  mechanism = side version-log with Compact-triggered GC (bounded
  two-version slots and full background-GC MVCC both declined).
- Implementation proceeds in two phases:
  1. **`MVCC-SPIKE`** (this unit): a gated, non-production validation
     module proving the core mechanism — versioned writes, a
     snapshot-consistent read, write-write conflict detection, and
     GC that reclaims only what no open snapshot needs — against a
     minimal harness modeled on the `Dog` domain, behind the existing
     `research` feature flag, alongside `src/generic_spike/`'s own
     precedent. Not wired into `src/server/**`, the real wire protocol,
     or any shipped binary. See
     `docs/design/MVCC-SPIKE-DESIGN.md` for the concrete shape and
     acceptance criteria.
  2. **Full implementation** (a later, separate unit, contingent on
     the spike's findings): wiring `SESSION_MVCC_ISOLATION` into
     `src/server/**` for real, across `Dog`/`Order`/`Employee`, with
     its own protocol version bump, integration tests, and
     `Compact` GC extension. Not authorized by this ADR alone; requires
     its own implementation-unit sign-off once the spike reports back.
