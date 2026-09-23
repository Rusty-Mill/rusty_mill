# ADR-0072: Real MVCC, phase 2 — production wiring for Memory/Entity/Relation, folded into the existing insert log and redo journal

- Status: **Accepted as designed** (owner picked scope and mechanism
  interactively, 2026-09-17 — see "Acceptance and implementation" below).
  Authorizes real implementation, no further spike — `ADR-0071`'s phase 1
  spike (`MVCC-SPIKE`, PR #233, merged) already validated the core logic
  (versioned writes, snapshot reads, write-write conflict detection,
  bounded GC) in isolation; this round wires the same, now-proven logic
  into `src/server/**` for real.
- Date: 2026-09-17
- Deciders: baileyrd
- Related: `ADR-0071` (phase 1 — the decision this ADR extends and
  partially revises; see "Supersedes" below), `docs/design/MVCC-SPIKE-DESIGN.md`
  (the validated mechanism this round wires up for real), `ADR-0033`
  (optimistic read-set validation — untouched, stays available under its
  own bit), `ADR-0025`/`ADR-0026` (the crash-recovery journal — the
  replay-to-reconstruct-state precedent this round's sidecar log reuses),
  `ADR-0052` (Compact — the operation this round's GC extends at the
  adapter level, not the generic trait level), `ADR-0063` (`WriteBatch`
  crash-atomicity on `Memory`/`Entity`/`Relation` — confirms these three
  adapters' journaled-write maturity).
- Supersedes/Superseded by: **partially supersedes `ADR-0071`'s Decision
  section** on the storage mechanism only — that section proposed
  widening `SlotFile`'s own slot layout to add a `last_write_txn` column
  (a real on-disk format bump requiring migration tooling for every
  table). Implementation-level research (this round) found `SlotFile`'s
  header has no spare space for this, meaning that approach would force
  a `SlotFile::SCHEMA_VERSION` bump — a materially bigger, migration-
  requiring change than `ADR-0071` characterized it as, and one with no
  existing migration tool (`ADR-0066`'s tooling targets an unrelated
  versioning mechanism, `SchemaTag`, not `SlotFile::SCHEMA_VERSION`).
  This ADR replaced that one design point with a fully separate sidecar
  file — and then, after further implementation-level research
  (see "Acceptance and implementation," round four) found that an
  *independently*-durable sidecar creates its own unclosed two-phase-
  commit gap against the primary write, revised again to fold MVCC
  before-images directly into the insert log's (`src/generic/insert_log.rs`)
  and the redo journal's (`src/server/journal.rs`) own existing durable
  entries instead (see "Decision"). `ADR-0071`'s Context, Decision drivers, and
  Considered options (the *choice* to build real MVCC via a side log
  plus `Compact`-triggered GC, as opposed to bounded slots or a
  background-GC thread) are unchanged and still govern. `ADR-0071` also
  assumed sessions existed only on `Dog`/`Order`/`Employee`; this round's
  research found all six adapters (`Dog`/`Order`/`Employee`/`Memory`/
  `Entity`/`Relation`) fully support sessions and `apply_transaction`
  today — that assumption is corrected here, not superseded, since
  `ADR-0071` made no scope decision that depended on it being false.

## Context

`ADR-0071` authorized a bounded validation spike (`MVCC-SPIKE`) before
any production commitment, given this was new storage-format territory.
That spike is merged (PR #233) and every acceptance criterion held:
versioned writes, snapshot-consistent reads (including the
pre-creation-absence and chain-walk-picks-the-right-interval cases),
whole-batch write-write conflict detection, and minimum-snapshot-bounded
GC with typed errors for reclaimed history — all proven in an isolated,
in-memory harness.

Scoping the real implementation surfaced two corrections to `ADR-0071`'s
own assumptions, both confirmed by direct code reading:

1. **`SlotFile`'s on-disk header has no spare space** (an 8-byte magic
   plus a 4-byte `SCHEMA_VERSION`, nothing else) — widening every slot to
   add a `last_write_txn` column, as `ADR-0071`'s Decision section
   proposed, means every table using `SlotFile` needs a real format
   migration, whether or not it ever uses MVCC. A fully separate sidecar
   file gives the identical guarantee — a durable, replayable record of
   each key's last-write txn and its superseded values — with **zero
   change to `SlotFile`, zero migration burden on existing data, and
   zero cost for a table that never opens an MVCC session.**
2. **Sessions and `apply_transaction` are not scoped to three adapters.**
   All six (`Dog`/`Order`/`Employee`/`Memory`/`Entity`/`Relation`) already
   implement the identical `with_exclusive`/`validate_batch`/
   `check_read_set`/`apply_batch` shape. `Memory`/`Entity`/`Relation` are
   the domains the real consumer (`rusty_remind_me`) actually uses;
   `Dog`/`Order`/`Employee` are this crate's original validation domains.
   The owner picked `Memory`/`Entity`/`Relation` as this round's scope.

## Decision drivers

- Zero migration burden and zero risk to existing non-MVCC data — the
  sidecar file is purely additive per table; a table that never opens an
  MVCC session never creates one.
- Reuse this crate's own established replay-to-reconstruct-state pattern
  (the crash-recovery journal, `ADR-0025`/`ADR-0026`) for the sidecar's
  recovery story, rather than inventing new persistence machinery.
- `Compact` is implemented across roughly ten composable store-wrapper
  layers (`src/generic/{store,mmap_store,mmap_scanned}.rs`); threading a
  new GC-boundary parameter through all of them, as `ADR-0071`'s "extend
  Compact" phrasing implied, would be a materially larger and riskier
  change than reusing the existing operation's *name* suggests. GC
  instead lives as new, standalone adapter-level logic (in each of
  `memory.rs`/`entity.rs`/`relation.rs`'s own `compact()`), never
  touching the generic `Compact` trait or its other implementors.
- Scope to the real consumer's domains (`Memory`/`Entity`/`Relation`)
  rather than the original three validation domains — proportional to
  actual need; `Dog`/`Order`/`Employee` remain a future round if ever
  wanted, not because they're harder, but because nothing needs them yet.
- Insert the write-write conflict check and version-log append at the
  exact point `src/server/{memory,entity,relation}.rs`'s
  `apply_transaction` already runs `check_read_set` (`ADR-0033`) — the
  same critical section, one more step, no new locking story.

## Decision

- **A table becomes "MVCC-active" the instant any connection's `Begin`
  first sets `SESSION_MVCC_ISOLATION` on it, for the table's remaining
  lifetime (durable across restarts — see the next bullet).** This is a
  real, deliberate revision from this ADR's own first draft, made after
  implementation-level research found that limiting bookkeeping to the
  session-commit path alone cannot deliver a correct snapshot guarantee:
  an ordinary (non-session) `Insert`/`Replace`/`Delete`/`Link`/
  `WriteBatch` — which any connection may issue at any time, whether or
  not it has ever heard of MVCC — never stamped a txn id under that
  narrower design, so a concurrent ordinary write to a key an open
  snapshot had already read (or a key created after the snapshot began)
  would be silently, incorrectly visible or invisible. **Once a table is
  MVCC-active, every mutation to it — an ordinary `Insert`/`Replace`/
  `Delete`/`Link`/`WriteBatch` exactly as much as a session `Commit` —
  stamps the affected key(s)' current last-write txn and appends a
  sidecar entry for whatever value(s) it superseded**, using one freshly
  assigned txn id per mutating request (a session `Commit`'s whole
  staged batch counts as one; a `WriteBatch`'s whole batch counts as
  one; a single ordinary `Insert`/`Replace`/`Delete`/`Link` counts as
  one). **A table that has never gone MVCC-active is completely
  unaffected — zero new file, zero stamping, zero cost, byte-for-byte
  today's behavior** — the cost line moves from "per session-commit" to
  "per table, once ever used," not "always, for every table."
- **Round eight correction: activation needs a one-time baseline scan.**
  A record that already existed before a table's first `Begin` has no
  chain entry at all — without a fix, it would read as "not found"
  under MVCC, and even its first post-activation write leaves no way to
  recover its true pre-activation value for an earlier snapshot. Fix:
  at first `Begin`, under the table's existing exclusive lock, scan
  every currently-live record once and write one baseline entry per
  record (reserved `txn_id = 0`) into the persistent store — a one-time,
  proportional-to-table-size cost, the same "scan everything under the
  write lock" shape `Compact` already uses. See
  `docs/design/MVCC-PRODUCTION-DESIGN.md`'s `MVCC2-FR-001` for the exact
  requirement.
- **Revised, real correction from this ADR's own second draft: MVCC's
  before-images are embedded directly in the durable writes this crate's
  storage engine already makes for its own crash safety, not appended to
  a separately-durable sidecar file.** Implementation-level research
  (round 4 of this ADR's own scoping) found that a genuinely separate
  sidecar file creates an unclosed two-phase-commit gap: a crash (or a
  failed append) between the primary write's own durability and the
  sidecar's durability leaves them permanently inconsistent — corrupting
  either a key's MVCC metadata or the sidecar's own claimed history, with
  no existing mechanism to reconcile them. Two durability layers already
  exist in this crate and already solve exactly this class of problem for
  the *primary* write itself:
  - **`src/generic/mmap_store.rs` + `src/generic/insert_log.rs`** — the
    per-operation log an ordinary (non-journaled) `insert`/`replace`/
    `delete` already appends and syncs *before* touching the slot.
    Its entries are already kind-tagged and length-prefixed
    (`[kind: u8][len: u32][payload]`) — the exact mechanism that already
    let this format grow from version 1 to version 2 (`ADR-0051`)
    without breaking old files. `replace`/`delete` already read the
    record's prior value immediately before this log append (needed for
    their own index maintenance) — capturing it for MVCC here is not an
    extra read, just an extra field on an append that already has the
    value in hand.
  - **`src/server/journal.rs`'s `JournalEntry`** — the redo entry a
    session `Commit`, atomic-mode `WriteBatch`, or journaled ordinary
    write already appends and syncs before its adapter closure runs.
    Its entries are also kind-tagged (`0`/`1` today, added at
    `JOURNAL_FORMAT_VERSION` 2, `ADR-0063`) inside a strict, refuse-on-
    mismatch header version — unlike the insert log's silent-upgrade
    policy. **New MVCC-carrying entry kinds (not a `JOURNAL_FORMAT_VERSION`
    bump)** — e.g. kind `2`/`3` alongside today's `0`/`1` — pair the
    existing `TransactionOp`/`WriteOp` payload with `(new_value, txn_id)`
    for each key the operation writes (round seven's revision: **no
    before-image, no read of prior state at all** — capturing a
    before-image via a pre-append read was found unsafe against
    concurrent journaled writers; see "Acceptance and implementation,"
    round seven. The `txn_id` reuses `commit_entry`'s own already-
    mutex-protected, already-monotonic `seq`). A table that never goes
    MVCC-active never produces these new kinds; its journal is
    untouched, at version 2, forever, exactly as today.
  - **Both replay paths already exist and are already idempotent by
    construction** — `merge_log`'s fold (insert log) overwrites/removes
    by id, so replaying an already-applied entry is a no-op; `with_journal`'s
    open path already re-applies a durable entry via `apply_batch`/
    `replay_write_batch`. This round extends both: when a replayed entry
    carries an MVCC payload, replay folds that payload into the MVCC
    version-index/history in the *same* pass it already uses to fold the
    primary state — using the entry's own embedded txn id, checked
    against what the index already has recorded, so a repeated replay of
    an already-folded entry is a no-op there too.
  - **Correction, round six: the insert log and the journal are not
    permanent — a genuinely persistent MVCC history store is still
    needed, but only flushed at existing checkpoint/compact boundaries,
    never per-write.** Round five's own "the sidecar is just a
    rebuildable cache, folded from the logs on open" framing is false:
    `mmap_store.rs::compact()` (`Compact`, `ADR-0052`) already calls
    `insert_log::clear` — which deletes the insert log file outright —
    once its entries are safely reflected in a freshly rewritten primary
    blob and slot file; `with_journal`'s open-path replay and the normal
    group-commit checkpoint both already call `journal.truncate()` once
    primary state is confirmed durably flushed. Once either reclamation
    happens, the MVCC payloads that were living in that log's own
    entries are gone, and nothing else durably remembers them — a real
    correctness loss, not a performance one, for `last_committed_txn`,
    every key's stamps, and any retained history a still-open (or
    future) snapshot would need. **The fix**: a real, durably-persisted,
    per-table MVCC history store exists after all (call it what
    implementation likes; not literally the same design as this ADR's
    first, per-write-appended draft) — but it is flushed only at the
    exact points these two reclamations already happen, using the exact
    lock/sequencing those points already provide, not on every write:
    - **Journal checkpoint** (`journal.rs`'s `commit_entry`, and the
      identical shape in each adapter's `with_journal` open-path
      replay): the adapter's own closure already computes
      `flushed = checkpoint_due && inner.checkpoint_flush().is_ok()`
      inside the same exclusive section `apply_batch` runs in, and
      `commit_entry` truncates the journal *only if* `flushed` is true
      — extend that same closure to also durably flush any pending
      MVCC payloads from the journal entries just applied into the
      persistent MVCC store, and fold *that* step's success into the
      same `flushed` boolean. If the MVCC flush fails, `flushed` is
      `false`, the journal is not truncated, and those entries remain
      for the next attempt — the exact same resilience the existing
      `checkpoint_flush()` gate already provides for primary durability,
      extended to cover MVCC, not a new mechanism.
    - **`Compact`** (`mmap_store.rs::compact()`): its existing first
      step already reads every insert-log entry (today, only to count
      them) before writing the fresh blob, rewriting slots, and calling
      `insert_log::clear` — extend that same read pass to also extract
      and durably flush each entry's MVCC payload into the persistent
      store, *before* any of the destructive steps that follow it,
      inside the same write-lock `compact()` already holds (confirmed:
      `memory.rs`'s own doc comment states `compact()` already runs
      "under the store's own write lock").
    - **Idempotency, not a new invariant**: a crash between "MVCC flush
      succeeded" and "truncate/clear completed" leaves the source log
      still present, so it gets folded/replayed again on the next open —
      re-flushing already-recorded txn ids into the persistent store
      must be a safe no-op (check-before-add), exactly the same
      idempotency this round's per-write folding (see above) already
      requires, and exactly the reasoning `mmap_store.rs`'s own existing
      doc comment already gives for why a crash before `insert_log::clear`
      completes is safe for primary-state recovery: "the next open's
      `merge_log` folds the log again, and that refold is already
      required to be idempotent." Nothing new to invent here — the same
      property, applied to one more piece of state.
    - **Recovery, restated**: table open loads the persistent MVCC store
      (everything already checkpointed/compacted) *and* folds/replays
      whatever remains in the insert log/journal (everything since the
      last checkpoint/compact — may overlap with what the store already
      has; the idempotency above makes that safe) — combining both gives
      the complete picture, never relying on either alone.
  - **Round eight correction: `mmap_store.rs::open()` also calls
    `insert_log::clear`, after its own `merge_log` fold** — every table
    reopen, including an ordinary server restart, not only an explicit
    `Compact` request. The identical flush-before-clear step must also be
    added to `open()`'s own fold, or a restart silently loses whatever
    MVCC history the insert log was still holding.
- **Recovery, on table open**: load the persistent MVCC store (everything
  already checkpointed/compacted away from the logs) and fold whatever
  remains in the insert log (`merge_log`, extended to also feed the
  version-index from MVCC-kind entries) and replay the journal (extended
  the same way) for anything since the last checkpoint/compact —
  combined, these reconstruct (a) `last_committed_txn` — the highest
  MVCC txn id observed across the store and both logs, `0` if none has
  ever committed (the next commit itself computes its own id as
  `last_committed_txn + 1`, exactly as
  `src/generic_spike/mvcc_spike.rs`'s own proven `commit()` already
  does — do not separately persist or reconstruct a "next-unused" value
  as a distinct field; nothing should compare a snapshot against it
  directly, see the `Begin` bullet below), (b) each key's current
  last-write txn, and (c) the version chain needed to answer a snapshot
  read behind the current value. No persistent store and no MVCC-kind
  entries anywhere means no MVCC history exists yet — an empty
  reconstruction (`last_committed_txn = 0`), not an error.
- **A table that has never gone MVCC-active behaves byte-for-byte as it
  does today.** `SlotFile`'s format, every existing file, and every
  read/write path are completely unchanged until the table's first ever
  MVCC `Begin`.
- **A fourth `BeginWith` bit**, `SESSION_MVCC_ISOLATION = 8`, composing
  with the existing three; `PROTOCOL_VERSION` moves from 26 to 27,
  matching every prior bit's own version-gating precedent (`ADR-0022`).
  Below version 27, the bit is `Malformed`, matching every prior bit.
- **`Begin` with this bit set marks the table MVCC-active in memory
  immediately, before any file exists** (necessary so that an ordinary
  write from a *different* connection, landing after this `Begin` but
  before this session's own first mutation, is still correctly stamped
  and later seen as a conflict/change by this snapshot — activation
  cannot wait for the sidecar file's lazy creation). `snapshot_txn` =
  the table's current `last_committed_txn` (the *last-committed*, not
  "next-unused," id — using a next-unused value here with the spike's
  own inclusive `last_write_txn <= snapshot_txn` read comparison would
  expose the very next commit made by anyone else after `Begin` as if
  it had already happened before the snapshot, a real off-by-one this
  ADR explicitly rejects; `src/generic_spike/mvcc_spike.rs`'s own
  `next_txn` field already holds exactly this "last committed" value
  despite its misleading name — implementation must not rename or
  reinterpret it). Registered into a small per-table in-memory
  open-snapshot set, exactly as `ADR-0071` already decided (never
  persisted, lost harmlessly on restart, used only by `Compact`'s GC
  (*since `ADR-0105`, by the automatic reclaim on every write path too*)
  step). Deregistered at `Commit`, `Rollback`, or disconnect.
- **Reads while such a session is open** use the identical logic
  `MVCC-SPIKE-DESIGN.md`'s `read()` already proved (primary value if its
  last-write txn is at or before the snapshot; otherwise the version-log
  entry whose interval covers it; a pre-creation snapshot sees absence;
  a reclaimed-but-needed interval is a typed error, never silent wrong
  data) — no change to that logic, just backed by the sidecar's
  reconstructed index instead of pure runtime state, and now correctly
  fed by *every* write path's stamping, not only session commits.
- **Every mutation records `(key, new_value, txn_id)` — no before-image,
  no read of any prior value — made durable as part of the exact same
  write the operation already makes for its own crash safety, never a
  separate append.** Round seven's own finding, superseding this ADR's
  earlier "capture the before-image via the caller's existing pre-append
  read" text: that read (`validate_batch`'s pre-append read, on the
  journaled path) runs *before* the exclusive section that actually
  serializes writers against each other, so two concurrent journaled
  writers to the same key could both read the same stale value there,
  and whichever entry applies second would durably record the wrong
  before-image — a real, verified bug. **Fix**: don't read prior state
  at all. Each write's own new value is already known at construction
  time; the only thing that must be assigned at a genuinely serialized
  point is the txn id itself. **Round eight correction**: that
  serialized point is *not* `journal.rs`'s own internal `seq`
  (`state.appended`) as first proposed — `seq` resets to `0` on every
  process restart and is scoped to the journal only, while some
  ordinary writes bypass the journal entirely through a separate,
  uncoordinated path into `mmap_store.rs`/`insert_log.rs` (confirmed
  directly). Reusing it would duplicate ids across restarts and collide
  between the two paths. Instead: a single, shared, per-table
  `AtomicU64` (not `journal.rs`'s own counter) is the txn-id source for
  both paths, seeded once at table open from the reconstructed state
  (never reset mid-lifetime), incremented via `fetch_add` inside
  whichever lock each path already holds when it constructs its durable
  entry — no new lock, no new critical section beyond what each path
  already needs, no change to the group-commit pipeline `ADR-0026`
  tuned. See `docs/design/MVCC-PRODUCTION-DESIGN.md`'s `MVCC2-FR-002`
  for the exact mechanics. The in-memory write-write conflict
  check (comparing each target's current last-write txn — the newest
  chain entry's `txn_id` — to the session's `snapshot_txn`; any advance
  refuses the whole commit, applying nothing, reported as
  `Response::TransactionFailed { index: 0, code: ErrorCode::Conflict }`,
  `ADR-0033`'s existing code, reused) still runs inside the existing
  exclusive section immediately before `apply_batch`, exactly where
  `check_read_set` already runs — a pure in-memory decision, unaffected
  by any of the above. Ordinary writes have no session and thus no
  `snapshot_txn` to conflict against — they simply record their chain
  entry unconditionally, exactly as a session's own validated writes do.
- **`Compact` now does two related but distinct things for an MVCC-active
  table, adapter-level, not a change to the generic `Compact` trait or
  any of its other implementors.** (1) **Durability**: before
  `insert_log::clear` runs, flush any not-yet-persisted MVCC payloads
  from the insert log into the persistent store (see the storage-
  mechanism revision above) — this is not optional and is not "GC," it
  is what makes `Compact` safe to run on an MVCC-active table at all.
  **Round eight correction: `mmap_store.rs::open()` also calls
  `insert_log::clear`, after its own `merge_log` fold** — every table
  reopen, including an ordinary server restart, not only an explicit
  `Compact` request. The identical flush-before-clear step must also be
  added to `open()`'s own fold, or a restart silently loses whatever
  MVCC history the insert log was still holding.
  (2) **Reclamation (GC proper)**: given the table's current minimum
  open-snapshot txn id (or "none open"), for each key's chain drop every
  entry older than the newest one at or below that boundary — but the
  rewrite must retain `last_committed_txn` and, for every key currently
  live in the primary store, its newest surviving chain entry; this
  baseline is no longer reconstructable purely by re-folding the insert
  log once `Compact` has cleared it, so retaining it here is a real
  correctness requirement again, not defense in depth. Both steps are
  written using the same write-to-temp-then-atomic-rename discipline
  `STORAGE-014`/
  `ADR-0052` already established for the primary data files, under the
  same write lock `compact()` already holds. `Compact`-triggered
  reclamation is the *only* GC path — no background thread. The journal's
  own separate checkpoint/truncate cycle needs the equivalent durability
  step (see above) but has no separate GC step of its own to add here —
  its truncate already only removes entries once they're durably
  reflected elsewhere.
- **Domain scope: `Memory`, `Entity`, `Relation` only, this round.**
  `Dog`/`Order`/`Employee` — confirmed equally capable of the identical
  wiring — remain an explicit future round if ever wanted.
- **No change to `SlotFile`'s on-disk format, no `SCHEMA_VERSION` bump,
  no `SchemaTag` bump, no migration tooling needed** — the sidecar file
  is the entire storage-format footprint of this round.
- **Explicitly out of scope, unchanged from `ADR-0071`, and distinct
  from full mutation-path coverage above**: multi-table atomicity, and
  *staging* `Insert`/`Link`/`Replace`/`Delete` as part of a session's own
  batch (still refused with `SessionOpen` — a session may only stage
  `UpdateField`, exactly as today). This is not in tension with ordinary,
  non-session `Insert`/`Link`/`Replace`/`Delete` calls now being
  MVCC-stamped when the table is active — those are a different code
  path (never inside a session) and always were allowed to run
  concurrently with an open session; they simply now participate
  correctly in that session's snapshot instead of silently bypassing it.
  Also unchanged: phantom-read protection for `FilterEq`/`ScanField`,
  and any change to `Dog`/`Order`/`Employee`.

## Consequences

### Positive

- Zero migration burden, zero format change to `SlotFile` or any table
  that never goes MVCC-active — the single biggest risk `ADR-0071`'s own
  storage-mechanism proposal carried is eliminated entirely, and the
  independent-sidecar two-phase-commit gap `ADR-0072`'s own second draft
  had is eliminated by construction, not papered over with a recovery
  protocol bolted on after the fact.
- Reuses the exact durability and replay machinery this crate already
  runs for its own crash safety (the insert log's kind-tagged entries
  and idempotent fold-on-open; the redo journal's kind-tagged entries
  and its own open/replay path; `checkpoint_flush()`'s own
  flush-before-truncate gate) for the per-write case, rather than
  inventing a bespoke two-phase reconciliation protocol of this round's
  own for it. The persistent MVCC history store this round still needs
  (for the checkpoint/compact case — see "Decision") is flushed only at
  those existing, infrequent boundaries, not per write, so it never
  reopens the two-phase-per-write gap this ADR's second draft had.
- Scoped to the domains that matter to the real consumer, not the
  original validation domains — proportional effort.
- `Compact`'s generic trait surface and its other nine implementors are
  completely untouched — GC risk is contained to three adapter files.

### Negative / tradeoffs

- **The insert log and the redo journal both gain new entry kinds** —
  a real, additive format extension to two pieces of foundational,
  widely-shared storage machinery (`src/generic/insert_log.rs`,
  `src/server/journal.rs`), not a change confined to one new file type.
  Backward compatible and zero-cost for a table that never goes
  MVCC-active (its logs never produce the new kinds), but a real
  increase in the surface of two files every future change to crash
  recovery must now account for.
- **A real, durably-persisted MVCC history store still exists after
  all** — round four's "it's just a rebuildable cache, never a
  correctness concern" framing was wrong (round six's own finding: the
  insert log and journal are not permanent, so something durable and
  longer-lived must outlive their own reclamation). It grows roughly
  one entry per commit, not only per superseded key, reclaimed only at
  the next `Compact` — the same accepted "unbounded between explicit
  Compact runs" cost `ADR-0071` already named. *Note (`ADR-0096`, 2026-09-21):
  until that round no `Compact` actually called `MvccIndex::gc`; it
  does now, at the oldest open snapshot, before the flush.* Unlike a naive per-write
  sidecar, though, it is only *flushed* at existing checkpoint/compact
  boundaries (piggybacking on locks and gates that already exist), not
  on every write — bounding the durability cost even though the
  eventual on-disk size cost remains.
- **Two new coordination points, not one.** Beyond the per-write
  before-image embedding (round four), `Compact` and the journal's
  checkpoint path both gain a "flush pending MVCC entries before
  reclaiming the source log" step — a real, if narrowly-scoped, addition
  to two more pieces of foundational, already-shipped machinery, past
  the insert-log/journal entry-kind additions already named above.
- **`Dog`/`Order`/`Employee` do not get this capability in this round**
  — an explicit, deliberate scope line, not a technical gap; revisit if
  ever wanted.
- **Materially larger implementation than this ADR's own first draft.**
  Full-table coverage (see "Decision" above) means every mutation path —
  `Insert`/`Replace`/`Delete`/`Link`/`WriteBatch`, not only session
  `Commit` — needs the stamping-and-log-append step in all three
  adapters, not one insertion point. This is a deliberate, informed
  revision (the owner picked full correctness over a narrower,
  session-only guarantee once the narrower version's real weakness was
  found), not scope creep discovered after the fact.
- **Once a table is MVCC-active, every ordinary write to it pays a
  small, permanent bookkeeping cost** (a counter read/increment and a
  sidecar append) for the table's remaining lifetime, even from
  connections that never open a session — the direct cost of the
  full-correctness guarantee. A table that never goes MVCC-active pays
  nothing, and this cost cannot be turned off once incurred (no
  "un-opt-in" path is provided or needed).
- **A pre-existing crash-recovery property, not new to this round, now
  matters more.** Reading `src/server/journal.rs`'s `commit_entry` and
  each journaled adapter's own `with_journal` replay path directly
  (confirmed against `memory.rs`) shows the redo entry for a batch is
  appended and made durable *before* the adapter's own validating
  closure runs — and on replay after a crash, a journaled
  `JournaledBatch::Transaction` entry is re-applied via `apply_batch`
  **directly**, without re-running `check_read_set` or any other
  validation the live commit path performs. This means `ADR-0033`'s own
  optimistic read-set validation already has a narrow theoretical
  window — a live commit rejected for a read-set conflict leaves a
  durable, untruncated redo entry that a subsequent crash-before-next-
  truncate could replay as if it had succeeded — that this round's
  write-write conflict check inherits identically, at the same
  insertion point. This is **not a new defect this round introduces**,
  and this round does **not** attempt to close it — doing so would mean
  changing replay to re-run the full validating closure instead of
  `apply_batch` alone, a materially larger change to already-shipped
  crash-recovery machinery than this round's own scope, and one that
  would need its own ADR covering `ADR-0033` too, not just MVCC. Named
  here for honesty, not silently accepted as already fixed; a real
  finding from this round's implementation-level research, not
  speculation. Unaffected by this ADR's later fold-in-the-existing-logs
  revision (round 4): whether a journal entry carries an MVCC before-image
  or not, a *rejected* commit's entry is durable-but-unapplied either way,
  and this property concerns only that narrow case — it is orthogonal to,
  and not fixed or worsened by, folding MVCC into the entry format.

## Validation and revisit triggers

- No further spike: `MVCC-SPIKE`'s isolated validation already covers
  the core read/write/conflict/GC logic this round wires up for real:
  what's new here is *integration* (the sidecar file's replay-recovery
  mechanics, the critical-section insertion point, and `Compact`'s
  extension), not the logic itself.
- Revisit the domain scope if `Dog`/`Order`/`Employee` are ever wanted —
  the identical wiring pattern applies; a future round, not a surprise.
- Revisit the sidecar file's per-commit (not per-superseded-key) entry
  granularity if it proves too space-hungry for a real insert-heavy
  workload — a real measurement question, not decided here.
- Revisit `ADR-0071`'s "no background GC" choice under the same
  condition it already named: if `Compact`-triggered reclamation proves
  operationally insufficient.
- Revisit the shared replay-does-not-revalidate window (see "Negative /
  tradeoffs" above) if it is ever actually hit in practice, or if
  `ADR-0033`'s own mechanism is revisited for the same reason — the two
  should be fixed together, not separately, since they share the exact
  same root cause.

## Acceptance and implementation

- Options offered and picked interactively, 2026-09-17: storage
  mechanism revised to a sidecar file (supersedes `ADR-0071`'s
  SlotFile-widening proposal); domain scope = `Memory`/`Entity`/
  `Relation` (`Dog`/`Order`/`Employee` deferred, all six confirmed
  equally capable).
- 2026-09-17, same session: two rounds of Codex-found specification
  conflicts during implementation scoping, each an advisory report with
  no code written, both independently verified by Claude by reading
  `src/server/journal.rs`/`memory.rs`/`entity.rs`/`relation.rs` directly.
  Round one: the snapshot-boundary off-by-one, the `Begin`-vs-`Commit`
  sidecar-creation contradiction, and the journal-ordering/replay window
  — all fixed by correcting this ADR's own wording, no scope change.
  Round two: empty-commit counter advancement and GC baseline retention
  (both fixed by correcting wording) plus a real, deeper finding —
  session-commit-only bookkeeping cannot deliver a correct snapshot
  guarantee against a concurrent *ordinary* (non-session) write, since
  that guarantee depends on every mutation to a key being stamped, not
  only mutations that happen to go through `apply_transaction`. Offered
  as a fork: full mutation-path coverage (bigger, correct) vs. an
  explicitly narrower, honestly-weaker guarantee (smaller, original
  scope). **Owner picked full mutation-path coverage** — see "Decision"
  above, now covering every `Insert`/`Replace`/`Delete`/`Link`/
  `WriteBatch` on an MVCC-active table, not only session `Commit`.
- Round three: a third Codex-found conflict, also advisory, also
  verified by direct code reading — per-op-mode `WriteBatch` already
  releases its lock and commits each operation independently
  (`ADR-0060`'s own picked option (a)), so requiring "one txn id per
  whole `WriteBatch`" was incompatible with that mode's own existing,
  already-shipped atomicity granularity. Resolved by a principled rule,
  not a special case: MVCC stamps and logs at exactly the granularity
  each write path already treats as atomic — a per-op-mode `WriteBatch`
  gets one txn id *per operation*, not per batch; nothing else changes.
  No critical section was widened; no existing concurrency behavior
  changes for any caller. See `docs/design/MVCC-PRODUCTION-DESIGN.md`'s
  `MVCC2-FR-002`.
- Round four: the deepest Codex-found conflict of the four, also
  advisory, verified this time by dedicated research into
  `src/generic/insert_log.rs`/`mmap_store.rs` and
  `src/server/journal.rs` before responding — a genuinely separate
  MVCC sidecar file has no coordinated durability with the primary
  write, so a crash (or a failed append) between the two leaves them
  permanently inconsistent, with no existing mechanism to reconcile
  them; this affects ordinary writes with no session or conflict check
  involved at all. **Owner picked the largest of four offered options:
  fold MVCC before-images directly into the insert log's and the
  journal's own existing durable entries** (both already have a
  kind-tagged, extensible format; `replace`/`delete` already read the
  prior value these entries need, for their own index maintenance) —
  eliminating the two-phase-commit gap by construction rather than
  protocol-negotiating around it. The sidecar/version-index becomes a
  rebuildable materialized cache of what these two logs already durably
  record, the same relationship the primary store's own in-memory state
  already has to the insert log. See "Decision" above (revised) and
  `docs/design/MVCC-PRODUCTION-DESIGN.md`'s `MVCC2-FR-001`/`002`/`003`/
  `008`/`009` (revised).
- Round five: a fifth Codex-found conflict, advisory, exposing that
  round four's "rebuildable cache" framing was itself incomplete — it
  didn't yet establish that the source logs are ever safe to reclaim
  once their entries are needed for MVCC. Superseded directly by round
  six's finding and fix below (rounds five and six are effectively one
  finding, resolved together).
- Round six: **the deepest and final finding.** Verified by dedicated
  research into `mmap_store.rs::compact()`'s and `journal.rs`'s exact
  checkpoint/truncate/clear sequencing before responding — the insert
  log and the redo journal are not permanent: `Compact` already deletes
  the insert log once its entries are reflected in a freshly rewritten
  primary blob; journal checkpoints already truncate the journal once
  primary state is confirmed durable. Round four's fold-in fix correctly
  closed the *per-write* two-phase gap, but said nothing about what
  happens to that same MVCC data once its home log is reclaimed —
  nothing else durably remembered it. **Resolved, not by picking either
  of Codex's two offered alternatives outright, but by recognizing they
  point at the same answer**: a real, durably-persisted MVCC history
  store is still needed (contra round four/five's "just a cache"
  framing), but it is flushed only at the exact points these two
  reclamations already happen — inside the same locks, gated by the
  same "flushed, therefore safe to truncate/clear" logic those points
  already use — never per write. This is not a new sidecar-file design
  reopening round four's mistake: the per-write durability fix stands
  unchanged; this adds one narrow, well-precedented step at two existing
  checkpoint/compact call sites. See "Decision" above (revised again)
  and `docs/design/MVCC-PRODUCTION-DESIGN.md`'s `MVCC2-FR-001`/`003`/
  `009`/`010` (revised).
- Round seven: the seventh Codex-found conflict, advisory, exposing a
  genuine race in round four's per-write fold-in — its before-image
  capture read the key's current value via `validate_batch`'s existing
  pre-append read, which runs *before* the exclusive section that
  actually serializes concurrent writers; two journaled writers to the
  same key could both read the same stale value, so whichever one
  applied second recorded a wrong before-image. Codex's own proposed
  fix (serialize before-image capture, txn assignment, append, and apply
  against other mutations) would have required relaxing the
  no-widened-critical-section constraint and revisiting the group-commit
  pipeline `ADR-0026` specifically tuned. **Resolved by a simplification
  instead, not by paying that cost**: entries stop carrying a
  before-image at all — each records `(key, new_value, txn_id)`, with
  the txn id reusing `journal.rs`'s own already-correctly-serialized,
  already-monotonic `seq` (`state.appended`), assigned under its own
  mutex before anything else, with the existing turn-gate already
  guaranteeing application order matches assignment order. No new lock,
  no new critical section, no cost to group-commit throughput. A
  snapshot read walks the chain for the newest entry at or below its own
  `snapshot_txn`; "not found," including pre-creation absence, falls out
  naturally as "no qualifying entry," with no separate creation-stamp
  field needed. See "Decision" above (revised again) and
  `docs/design/MVCC-PRODUCTION-DESIGN.md`'s `MVCC2-FR-002`/`006`/`008`
  (revised).
- Round eight: three more findings from continued implementation-scoping
  research, all advisory, all bounded fixes within the direction already
  chosen — no new tradeoff for the owner to weigh. (1) A table's
  pre-existing records had no chain entry at activation and would read
  as wrongly absent — fixed with a one-time baseline scan at first
  `Begin` (`MVCC2-FR-001`). (2) Round seven's `journal.rs` `seq` reuse
  was itself flawed — it resets on every restart and doesn't cover the
  separate, uncoordinated path some ordinary writes take around the
  journal entirely — fixed with a single shared, per-table `AtomicU64`,
  seeded at open from reconstructed state, incremented from whichever
  lock each path already holds (`MVCC2-FR-002`). (3)
  `mmap_store.rs::open()` clears the insert log too, not only `Compact`
  — the round-six flush-before-clear fix needed a second call site
  (`MVCC2-FR-010`). See "Decision" above (revised again) and
  `docs/design/MVCC-PRODUCTION-DESIGN.md`'s `MVCC2-FR-001`/`002`/`010`
  (revised), plus new acceptance criteria 13–16.
- Round nine: a ninth finding, surfaced by Claude during direct
  implementation (not Codex — Codex's sandbox was unusable by this
  point, see round ten below), verified before being accepted: round
  eight's own shared `AtomicU64` fix still left a race. Assigning
  `txn_id` at journal-append time (inside `journal.rs`'s own mutex, per
  round seven/eight) leaves a real gap before the journaled write
  actually applies (under the store's separate `RwLock`, once the
  turn-gate admits it) — during that gap, a concurrent *non-journaled*
  write (which assigns its id at apply time, under `RwLock` alone, no
  such gap) can be assigned a *higher* id and fully apply first. A
  snapshot opened in between would compute `last_committed_txn` from
  the higher-numbered bypass write already folded in, and when the
  lower-numbered journaled write later applies, its id would make it
  look like it happened *before* that snapshot even though it took
  effect afterward — a real serializability violation, exactly the
  shape `MVCC2-FR-015`'s cross-path ordering test exists to catch.
  **Fix, and a further simplification, not just a patch**: assign
  `txn_id` at *apply* time — inside the same `with_exclusive` critical
  section that already runs `apply_batch`/`apply_prepared` and the
  version-index update — for both paths uniformly, instead of at
  journal-append time. `txn_id` becomes a purely reconstructed
  quantity, like `last_committed_txn` already is per `MVCC2-FR-003`,
  re-derived deterministically by replaying entries in log order rather
  than a literal value embedded in the durable bytes. This removes the
  need for new insert-log/journal entry *kinds* entirely — replay only
  needs the table's own MVCC-active flag (one bit, reconstructed at
  open) to know whether to fold each entry into the version index,
  assigning ids in log order exactly as a live apply assigns them in
  `with_exclusive` order. `MVCC2-FR-002`, `008`, `009`, `010`, and the
  "Proposed shape" section are revised accordingly — the durable
  insert-log/journal entry format is now **unchanged** from today's;
  only the in-memory version index and the persistent MVCC history
  store (still flushed only at the three existing reclamation
  boundaries) are new.
