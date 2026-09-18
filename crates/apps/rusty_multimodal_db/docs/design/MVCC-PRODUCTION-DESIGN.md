# MVCC Production Design (Accepted)

- Status: **Accepted** — owner picked scope and mechanism interactively,
  2026-09-17, alongside `ADR-0072`. Authorizes real, production wiring
  of `SESSION_MVCC_ISOLATION` into `Memory`/`Entity`/`Relation`, with
  full mutation-path coverage, `(new_value, txn_id)` chain entries
  (no before-image capture — round seven's revision) folded directly
  into the existing insert log and redo journal, and a persistent
  history store flushed only at existing checkpoint/compact boundaries
  (round six's revision) (all revised mid-round — see `ADR-0072`'s own
  "Decision" and "Acceptance and implementation").
- Date: 2026-09-17
- Related: `ADR-0072` (the full decision record — read that first for
  Context/Decision-drivers/Decision; this document is the concrete
  implementation shape), `docs/design/MVCC-SPIKE-DESIGN.md` (the
  already-validated core logic this document wires up for real — do not
  re-derive it, reuse it), `ADR-0033`/
  `docs/design/SERVER-SESSION-SNAPSHOT-ISOLATION-DESIGN.md` (the
  `check_read_set` insertion point session `Commit` reuses), `ADR-0025`/
  `ADR-0026`/`ADR-0063` (the redo journal, its kind-tagged entry format,
  and its replay-to-reconstruct-state precedent — extended, not
  replaced, by this round), `ADR-0051` (the insert log's own kind-tagged
  entry format and its precedent for an additive version bump), `ADR-0052`
  (`Compact`, extended at the adapter level only).

## Purpose and scope

Wire the phase-1 spike's already-proven MVCC mechanism (versioned
writes, snapshot-consistent reads, write-write conflict detection,
minimum-snapshot-bounded GC) into the real server, for real client
connections, against `Memory`/`Entity`/`Relation` — the domains the real
consumer (`rusty_remind_me`) uses — covering **every** mutation path on
a table once it has ever used MVCC (not only session commits), with
`(new_value, txn_id)` chain entries — no before-image capture, see
`MVCC2-FR-002` — folded into the **existing** insert log and redo
journal entries (not a separately-durable per-write file — see
`ADR-0072`'s round-four revision for why), backed by a persistent
history store flushed only at existing checkpoint/compact boundaries
(`ADR-0072`'s round-six revision).

**In scope:**

- A table becomes MVCC-active the instant any connection's `Begin` first
  sets `SESSION_MVCC_ISOLATION` on it, for the table's remaining
  lifetime (durable once the first MVCC-carrying entry exists in the
  persistent history store — see `MVCC2-FR-001`).
- **New, additive entry kinds in two existing durable logs** —
  `src/generic/insert_log.rs` (for ordinary, non-journaled `Insert`/
  `Replace`/`Delete`) and `src/server/journal.rs` (for session `Commit`,
  atomic-mode `WriteBatch`, and journaled ordinary writes) — each
  pairing the existing entry payload with `(new_value, txn_id)` for each
  key the operation writes, with the `txn_id` assigned at whatever point
  that log already serializes its own append against concurrent writers
  (`journal.rs`'s own `seq`, for journaled paths — `MVCC2-FR-002`). No
  read of prior state, no new file, no `JOURNAL_FORMAT_VERSION` bump, no
  `insert_log` version bump — new kind values only, produced only by an
  MVCC-active table.
- **Every mutation to an MVCC-active table records `(key, new_value,
  txn_id)` for everything it writes** — an ordinary `Insert`/`Replace`/
  `Delete`/`Link`/`WriteBatch` exactly as much as a session `Commit` —
  as part of the exact durable write that operation already makes,
  never a second one. A table that has never gone MVCC-active is
  completely unaffected.
- A real, persistent per-table MVCC history store, reconstructed (in
  memory) on table open by combining it with whatever the insert log/
  journal still hold since the last checkpoint/compact — the in-memory
  structures `MVCC-SPIKE-DESIGN.md`'s `MvccTable`-equivalent logic
  already needs (current last-write-txn per key, the version chain,
  `last_committed_txn`). This store is flushed *only* at the insert
  log's/journal's own existing reclamation boundaries (`Compact`,
  journal checkpoint), never per write — per-write durability comes from
  the log entries themselves (previous bullet).
- A fourth `BeginWith` bit, `SESSION_MVCC_ISOLATION = 8`,
  `PROTOCOL_VERSION` 26 → 27.
- Wiring into `src/server/{memory,entity,relation}.rs`: `Begin` marks
  the table active in memory and opens a snapshot; `GetById` (and the
  existing read-your-writes/snapshot-isolation overlay points) answers
  as of that snapshot when this bit is set; session `Commit`'s
  write-write conflict check runs inside the existing `check_read_set`
  critical section (unchanged — a pure in-memory decision); every
  mutation path records its `(new_value, txn_id)` entry as part of its
  own existing durable append (no prior-value read needed) and folds it
  into the version-index after that append is confirmed.
- `Compact`'s adapter-level extension for these three adapters only:
  flushing pending MVCC history to the persistent store before clearing
  the insert log (a durability requirement), then reclaiming history no
  open snapshot needs from that store while retaining its baseline (GC
  proper) — see `MVCC2-FR-010`. The journal's own checkpoint path gets
  the equivalent flush-before-truncate step.
- Client support: `SchemaDrivenClient`/`SessionOptions` (Rust),
  `clients/python/` (protocol + client), matching every prior
  `BeginWith` bit's own client-side precedent.
- Tests: unit tests per adapter (mirroring `dog.rs`'s existing
  `check_read_set` test pattern) plus real two-connection integration
  tests (mirroring `ADR-0033`'s own flagship pattern: one connection's
  snapshot survives a concurrent commit *and* a concurrent ordinary
  write; a conflicting session commit is rejected whole; GC reclaims
  only what it should and never loses the baseline).

**Out of scope (see `ADR-0072`'s own "Explicitly out of scope"):**

- `Dog`/`Order`/`Employee` — not wired this round.
- Multi-table atomicity; *staging* `Insert`/`Link`/`Replace`/`Delete`
  inside a session's own batch (still refused with `SessionOpen` — a
  session may only stage `UpdateField`, unchanged); phantom-read
  protection for `FilterEq`/`ScanField` — all unchanged from `ADR-0071`.
  (Ordinary, non-session `Insert`/`Link`/`Replace`/`Delete` calls are a
  *different* code path from session staging and are now MVCC-stamped
  per this design — not a contradiction, see `ADR-0072`.)
- Any change to `SlotFile`'s format, `SCHEMA_VERSION`, or `SchemaTag`.
- A generic `Compact` trait change, or any change to the ~9 other
  implementors of that trait outside `memory.rs`/`entity.rs`/
  `relation.rs`.

## Requirements

- `MVCC2-FR-001` — **MVCC activation; a persistent history store exists,
  but is flushed only at checkpoint/compact boundaries, never per
  write.** A table becomes MVCC-active at the instant of its first-ever
  `Begin` with `SESSION_MVCC_ISOLATION` set — this activates in-memory
  bookkeeping immediately (necessary so an ordinary write from a
  *different* connection, landing after this `Begin` but before any
  commit, is still correctly stamped and later visible to conflict/read
  checks). Per-write durability of its `(new_value, txn_id)` entry
  comes from the insert log's/journal's own new entry kinds
  (`MVCC2-FR-002`/`008`) — no separate per-write append. But those logs
  are **not permanent**:
  `Compact` already clears the insert log once its entries are reflected
  in a freshly rewritten primary blob, and journal checkpoints already
  truncate the journal once primary state is confirmed durable — so a
  genuinely persistent, per-table MVCC history store is still required,
  to receive whatever MVCC payloads a log held right before it is
  reclaimed (see `MVCC2-FR-009`/`010` for exactly where). This store, not
  log-entry presence, is the durable "has this table ever gone
  MVCC-active" signal that survives across any number of
  checkpoints/compactions. A `Begin` followed by no mutation from
  anyone, ever, creates no entry anywhere and the table stays
  indistinguishable from one that never used MVCC. A table that has
  never gone MVCC-active is byte-for-byte unchanged: no new entry kind
  ever produced, no store ever created, no stamping, no cost.
  **Round eight's own finding: activation needs a one-time baseline
  scan, or pre-existing records become invisible.** A record that
  already existed before this table's first `Begin` has no chain entry
  at all — under `MVCC2-FR-006`'s "no qualifying entry = absence" rule,
  it would incorrectly read as "not found," and even after its first
  post-activation write, a snapshot from before that write still has no
  way to recover its true pre-activation value (the entry only records
  the *new* value, per `MVCC2-FR-002`). **Fix**: at the table's
  first-ever `Begin`, before that connection's session is usable for
  any MVCC read or conflict check, take the table's existing exclusive
  lock once, scan every currently-live record, and durably write one
  baseline entry per record — `(key, current_value, txn_id = 0)` — into
  the persistent history store (`MVCC2-FR-010`). Reserve `txn_id = 0`
  exclusively for this purpose; every real, assigned txn id
  (`MVCC2-FR-002`) starts at `1`, so a baseline entry is always the
  oldest possible entry in any chain and is always visible to every
  snapshot that has no newer entry to prefer. This is a one-time,
  proportional-to-table-size cost, structurally the same "scan
  everything under the write lock" pattern `Compact` already uses — not
  a new kind of operation this crate has never done. A table with zero
  live records at activation writes zero baseline entries (nothing to
  seed).
- `MVCC2-FR-002` — **Round nine revision (superseding the append-time
  assignment below): `txn_id` is assigned at *apply* time — inside the
  same `with_exclusive` critical section that already runs
  `apply_batch`/`apply_prepared` and folds the write into the version
  index — for every write path uniformly, not at journal-append time.**
  Round eight's shared-`AtomicU64`-at-append-time fix still left a race:
  a journaled write's id is assigned inside `journal.rs`'s own mutex,
  before its fsync and turn-gate wait, leaving a real gap before it
  actually applies under the store's separate `RwLock`; a concurrent
  non-journaled write assigns its id at apply time, under that same
  `RwLock`, with no such gap, and can be assigned a *higher* id while
  applying *first*. A snapshot opened in the gap would see
  `last_committed_txn` reflect the higher-numbered write already
  folded in, and when the lower-numbered journaled write later applies,
  its id would make it look like it happened before that snapshot even
  though it took effect afterward — a real serializability violation.
  **Fix**: assign `txn_id` uniformly at the moment a write actually
  takes effect (inside `with_exclusive`), for both paths — this makes
  `txn_id` a purely reconstructed quantity, exactly like
  `last_committed_txn` already is (`MVCC2-FR-003`), re-derived
  deterministically by folding entries in log order (replay) or by
  `fetch_add` in `with_exclusive` acquisition order (live), rather than
  a literal value embedded in the durable insert-log/journal bytes.
  **This removes the need for new insert-log/journal entry kinds
  entirely** — the durable format is unchanged from today's; only the
  in-memory version index and the persistent MVCC history store are
  new. Replay needs only the table's own MVCC-active flag (reconstructed
  at open) to know whether to fold each entry into the version index,
  in log order, assigning ids the same way a live apply does. See the
  "Proposed shape" section and `MVCC2-FR-008`/`009`/`010` below, revised
  accordingly; the paragraph immediately following (the original,
  append-time design) is retained for its still-valid framing of *why*
  no before-image is captured, superseded only on *where* the id is
  assigned.
  **No before-image capture at all — each entry
  records `(key, new_value, txn_id)`, and a txn id is only ever assigned
  at a point that is already correctly serialized. This replaces the
  original spike's `(old_value, superseded_by_txn)` shape; the
  spike's *guarantees* (every one of its nine tests' assertions) must
  still hold under this equivalent representation — see "Proposed
  shape."** Round seven's own finding, verified by direct code reading,
  is why: capturing "the value this write superseded" requires reading
  the current value at some point, and the only point available before
  a journaled write's durable append is `validate_batch`'s existing
  pre-append read — which runs *before* the exclusive section that
  actually serializes writers against each other. Two concurrent
  journaled writers to the same key can both read the same stale
  current value there, so whichever one's entry gets applied *second*
  durably records the wrong before-image — a real bug, not
  theoretical. **The fix removes the need for that read entirely**: a
  write's own new value is already known at construction time, with no
  extra read of anything. The only thing that still needs correct,
  race-free serialization is the *txn id itself*.
  **Round eight's own correction: `journal.rs`'s `seq` (`state.appended`)
  is not usable directly — it resets to `0` on every process restart
  (`journal.rs`'s own `open` re-initializes `appended = 0`), and it is
  scoped to the journal only, while some ordinary writes bypass the
  journal entirely and reach `mmap_store.rs`/`insert_log.rs` through a
  separate, uncoordinated path (confirmed: `memory.rs`'s ordinary
  replacement handling calls into the primary store directly, not
  through `journal.rs`). Reusing `seq` as-is would duplicate ids across
  restarts and collide between the two independent paths.** Fix: a
  single per-table, in-memory `AtomicU64` (not `journal.rs`'s own
  internal counter) is the txn-id source for *both* paths. Seeded once,
  at table open, from the reconstruction `MVCC2-FR-003` already
  performs — the highest txn id found across the persistent store and
  any un-reclaimed log entries, plus one (real ids start at `1`; `0` is
  reserved for `MVCC2-FR-001`'s baseline entries) — never reset to zero
  mid-lifetime. Every MVCC-relevant write, whichever path it takes,
  calls `fetch_add(1, Ordering::SeqCst)` on this *same* counter, inside
  whichever lock that path already holds when it constructs its durable
  entry (`journal.rs`'s own mutex inside `commit_entry`, at the very
  start, before anything else, for journaled writes; the equivalent
  already-serialized point for the `mmap_store.rs`/`insert_log.rs`
  bypass path — implementation confirms directly whether an existing
  lock already serializes that path's append, or adds one if it
  doesn't, per round four/six's research finding no group-commit-style
  pipelining split there). A plain atomic increment needs no shared
  lock between the two call sites — it is safe and well-ordered from
  either, which is exactly what "one counter, two independent lock
  domains" requires. No new lock is added *beyond* whatever each path
  already needs to serialize its own append; no serialization change to
  the group-commit pipeline `ADR-0026` measured and tuned. A session
  `Commit`'s whole staged batch and an atomic-mode `WriteBatch`'s whole
  batch each construct one journal entry, so each does exactly one
  `fetch_add`, covering every key it touches. **A per-op-mode
  `WriteBatch` already calls `commit_entry` (or its per-op equivalent)
  once per operation** (confirmed: each operation in that mode already
  acquires and releases the store's lock independently, `ADR-0060`'s
  own picked option (a)) — this naturally gives each operation its own
  `fetch_add`/txn id with zero special-casing, exactly matching the
  granularity that mode already treats as atomic; do not widen its
  lock scope to force a shared id, and nothing needs to. **The one
  carve-out**: an empty mutation (a session `Commit` with zero staged
  writes) remains a pure no-op — no entry, no `fetch_add` — exactly as
  `src/generic_spike/mvcc_spike.rs`'s own already-proven
  `empty_commit_and_counter_exhaustion_do_not_mutate_state` test
  requires (confirm this is already how the existing non-MVCC code
  behaves, since journal entries are already only appended for real
  batches, so no MVCC-specific check is needed to skip an empty one).
  `last_committed_txn` is the shared counter's own current value,
  reconstructed at open from the persistent store and both logs and
  never reset mid-lifetime — never separately computed as "+1" ahead of
  time by a caller outside the serialization point that assigns it.
- `MVCC2-FR-003` — **Recovery combines the persistent store with
  whatever the insert log/journal still hold.** On table open: (a) load
  the persistent MVCC history store (everything already flushed at a
  prior checkpoint/compact — `MVCC2-FR-009`/`010`); (b) fold the insert
  log (`merge_log`, `src/generic/mmap_store.rs`'s existing,
  already-idempotent-by-construction fold on open) and replay the
  journal (each adapter's `with_journal` open path, already re-applies
  durable entries), both extended to also feed the version-index from
  any MVCC-kind entries they still contain — since these logs are only
  cleared/truncated *after* a successful flush to the persistent store
  (`MVCC2-FR-009`/`010`), what remains here is exactly "since the last
  checkpoint/compact," and may safely overlap with what the persistent
  store already has (idempotent — a repeated fold/replay of an
  already-recorded txn id is a no-op, checked against what the
  version-index already holds, mirroring how `merge_log`'s overwrite-by-id
  is already naturally idempotent for primary state). Combined, these
  reconstruct, per key, the full chain of `(new_value, txn_id)` entries
  ever recorded — from which `last_committed_txn` (the highest txn id
  observed across the store and both logs, `0` if none), the current
  value (the entry with the highest txn id), and any snapshot's answer
  (the entry with the highest txn id at or below that snapshot) are all
  directly derivable — no separately-tracked "last-write txn" or
  "creation stamp" field needed; both are just properties of the chain.
  No persistent store and no MVCC-kind entries anywhere means an empty
  reconstruction
  (`last_committed_txn = 0`), not an error. A malformed/truncated
  trailing entry in either log is handled exactly as that log's own
  existing recovery already handles one today — no new failure mode
  introduced for MVCC specifically.
- `MVCC2-FR-004` — **`SESSION_MVCC_ISOLATION = 8`, `PROTOCOL_VERSION =
  27`.** Composes with the existing three `BeginWith` bits
  independently. Below version 27, the bit is `Malformed`, matching
  every prior bit's precedent (`ADR-0022`). The version table and golden
  wire vectors gain row 27.
- `MVCC2-FR-005` — **`Begin` opens a snapshot.** When this bit is set,
  `Begin` records `snapshot_txn` = the table's current
  `last_committed_txn` (from the reconstructed/live state — the *last
  committed* id, never a "next-unused" one: using next-unused here,
  combined with the inclusive `last_write_txn <= snapshot_txn` read
  comparison `MVCC-SPIKE-DESIGN.md` already established, would make the
  very next commit anyone else makes after `Begin` visible to this
  snapshot, which is wrong) on the connection, and registers it into a
  small per-table in-memory open-snapshot set (server-process lifetime
  only, never persisted — the same shape `ADR-0071` already decided).
  Deregistered at `Commit`, `Rollback`, or disconnect (including an
  ungraceful disconnect — reuse whatever cleanup hook the existing
  session teardown already has for this).
- `MVCC2-FR-006` — **Snapshot-consistent reads, adapted to the
  `(new_value, txn_id)` chain (`MVCC2-FR-002`).** While such a session
  is open, `GetById` (and any other read path this bit should cover —
  implementation confirms against the existing `SESSION_SNAPSHOT_ISOLATION`
  and `SESSION_READ_YOUR_WRITES` overlay points, since a session may
  combine any of the four bits) answers with the entry carrying the
  highest `txn_id` at or below the snapshot's own `snapshot_txn`, for
  that key's chain — the fast path (current primary value) applies
  whenever that entry is also the chain's newest overall; otherwise walk
  back through older entries. No entry at or below `snapshot_txn` means
  absence — including a key created after the snapshot began, which
  naturally has no qualifying entry at all, with no separate
  "pre-creation" case to special-case. A reclaimed-but-needed interval
  (GC removed the entry a snapshot needed) is a typed, distinct error
  (not `NotFound`, not silent wrong data) — `MVCC-SPIKE-DESIGN.md`'s
  proven logic for this must carry over even though the entry shape
  changed. This must now hold correctly against a key created or
  overwritten by an *ordinary* write from another connection, not only
  against another session's commit — the direct benefit of
  `MVCC2-FR-008`'s full coverage. Read-your-writes composes exactly as
  it does with the other three bits today.
- `MVCC2-FR-007` — **Write-write conflict check at session `Commit`,
  same critical section as `check_read_set`.** For every key the
  transaction's own staged `UpdateField` batch targets, compare its
  current last-write txn (kept live as any mutation — session or
  ordinary — lands) to the session's `snapshot_txn`; any advance refuses
  the whole commit, applying nothing, reported as
  `Response::TransactionFailed { index: 0, code: ErrorCode::Conflict }`
  (`ADR-0033`'s existing code — no new `ErrorCode` variant). This
  comparison happens inside the exact same `with_exclusive`/
  `with_journal` section `check_read_set` already occupies — no new
  lock, no new window for another commit to land in between. (This
  transaction's own `(new_value, txn_id)` entry was already durably
  appended earlier, at journal-append time, per `MVCC2-FR-008`/`002` —
  this check is purely the in-memory conflict decision against the
  version-index's already-recorded chain, not a durability step.) This
  check only covers the transaction's *own* writes vs. its
  snapshot — it is a write-write check, not a re-validation of every key
  the session merely read (that remains `ADR-0033`'s own, separate bit).
- `MVCC2-FR-008` — **Full mutation-path coverage — the central
  correctness requirement of this round — using the no-before-image
  `(new_value, txn_id)` model (`MVCC2-FR-002`, round nine: `txn_id`
  assigned at apply time inside `with_exclusive`, in no new durable
  entry kind — see that requirement's round-nine note).** Once a table is
  MVCC-active, every ordinary (non-session) `Insert`, `Replace`,
  `Delete`, `Link`, and `WriteBatch` must record an entry — at the
  granularity `MVCC2-FR-002` already fixes (atomic-mode `WriteBatch` and
  a single `Insert`/`Replace`/`Delete`/`Link` as one unit; **each
  individual operation inside a per-op-mode `WriteBatch` separately**,
  since that mode does not commit its batch as one unit today and this
  round must not make it start doing so). Concretely: at the point that
  unit's own durable append already happens (`journal.rs`'s `seq`
  assignment for a journaled write; the equivalent already-serialized
  point for a non-journaled `mmap_store.rs` write — confirmed, not
  assumed, per `MVCC2-FR-002`), record `(key, new_value, txn_id)` for
  every key the operation writes, as a new kind value in that same log
  entry — **no read of the key's current/old value is needed or
  performed for this purpose**, closing the exact race round seven's
  research found in an earlier before-image-based draft of this
  requirement. After that append is confirmed, the version-index simply
  gains this new chain entry — no separate "stamp `last_write_txn`" or
  "stamp `created_at`" step, since both are just properties of the
  chain (`MVCC2-FR-003`). Ordinary writes have no session and thus
  nothing to conflict-check against — they append and index
  unconditionally, exactly as a session's own validated writes do.
  **Without this requirement, `MVCC2-FR-006`'s snapshot-consistent-read
  guarantee is false the moment any connection issues an ordinary write
  to a key an open MVCC snapshot has read or will read** — this is not
  an optional hardening pass, it is required for this feature to be
  correct at all. A table that has never gone MVCC-active
  (`MVCC2-FR-001`) is completely unaffected by this requirement.
- `MVCC2-FR-009` — **Two accepted durability properties, deliberately
  left as-is — read both carefully, they are different.**
  1. *Rejected session commits* (`ADR-0033`'s own pre-existing property,
     confirmed unaffected by this round): read `src/server/journal.rs`'s
     `commit_entry` and each adapter's own `with_journal` open/replay
     path — the redo entry for a *session* commit is appended and made
     durable *before* the adapter's validating closure
     (`check_read_set`, and this round's write-write check) runs, and
     replay re-applies a durable `JournaledBatch::Transaction` via
     `apply_batch` **directly, without re-running any validation**. A
     live session commit rejected by the conflict check thus leaves a
     durable, unapplied redo entry that a crash before the next commit's
     truncate could in principle replay as if it had succeeded — the
     exact same narrow window `ADR-0033`'s own `check_read_set` already
     has. **Do not build new recovery/replay machinery to close this**
     — explicitly out of scope (see `ADR-0072`'s "Negative /
     tradeoffs"); it would need its own ADR covering `ADR-0033` too.
  2. *Primary/MVCC per-write coordination* (this round's own concern,
     closed by construction for the per-write case, not accepted as a
     risk): because the `(new_value, txn_id)` entry is embedded in the
     *same* durable entry as the primary write (`MVCC2-FR-008`), there
     is no crash window where one becomes durable without the other for
     a single write — they are the same byte range, the same
     `sync_data`/`append_unsynced` call, and (per `MVCC2-FR-002`'s
     round-seven revision) no read of prior state is needed to construct
     it in the first place, closing the concurrent-writer race that
     specific reads would have reopened. Do not reintroduce a separate
     append for the MVCC payload "for simplicity" — that is exactly the
     design this round's own research rejected as unsafe. **This is
     narrower than "MVCC durability is fully solved"** — see
     `MVCC2-FR-009`'s own continuation below for the checkpoint/compact
     case, which is a *different* coordination point with its own fix.
  3. *Checkpoint/compact coordination* (a second, later finding — the
     insert log and journal are not permanent): `Compact`
     (`mmap_store.rs::compact()`) already deletes the insert log via
     `insert_log::clear` once its entries are reflected in a freshly
     rewritten primary blob; journal checkpoints (both the normal
     group-commit path and `with_journal`'s open-path replay) already
     truncate the journal once primary state is confirmed durable via
     `checkpoint_flush()`. Whatever MVCC payloads were living only in
     that log's own entries are gone once either reclamation completes
     — nothing else durably remembers them unless a persistent MVCC
     history store received them *first*. See `MVCC2-FR-010` for exactly
     where and how.
- `MVCC2-FR-010` — **A real, persistent MVCC history store, flushed only
  at the *three* existing reclamation boundaries — journal checkpoint,
  `Compact`, and (round eight's own addition) `mmap_store.rs::open()`
  itself — never per write.** Round nine note: since no new durable
  entry kind exists (`MVCC2-FR-002`), "flush pending MVCC payloads
  before reclaiming the source log" means folding every not-yet-flushed
  entry into the version index (assigning `txn_id`s in log order,
  exactly as replay-at-open already does) and writing the resulting
  chain state to the persistent store — the mechanism is a version of
  the same fold-then-flush this requirement already describes, just
  without a separate MVCC-tagged payload to extract from each entry.
  1. **Journal checkpoint.** Each adapter's `with_journal` commit path
     already computes, inside the same exclusive section `apply_batch`
     runs in: `flushed = checkpoint_due && inner.checkpoint_flush().is_ok()`
     (confirmed in `memory.rs`, matching shape in `entity.rs`/`relation.rs`);
     `journal.rs`'s `commit_entry` truncates the journal *only if*
     `flushed` is `true`. Extend that same expression to also durably
     flush any pending MVCC payloads (from the journal entries just
     applied) into the persistent store, and fold *that* step's success
     into the same `flushed` boolean — e.g.
     `flushed = checkpoint_due && inner.checkpoint_flush().is_ok() && flush_mvcc_history(...).is_ok()`
     shaped, exact form is implementation's call. If the MVCC flush
     fails, `flushed` is `false`, the journal is *not* truncated, and
     those entries remain for the next attempt — the identical
     resilience the existing gate already provides for primary
     durability, extended to MVCC, not a new mechanism. Apply the
     identical extension to `with_journal`'s open-path replay (replay
     batches, flush MVCC history, `checkpoint_flush()`, *then*
     `journal.truncate()` — in that order).
  2. **`Compact`.** `mmap_store.rs::compact()`'s existing first step
     already reads every insert-log entry (today, only to count them)
     before writing the fresh blob, rewriting slots, and calling
     `insert_log::clear` — extend that same read pass to also extract
     and durably flush each entry's MVCC payload into the persistent
     store, *before* any of the destructive steps that follow. This
     needs no new lock: `compact()` already runs under the store's own
     write lock (confirmed via `memory.rs`'s own doc comment).
  3. **`mmap_store.rs::open()` itself — a second, more frequent
     reclamation point round eight's own research found**: `open()`
     already calls `merge_log` to fold the insert log into `records`,
     then already clears the insert log afterward (not only `Compact`
     does this — every table reopen, including a normal server
     restart, does too). Apply the identical extension used for
     `Compact`: while folding the insert log's entries during `open()`,
     also extract and durably flush their MVCC payloads into the
     persistent store *before* the log gets cleared. This is a straight
     port of item 2's own fix to a second call site with the same
     shape, not a new mechanism.
  4. **Idempotency, reusing an existing invariant, not inventing one.**
     A crash between "MVCC flush succeeded" and "truncate/clear
     completed" leaves the source log still present, so it gets
     folded/replayed again on the next open (`MVCC2-FR-003`) —
     re-flushing already-recorded txn ids into the persistent store must
     be a safe no-op (check-before-add). This is exactly the same
     idempotency `MVCC2-FR-002`/`003`'s per-write folding already
     requires, and exactly the reasoning `mmap_store.rs`'s own existing
     doc comment already gives for why a crash before `insert_log::clear`
     completes is safe for primary-state recovery ("the next open's
     `merge_log` folds the log again, and that refold is already
     required to be idempotent") — applied to one more piece of state,
     not a new property to prove.
  5. **GC proper, on the persistent store, adapter-level only** — not a
     new trait method, not a new parameter on the generic `Compact`
     trait, and not a change to the insert log's or journal's own
     existing truncation/checkpoint behavior beyond the flush step
     above: given the table's current minimum open-snapshot txn id (or
     "none open"), for each key's chain, drop every entry whose `txn_id`
     is older than the newest entry at or below that boundary — the
     standard MVCC GC rule (`MVCC-SPIKE-DESIGN.md`'s own already-proven
     `gc()` logic, adapted to the `(new_value, txn_id)` shape) — but the
     rewrite must retain `last_committed_txn` and, for every key
     currently live in the primary store, its newest surviving chain
     entry. **This baseline is a real correctness requirement again, not
     defense in depth** — once `Compact` has cleared the insert log, that
     baseline is no longer reconstructable by re-folding it. Written via
     write-to-temp-then-atomic-rename (`STORAGE-014`'s precedent). This
     is the *only* GC path — no background thread. The nine other
     `Compact` implementors (`BaseStore`, `Indexed`, `Scanned`,
     `Symmetric`, `MultiSymmetric`, `NameIndex`, `Reversed`, `Ordered`,
     `MmapScanned`) are untouched — confirm this by grep, not
     assumption, before claiming it in the PR.
- `MVCC2-FR-011` — **Client support.** `SessionOptions`/
  `SchemaDrivenClient` (Rust) and `clients/python/rusty_multimodal_db/`
  (protocol + client) gain the new bit, matching
  `SESSION_SNAPSHOT_ISOLATION`'s own client-side shape exactly (same
  method-naming pattern, same version gate).
- `MVCC2-FR-012` — **`Memory`/`Entity`/`Relation` only.** `Dog`/`Order`/
  `Employee`'s `apply_transaction`/session/ordinary-write code paths are
  unchanged — confirm no accidental shared-code edit leaks the new
  bit's behavior or the new stamping step onto them (e.g. if any touched
  function is shared code across all six adapters, the MVCC step must be
  adapter-specific, not injected into shared code all six call, unless
  that shared code is scoped so `Dog`/`Order`/`Employee` are structurally
  never MVCC-active and the new step is a no-op for them by construction).

## Proposed shape

The exact new entry-kind byte layouts in the insert log and the
journal, and the persistent MVCC history store's own on-disk format and
naming, are implementation's call — this document fixes the
*guarantees* (`MVCC2-FR-001..012` above), not the byte-for-byte layout.
Structural points that are fixed:

1. **Reuse `MVCC-SPIKE-DESIGN.md`'s already-proven read/conflict/GC
   logic, adapted from `(old_value, superseded_by_txn)` to
   `(new_value, txn_id)` (`MVCC2-FR-002`, round seven's revision)** — the
   `MvccTable`/`Snapshot`-equivalent shape from
   `src/generic_spike/mvcc_spike.rs` is the reference for *guarantees*,
   not byte-for-byte reusable as written, since its entry shape assumed
   a before-image that this round's research found unsafe to capture
   under real concurrent journaled writers. Every one of the spike's nine
   tests' *assertions* (a snapshot retains its pre-write view; the
   chain-walk picks the correct interval, not just "the previous one";
   a conflicting commit applies nothing; GC reclaims only what it
   should and never resurrects reclaimed history) must still hold, ported
   to the new shape — do not treat this as license to weaken any of
   them. This round's version does **not** itself decide durability or
   read any prior value — by the time it runs, the `(new_value, txn_id)`
   entry is already durable as part of the insert log's or journal's own
   entry (`MVCC2-FR-008`), with `txn_id` already correctly assigned by
   the caller's own already-serialized append point; this logic only
   needs to (a) do the in-memory write-write conflict comparison against
   the chain's newest entry and (b) append the new chain entry to the
   in-memory version-index once the caller confirms the underlying
   durable append succeeded. Design its API so it is callable from
   *every* mutation path (`MVCC2-FR-008`), not only a session-commit-
   shaped call site (e.g. a `record_write(index, key, txn_id, new_value)`-shaped
   function every write path calls after its own durable append, rather
   than something coupled to `TransactionOp`/session-batch types).
2. **Each write path assigns its txn id at the exact point that path
   already serializes appends against concurrent writers, and needs no
   read of any prior value.** For a journaled path (session `Commit`,
   atomic-mode `WriteBatch`, journaled ordinary writes), that point is
   `journal.rs`'s `commit_entry`'s own `seq` (`state.appended`)
   assignment, under its own mutex, before `append_unsynced`/`sync` —
   reuse `seq` directly as the txn id; do not compute a separate
   `last_committed_txn + 1` anywhere else, since anything computed
   outside that exact serialization point can race (round seven's
   finding). For a non-journaled `Insert`/`Replace`/`Delete` at the
   `mmap_store.rs`/`insert_log.rs` layer, confirm directly whether an
   equivalent already-serialized counter exists there; if not, a small
   per-table `AtomicU64` incremented under whatever lock already
   serializes that log's own append is the minimal addition — verify
   there is no cross-writer race there before assuming today's
   structure is already safe. **For a per-op-mode `WriteBatch`
   specifically**, each individual operation already acquires and
   releases its own lock and calls its own append independently
   (confirmed directly in `memory.rs:763`/`entity.rs:653`/
   `relation.rs:408`) — each such append gets its own `seq`/txn id
   naturally, zero special-casing needed; do not widen per-op mode's
   lock scope to force a shared id, which would change its
   already-shipped concurrent-access behavior for every caller. Read
   each of `memory.rs`/`entity.rs`/`relation.rs`'s handling of each
   operation kind, and `src/generic/insert_log.rs`'s/`mmap_store.rs`'s
   own `insert`/`replace`/`delete`, before writing a single line; do not
   assume they are byte-identical to each other or to `dog.rs`'s shape.
3. **Superseded, round nine: no new entry kinds in either log.** Since
   `txn_id` is now assigned at apply time, not append time
   (`MVCC2-FR-002`'s round-nine note), the durable insert-log/journal
   bytes need no `txn_id` field at all — a table's existing entries
   (kind `0`/`1`, unchanged) are folded into the version index on
   replay, in log order, assigning ids the same way a live apply does.
   `insert_log`'s version stays 2; `JOURNAL_FORMAT_VERSION` stays 2. A
   table that never goes MVCC-active is unaffected exactly as much as
   the original (now-superseded) new-kind design intended, just with
   zero format footprint instead of an unused-but-defined kind value.
4. **Activation state** (`MVCC2-FR-001`'s "has this table ever gone
   MVCC-active") needs to be checked cheaply on every write, including
   by a table that is *not* MVCC-active — implementation's call how (an
   `AtomicBool`/`Option`, set once an MVCC-kind entry is folded/replayed
   at open or produced live, checked before any chain-entry-recording
   logic runs, is the obvious shape), but it must not meaningfully slow
   down a table that never uses MVCC.

## Acceptance criteria

1. A table that has never gone MVCC-active has zero new entry kinds in
   its insert log or journal and zero behavior change — every existing
   test in `tests/server_*_integration.rs` for `Memory`/`Entity`/
   `Relation` passes completely unmodified, and the insert log/journal's
   own existing golden-format tests (if any) still pass unmodified.
2. `SESSION_MVCC_ISOLATION`/`PROTOCOL_VERSION = 27` exist exactly as
   specified; the version table and golden vectors updated; the bit is
   `Malformed` below version 27; it composes with all three existing
   bits in every combination without changing their own behavior.
3. **Real two-connection test, each of `Memory`/`Entity`/`Relation`**:
   connection A opens an MVCC session and reads a field; connection B
   commits a *session* change to that same field; A's own subsequent
   read (same session, same snapshot) still sees the pre-B value; A's
   commit touching that field is refused with `Conflict`, applying
   nothing, including any other uncontested keys in the same batch.
4. **The same scenario with B using an *ordinary* (non-session)
   `Replace`/`UpdateField`-via-`WriteBatch` instead of a session commit**
   — proving `MVCC2-FR-008`: A's snapshot still sees the pre-B value,
   and if A's own session later stages a write to that same key, its
   commit is refused with `Conflict` exactly as it would be against a
   conflicting session commit.
5. **A key created by an ordinary (non-session) `Insert` after A's
   snapshot began** is invisible to A's snapshot (still "not found"),
   even though the insert did not happen inside any session —
   proving `MVCC2-FR-008` closes the exact gap Codex's implementation-time
   research found in the narrower, session-only draft of this design.
6. The same sequences (3–5) with no intervening conflicting write: A's
   commit or read succeeds normally — no false conflicts, no false
   invisibility.
7. **Reopen immediately after a write, before any checkpoint or
   `Compact` has ever run**: closing the server and reopening the table
   correctly reconstructs that write's MVCC history *purely from the
   insert log's/journal's own MVCC-kind entries* (`MVCC2-FR-003`), for
   history created by *both* session commits and ordinary writes, with
   no persistent-store flush having happened yet — proving the per-write
   fold-in (`MVCC2-FR-008`) genuinely removes the per-write two-phase gap
   on its own, independent of the checkpoint/compact machinery.
8. **Reopen after at least one journal checkpoint or `Compact` has run**:
   a snapshot opened after such a reopen still sees the full, correct
   history from *before* that checkpoint/compact, proving
   `MVCC2-FR-009`/`010`'s flush-before-reclaim step actually ran and
   the persistent store actually has what the now-cleared/truncated log
   no longer does. This is the central proof this round exists to
   deliver: reconstruction must not silently go wrong once the source
   logs are gone.
9. **A crash simulated between the persistent-store flush and the
   corresponding `truncate`/`clear`** (or the closest equivalent this
   test suite's durability tests already use — e.g. killing the process
   or skipping the truncate/clear call in a test harness after the flush
   step runs): the next open still reconstructs correctly and does not
   duplicate or corrupt history — proving `MVCC2-FR-010`'s idempotent
   double-flush safety for real, not just by inspection.
10. `Compact` on a table with open MVCC snapshots reclaims only
    historical entries no open snapshot needs, leaving still-needed
    ones intact — and after a full reclaim (no snapshots open), a fresh
    reopen still reports the correct `last_committed_txn` and every
    live key's correct creation/last-write stamps from the retained
    baseline, proving `MVCC2-FR-010`'s retention requirement (now a
    real correctness requirement, not defense in depth, once the insert
    log has been cleared).
11. `Dog`/`Order`/`Employee`'s full existing test suites pass completely
    unmodified — this round changed nothing there.
12. `SchemaDrivenClient` and the Python client both support the new
    bit, exercised end-to-end in at least one real test per client.
13. **Activation baseline (`MVCC2-FR-001`, round eight)**: insert a
    record on a table *before* it ever goes MVCC-active; then open an
    MVCC session and read that record — it must be visible with its
    correct value, not "not found." Then replace that same record;
    a snapshot opened *before* the replacement must still see the
    original (pre-activation) value, not absence and not the new value.
14. **Counter durability across restart (`MVCC2-FR-002`, round eight)**:
    commit several MVCC-tracked writes, restart the server (a real
    process restart or this suite's existing equivalent), commit
    another MVCC write, and confirm its txn id is strictly greater than
    every id used before the restart — proving the shared counter is
    correctly reseeded from persisted state, not reset to zero.
15. **Cross-path id uniqueness (`MVCC2-FR-002`, round eight)**: a
    session-based write and an ordinary (non-journaled) write to
    different keys on the same MVCC-active table never receive the same
    txn id, and a real two-connection test using both paths concurrently
    shows correct, non-colliding ordering.
16. **`open()` also flushes before clearing (`MVCC2-FR-010`, round
    eight)**: commit an MVCC write via the non-journaled path, close and
    reopen the table (not via `Compact`), and confirm the write's
    history is still correctly reconstructible — proving the flush was
    added to `open()`'s own insert-log clear, not only `Compact`'s.
17. `cargo fmt -p rusty_multimodal_db -- --check`,
    `cargo clippy -p rusty_multimodal_db --all-features -- -D warnings`,
    and `cargo test -p rusty_multimodal_db --all-features --no-fail-fast`
    all clean, with the new tests included and every pre-existing test's
    outcome unchanged.

## Verification plan

- Unit tests per adapter (`memory.rs`/`entity.rs`/`relation.rs`):
  reconstruction-from-replay correctness (including a baseline surviving
  a full GC), the write-write conflict check in isolation, and — new for
  this round — an ordinary write's stamping/logging behavior in
  isolation, mirroring `mvcc_spike.rs`'s own nine tests' shapes, now
  against the real adapter code and covering non-session paths too.
- `tests/server_*_integration.rs` (one new test file per domain, or
  extensions to the existing ones — implementation's call): the real
  two-connection scenarios in acceptance criteria 3–10 above, over a real
  socket, matching this crate's own "flagship concurrent test" tradition
  (see `ADR-0033`'s own `snapshot_isolation_detects_a_conflicting_commit_from_another_connection`
  as the direct precedent to extend or mirror) — explicitly including at
  least one scenario per domain where the conflicting/creating write is
  an *ordinary* request, not a session commit.
- `tests/server_protocol_version.rs`: the version pin (27), the
  unknown-bit-below-27 gate, the golden vector for the new flag.
- `tests/server_python_client.rs`/`clients/python/tests/`: the new bit
  exercised from the Python client.

## Traceability

- → `ADR-0072`, extending `SERVER-001`'s next minor/FR. Registers as
  `SERVER-MVCC` (or implementation's own naming, matching this crate's
  `SERVER-<CAPS>` convention) in `docs/roadmap/ROADMAP.md`,
  `docs/PROJECT-STATUS.md`, and `docs/traceability/TRACEABILITY.md`.
  Closes `docs/FUTURE-GROWTH.md`'s "real MVCC" bullet's "not wired into
  the server" line for `Memory`/`Entity`/`Relation` specifically —
  update that document's wording precisely, not by overstatement (it
  remains true for `Dog`/`Order`/`Employee`).

## Open questions

- Whether `Dog`/`Order`/`Employee` are ever wired the same way — a
  future round if wanted, not decided here.
- The new entry kinds' exact byte format in the insert log and the
  journal, and the materialized cache's own format/naming if persisted —
  implementation's call, within this document's fixed guarantees.
- Whether the promoted, non-`research`-gated MVCC core logic module
  lives in `src/generic/` (alongside the other production storage
  machinery) or directly in `src/server/` (since only the server uses
  it today) — implementation's call; state which and why in the PR.
- The shared journal-ordering/replay-revalidation window named in
  `MVCC2-FR-009` — a real, pre-existing property, not solved here;
  revisit alongside `ADR-0033` if ever warranted.

## Change history

- 2026-09-17: Initial proposal and acceptance, same session, following
  the owner's interactive picks (sidecar-file storage revision,
  `Memory`/`Entity`/`Relation` domain scope) — `ADR-0072`.
- 2026-09-17: Revised, same session, after two rounds of Codex-found
  specification conflicts during implementation scoping (advisory
  reports, no code written for either): (1) snapshot boundary corrected
  to `last_committed_txn`; (2) sidecar creation timing unified to first
  `Commit`; (3) journal-ordering window named as an accepted, shared
  `ADR-0033` limitation rather than requiring new recovery machinery;
  (4) empty commits explicitly exempted from counter advancement; (5)
  GC required to retain a non-reclaimable baseline; (6) **scope expanded
  to full mutation-path coverage** (`MVCC2-FR-008`) after research found
  session-commit-only bookkeeping cannot deliver a correct snapshot
  guarantee against ordinary writes — the owner picked full correctness
  over a narrower, explicitly-weaker guarantee.
- 2026-09-17: Revised again, same session, round three — per-op-mode
  `WriteBatch` already commits each operation independently
  (`ADR-0060`), so "one txn id per whole batch" was wrong for that mode;
  fixed by granularity-matches-existing-atomicity rule (`MVCC2-FR-002`),
  not a widened lock.
- 2026-09-17: Revised a fourth time, same session, after the deepest
  finding — a genuinely separate MVCC sidecar file has no coordinated
  durability with the primary write, an unclosed two-phase-commit gap
  affecting ordinary writes with no session involved at all. **Owner
  picked the largest of four offered options**: fold MVCC before-images
  directly into the insert log's and the journal's own existing durable,
  kind-tagged entries (new kind values, no format-version bump, no new
  file) instead of a separately-durable sidecar file — eliminating the
  gap by construction. The "sidecar" was described at this point as a
  rebuildable materialized cache of what these two logs already durably
  record — **this framing was corrected in round six below.**
- 2026-09-17: Revised a sixth time, same session — round five found the
  round-four "rebuildable cache" framing itself incomplete (advisory,
  no code), and round six's dedicated research confirmed why: the
  insert log and the redo journal are **not permanent** — `Compact`
  already deletes the insert log once compacted, and journal checkpoints
  already truncate the journal once primary state is durably flushed.
  Once either reclamation happens, any MVCC payload that lived only in
  that log's own entries is gone. **Fix**: a real, persistent, per-table
  MVCC history store is still needed, but it is flushed *only* at these
  two existing reclamation boundaries — inside the same locks, gated by
  the same "flushed, therefore safe to truncate/clear" logic those
  points already use (`checkpoint_flush()`'s existing gate;
  `compact()`'s existing write lock) — never per write. The per-write
  fold-in from round four is unchanged and still closes the per-write
  gap on its own; this closes the separate checkpoint/compact gap
  round five surfaced. See `ADR-0072`'s "Acceptance and implementation,"
  rounds five and six, for the full account.
- 2026-09-17: Revised a seventh time, same session — round four's
  per-write fold-in still captured a *before-image* by reading the
  key's current value, and that read (`validate_batch`'s existing
  pre-append read, on the journaled path) runs *before* the exclusive
  section that actually serializes writers. Two concurrent journaled
  writers to the same key can both read the same stale value there, so
  whichever entry gets applied second durably records the wrong
  before-image — a real, verified bug, not the already-accepted
  rejected-commit window. **Fix, and a genuine simplification, not just
  a patch**: entries no longer carry a before-image at all. Each entry
  now records `(key, new_value, txn_id)` — the new value is already
  known with no extra read, and the only thing needing correct
  serialization is the txn id, which reuses `journal.rs`'s own
  already-mutex-protected, already-monotonic `seq` (`state.appended`)
  directly, with the existing turn-gate already guaranteeing
  application happens in that same order — no new lock, no new critical
  section, no cost to the group-commit pipeline `ADR-0026` tuned. A
  snapshot read now walks the chain for the newest entry at or below its
  own `snapshot_txn`; "not found" and "pre-creation absence" both fall
  out naturally as "no qualifying entry exists," with no separate
  creation-stamp field needed. `MVCC2-FR-002`, `006`, `008`, and the
  "Proposed shape" section are all revised accordingly.
- 2026-09-18: Revised an eighth time, same session — three more findings
  from continued implementation-scoping research, all bounded fixes
  within the already-chosen direction, no new tradeoff: (1) a table's
  pre-existing records had no chain entry at activation, making them
  wrongly invisible — fixed with a one-time baseline scan at first
  `Begin`, seeding `txn_id = 0` entries (`MVCC2-FR-001`); (2) round
  seven's reuse of `journal.rs`'s `seq` as the txn id was itself wrong —
  `seq` resets on every restart and is scoped to the journal only, while
  some ordinary writes bypass the journal entirely through a separate,
  uncoordinated path — fixed with a single shared, per-table
  `AtomicU64`, seeded at open from reconstructed state and incremented
  from whichever lock each path already holds (`MVCC2-FR-002`); (3)
  `mmap_store.rs::open()` clears the insert log too, not only `Compact`
  — the round-six flush-before-clear fix needed a second call site
  (`MVCC2-FR-010`).
- 2026-09-18: Revised a ninth time — found by Claude during direct
  implementation, not Codex (see the following entry): round eight's
  shared `AtomicU64`-at-append-time fix still left a cross-path race —
  a journaled write's id, assigned at journal-append time, leaves a gap
  before its actual apply that a concurrent non-journaled write (which
  assigns at apply time, no gap) can race ahead of, assigning a higher
  id while applying first, corrupting apply-order-vs-id-order for a
  snapshot opened in between. **Fix**: assign `txn_id` at apply time —
  inside `with_exclusive` — for both paths uniformly, making it a
  purely reconstructed quantity like `last_committed_txn` already is,
  rather than a value embedded in the durable bytes. This *removes* the
  need for new insert-log/journal entry kinds entirely (a
  simplification, not new machinery) — see `MVCC2-FR-002`/`008`/`009`/
  `010` and "Proposed shape" point 3, revised accordingly.
- 2026-09-18: Implementation delegated first to Codex via `codex-build`;
  Codex's sandbox execution environment became persistently unusable
  (a local IPC/process-spawn timeout affecting every command, unrelated
  to this spec — see `ADR-0072`'s "Acceptance and implementation,"
  round nine) even after a real, verified CLI-path fix and a full
  machine restart. Owner picked Claude to implement this work order
  directly; a fresh Codex session should independently inspect the
  result once its sandbox is restored, per this crate's own established
  host-takeover review convention.