- Round ten (implementation-time finding, Claude, direct implementation):
  confirmed by reading `GenericMmapStore::open` directly —
  `insert_log::clear` there runs unconditionally, before any
  server-layer code (including `MemoryConnectionStore::with_mvcc`) gets
  a chance to see the log's contents. This means a table with pending
  insert-log entries at the moment of a reopen (crash or ordinary
  restart) loses that history before MVCC's own reconstruction can run
  — a real gap in `MVCC2-FR-010` item 3's own restart-recovery story,
  deeper than "only when chained after `with_journal`." Closing it needs
  a hook threaded through `GenericMmapStore::open` itself — a
  generic-layer change shared by every domain in this crate (`Dog`/
  `Order`/`Employee` included), deliberately not attempted without its
  own dedicated review given the risk of touching that
  crash-recovery-critical function. Everything else already implemented
  is unaffected: live writes after a table opens are fully covered, and
  `Compact`'s own flush-before-clear (a runtime operation the adapter
  already controls the ordering of) works correctly.
- Round eleven: Codex's own sandbox execution environment became unusable
  on the coordinating machine for reasons unrelated to this spec (a
  persistent local IPC/process-spawn failure between the Codex CLI and
  its sandbox helper, confirmed via direct log inspection to affect
  every command the CLI attempted, not specifically this work order, and
  not resolved by fixing a separate, real CLI-path misconfiguration or a
  full machine restart). The owner picked Claude to implement this work
  order directly rather than continue blocking on that environment
  issue; a fresh Codex session should independently inspect the result
  once its sandbox is restored, per this crate's own established
  after-a-host-takeover review convention.
- Implementation delegated first to Codex via `codex-build` (nine rounds
  of advisory-only scoping, no code — see above); ultimately implemented
  directly by Claude after Codex's sandbox environment became unusable,
  per this crate's own established host-takeover convention. See
  `docs/design/MVCC-PRODUCTION-DESIGN.md` for the concrete shape and
  acceptance criteria.
- 2026-09-18, merged: PR #234 (the core, `Memory`/`Entity`/`Relation`),
  then seven follow-on PRs the same day, each independently verified by
  Claude (diff read, fresh `fmt`/`clippy`/`test`) before merge — the
  gaps PR #234 named as open are now closed or explicitly retained:
  - #235 — `Entity`/`Relation` real-socket integration tests (6 each,
    mirroring `Memory`'s).
  - #236 — client support (`MVCC2-FR-011`): Rust
    `SessionOptions::mvcc_isolation()`; Python `SESSION_MVCC_ISOLATION`.
  - #237/#238 — round ten's insert-log restart gap, closed without the
    generic-layer hook it predicted: `docs/design/MVCC-OPEN-HOOK-
    PROPOSAL.md` weighed (a) a `GenericMmapStore::open` hook, (b) a
    pre-open read of the pending log composed from already-`pub(crate)`
    `insert_log::{read_entries, log_path}`, bundled into an
    `open_with_mvcc(path)` constructor per domain, (c) docs only; the
    owner picked (b). Zero change to `src/generic/mmap_store.rs`.
  - #239 — `Dog`/`Order`/`Employee` wired, at the owner's explicit
    "everything" pick, **reversing this ADR's own deferral** above.
    Structurally different and simpler: none of the three implements
    `insert_record`/`replace_record`/`delete_record`/`compact`/
    `write_batch` (all default `Unsupported`), so their only mutation
    path is `UpdateField`/session `Commit` — no insert log to fold, no
    `Compact` flush; the journal checkpoint is their sole reclamation
    boundary.
  - #240 — **a real correctness bug, found only by #241's deployment
    test, not by any unit or integration test above**: `mvcc_flush_now`
    was called from `compact()`, the journaled checkpoint branches, and
    `mvcc_begin`'s one-time baseline — never from any *non-journaled*
    write path (`insert_record`/`replace_record`/`replace_record_if`/
    `delete_record`/`write_batch`'s atomic branch/`apply_transaction`/
    `apply_transaction_mvcc`, and `update_field` on the three new
    domains). Round six's "flush only at reclamation boundaries" was
    correct for *journaled* tables; an *unjournaled* table has no
    checkpoint boundary at all, so its history was persisted only by an
    explicit `Compact`. Symptom: a session `Commit`, then a restart, then
    a fresh MVCC session read the stale pre-commit baseline while a plain
    `GetById` showed the committed value. Fix, all six domains: flush
    after every MVCC-recording write, inside the same exclusive section,
    surfacing a flush I/O failure as `ErrorCode::Storage`; proportionate
    because those paths already pay a per-write fsync. 22 restart-safety
    tests added.
  - #241 — `SERVER_MVCC_ISOLATION` on `memory_server`: a pre-open
    file-existence check picks `with_mvcc` (fresh) vs. `open_with_mvcc`
    (reopen) per table; refuses to start combined with
    `SERVER_TXN_JOURNAL_PATH` (a named limitation, not a silent gap);
    three tests spawn the real compiled binary over TCP. Held unmerged
    at the owner's call until #240 landed, then its restart test was
    changed from documenting the gap to asserting the fix.
- Still open after #241, deliberately: reconstruction from a pending
  *journal* remainder (the open-hook proposal's own open question — the
  insert-log twin is closed); MVCC and the crash-atomic journal on the
  same table; and independent Codex review of the entire line — its
  Windows sandbox runner never connected its pipe at any point
  (investigated 2026-09-19: the runner launches as the sandbox user and
  logs on successfully, then times out connecting; the one coincident
  machine change is Defender's 09-17 platform update; unresolved).
- 2026-09-19: this record, `docs/design/MVCC-PRODUCTION-DESIGN.md`'s
  change history, `PROJECT-STATUS.md`, `ROADMAP.md`, `TRACEABILITY.md`,
  and `FUTURE-GROWTH.md` reconciled — all six had been left at PR #234's
  state, still listing the gaps #235–#241 closed.
