# MVCC Open-Hook Proposal (Accepted)

- Status: **Accepted, option (b)** — a design covering the one gap
  `ADR-0072`/`docs/design/MVCC-PRODUCTION-DESIGN.md` named and
  deliberately did not close: restart recovery of a table's MVCC
  history from a *pending* insert-log remainder specifically
  (`MVCC2-FR-010` item 3's own unfinished plan). The owner picked option
  (b) — a pre-open read composed from existing `pub(crate)` primitives,
  bundled into a new `open_with_mvcc` constructor per domain, no
  `src/generic/mmap_store.rs` change at all. Implementation is a
  separate, follow-up unit.
- Date: 2026-09-18
- Related: `ADR-0072` (the decision this extends), `docs/design/
  MVCC-PRODUCTION-DESIGN.md` (`MVCC2-FR-001`/`003`/`010`, the
  requirements this gap sits inside), `src/generic/mmap_store.rs`
  (`GenericMmapStore::open`, the function at the center of this gap),
  `src/generic/insert_log.rs` (`read_entries`, `LogEntry`, `log_path` —
  already `pub(crate)`, already used directly by `with_mvcc` today).

## Context

`MemoryConnectionStore::with_mvcc`/`EntityConnectionStore::with_mvcc`/
`RelationConnectionStore::with_mvcc` (merged in PR #234) each reconstruct
a table's MVCC version index from two sources: the persisted
`<mmap_path>.mvcc` history store (if any), and whatever the insert log
still holds since that store's last flush — read directly via
`insert_log::read_entries`, skipping the first
`insert_log_entries_reflected` entries the persisted store's own header
already accounts for.

This works correctly for a **fresh** table (no insert log exists yet) and
for a table reopened **after** `with_mvcc` has already flushed everything
current (`Compact`, or `with_mvcc`'s own baseline flush at first
`mvcc_begin`) — both cases were verified end-to-end in PR #234's own test
suite (`compact_flushes_mvcc_history_and_a_reopen_reconstructs_it` in
each of `memory.rs`/`entity.rs`/`relation.rs`).

It does **not** work for the case in between: a table with genuinely
pending, unflushed insert-log entries (ordinary `Insert`/`Replace`/
`Delete` calls made after the last flush) at the moment of a reopen.
Reading `src/generic/mmap_store.rs::GenericMmapStore::open` directly
(lines 641–723 as of this session) confirms why, precisely:

```rust
pub fn open(records: Vec<R>, path: &Path) -> Result<Self, DurabilityError> {
    let log = insert_log::log_path(path);
    let records = Self::merge_log(records, insert_log::read_entries(&log, R::SCHEMA_TAG)?);
    // ... blob currency check, slot-file reconciliation ...
    insert_log::clear(&log)?;   // <-- unconditional, every call, every domain
    Ok(Self { .. })
}
```

`insert_log::clear` runs **unconditionally**, at the end of every
`open()` call, with no parameter and no hook for a caller to intervene
between the fold and the clear. `open_portable` (`open(read_portable_records(path)?, path)`)
and every domain's own `create_..._production_stack`/
`open_..._production_stack[_portable]` helper (`Memory`/`Entity`/
`Relation`/`Reminder` front-door, `Order`/`Employee` `research`-gated —
`Dog` does not use `GenericMmapStore` at all, confirmed by grep) all
route through this same function for their primary record store. By the
time any of them returns and a server-layer `with_mvcc` call could run,
the insert log is already empty — not because it was correctly folded
into MVCC, but because `open()`'s own primary-state fold already
consumed and cleared it for an entirely different purpose (keeping the
companion blob current), with no notion of MVCC at all.

**What is and is not actually lost.** The *primary* record data is never
at risk — `merge_log`'s fold into the blob/slot file is exactly the same
whether or not MVCC is involved, and is already correct and already
tested. What is lost is purely the *derived* MVCC version-chain data for
whichever keys those pending entries touched: a snapshot that needed to
see one of those keys' pre-reopen-but-post-last-flush history would
either see the wrong (too-current) value or hit a `HistoryReclaimed`-shaped
gap, depending on exact timing — not a crash, not corruption, not a loss
of durability for anything but MVCC's own read-time correctness.

## Decision drivers

- **Zero risk to `Dog`/`Order`/`Employee`/`Reminder`'s existing crash
  recovery.** `GenericMmapStore::open` is shared, heavily tested,
  crash-recovery-critical code with no domain-specific knowledge today.
  Any fix must leave its behavior byte-for-byte identical for every
  caller that never uses MVCC — this is non-negotiable, matching
  `ADR-0072`'s own repeated "confirm no accidental shared-code edit
  leaks onto them" instruction (`MVCC2-FR-012`).
- **Don't duplicate `open()`'s reconciliation logic.** It is subtle,
  already-audited code (header checks, slot-file reconciliation, the
  blob-currency check, the "next free slot" `O_APPEND` race fix) — a
  second, independently-maintained copy of it is exactly the
  drift-prone shape this crate's own `write_slot_into`/`SlotFile`
  history has already named and avoided once before.
- **`insert_log::read_entries` is already a pure, non-destructive,
  `pub(crate)` read** — it "never touches the mmap file, never writes
  anything" (its own doc comment), and is already called directly by
  `with_mvcc` today. Any fix that can be built entirely from this
  existing primitive, without touching `mmap_store.rs` at all, is
  materially lower-risk than one that must.
- **Proportionality.** This gap is real but narrow: it only matters for
  a table that is (a) MVCC-active, (b) has pending insert-log entries at
  the exact moment of (c) a reopen. It is not a primary-data-loss bug,
  and no real deployment exists yet (`memory_server.rs` does not call
  `with_mvcc` today) — closing it is worth doing carefully, not urgently.

## Considered options

### (a) A new sibling function, `open_with_log_hook`, sharing `open`'s logic via an extracted private helper

Extract `open`'s current body into a private `fn open_impl(records, path,
on_pending_log: impl FnOnce(&[LogEntry<R, R::Id>])) -> Result<Self, ..>`
that calls `on_pending_log` with the raw, already-read (but not yet
cleared) log entries at the exact point `merge_log` already has them,
immediately before `insert_log::clear`. `open` becomes `open_impl(records,
path, |_| {})`; a new `pub(crate) fn open_with_log_hook(records, path,
on_pending_log) -> Result<Self, ..>` becomes the second, thin public
entry point. `open_portable` is unaffected (still `open(..)`); a new
`open_portable_with_log_hook` would be needed too, for parity with how
`with_mvcc` is actually called (via the `*_portable` helpers in
`memory.rs`'s/`entity.rs`'s/`relation.rs`'s own test suites and,
presumably, real restart code).

**Pros**: the hook fires at exactly the right moment, inside the same
function that reads the log, so there's no risk of it seeing stale data
or racing anything; one shared implementation, no duplicated
reconciliation logic; `open`'s own existing callers (`Dog` doesn't apply;
`Order`/`Employee`/`Reminder`) are provably untouched, since `open` itself
becomes a one-line wrapper with unchanged behavior.

**Cons**: touches `src/generic/mmap_store.rs` — the exact file this
round's decision drivers most want to avoid touching without dedicated
review — and touches it in all six domains' shared code path, even
though only three domains will ever use the new hook. Every domain's
own `create_..._production_stack`/`open_..._production_stack[_portable]`
helper (`memory.rs`/`entity.rs`/`relation.rs`, and *only* those three)
would also need a `..._with_mvcc_hook` sibling to actually expose the new
`open_with_log_hook`/`open_portable_with_log_hook` through their own
relationship-layer wrapping (`Ordered::new(MultiSymmetric::open(core,
..))` etc.) — a real, if mechanical, second layer of new sibling
functions on top of the generic-layer ones.

### (b) A pre-open read, composed entirely from existing `pub(crate)` primitives, no generic-layer change at all

Since `insert_log::read_entries`/`log_path` are already `pub(crate)` and
already called directly by `with_mvcc`, a caller can simply read the
pending log entries **before** calling the normal
`open_..._production_stack[_portable]` path at all — a plain,
non-destructive read that races nothing (this all happens single-threaded,
at startup, before the table is served to any connection). Concretely:
a new server-layer constructor per domain —
e.g. `MemoryConnectionStore::open_with_mvcc(path: &Path) -> Result<Self, DurabilityError>`
— that:

1. Reads `insert_log::read_entries(&insert_log::log_path(path), Memory::SCHEMA_TAG)`
   directly, capturing the *complete* pending entry list (not yet
   skipping anything — the skip count comes from the *persisted* `.mvcc`
   store's own header, read next).
2. Calls the existing `open_memory_production_stack_portable(path)` (or
   `open_memory_production_stack`, for a caller with its own record
   list) exactly as today — unchanged, including its own internal
   `GenericMmapStore::open` call and insert-log clear.
3. Calls `with_mvcc`'s existing reconstruction logic, but fed the
   pre-read entries from step 1 (skipping `insert_log_entries_reflected`
   of them, exactly as today) instead of re-reading the log itself —
   which by step 3's point is already empty.

**Pros**: zero changes to `src/generic/mmap_store.rs`, or to any other
domain's `create_.../open_...` helpers — the entire fix lives in the
three MVCC-capable adapters' own `with_mvcc`/new `open_with_mvcc`
functions, the files `ADR-0072` already touched and that already own
this concern. No new generic-layer public API surface at all. Verifiably
zero risk to `Dog`/`Order`/`Employee`/`Reminder`, since nothing they
depend on changes.

**Cons**: a real ordering foot-gun if left as two separate calls a
caller must remember to sequence correctly (read-log-then-open, not
open-then-read-log) — mitigated by bundling both steps into one new
`open_with_mvcc` constructor per domain (as sketched above) so there is
only one correct call site, not a documented-but-unenforced convention.
Reads the insert log twice in the reopen-with-pending-entries case (once
here, once inside `open()`'s own `merge_log`) — cheap (the log is
bounded by "since the last checkpoint/compact," the same size bound
`MVCC2-FR-010` already accepts elsewhere) but a real, if minor,
redundant I/O pass, and philosophically odd — reading a file specifically
so its own imminent read-and-clear doesn't lose data first.

### (c) Accept and more prominently document the limitation instead of closing it

Leave `with_mvcc` exactly as merged; strengthen the existing doc comment
(already present on `with_mvcc` in all three adapters) with the more
precise framing this document's own Context section gives — specifically,
that it is *not* limited to "chained after `with_journal`," and name the
exact compound condition (MVCC-active *and* pending insert-log entries
*and* a reopen) that triggers it, so a future implementer or operator
knows precisely when it applies rather than having to re-derive it.

**Pros**: zero code risk, zero new surface anywhere — the true "smallest
possible change." Proportionate to the gap's actual severity today: no
real deployment exists yet, so no real data is at risk from leaving this
open a while longer. Keeps the fix decision fully separate from
`ADR-0072`'s own already-large, already-merged change, easier to review
in isolation whenever it is picked up.

**Cons**: does not close the gap `MVCC2-FR-010` item 3 itself named as
part of the original design's own plan — a real, if narrow, correctness
gap remains for as long as this option stands. Every future round that
touches `with_mvcc` (client wiring, `Entity`/`Relation` integration
tests, a real `memory_server.rs` deployment) inherits an unclosed item
that a reviewer has to re-confirm is still acceptable each time.

## Recommendation

**Option (b), bundled into one `open_with_mvcc` constructor per domain.**
It closes the gap `MVCC2-FR-010` item 3 named, using primitives that
already exist and are already `pub(crate)`, without touching
`src/generic/mmap_store.rs` or any of the four domains that don't use
MVCC at all — the smallest change that actually closes the gap, not just
documents it more precisely. Option (a) is not wrong, and *is* the
more "textbook" place to put a hook like this — but it pays for that
with real risk to shared, crash-recovery-critical code and a second
layer of new sibling functions in every domain's own relationship-layer
wrapper, for a benefit (firing the hook at the theoretically-exact
right instant) that option (b) achieves in practice anyway, since
nothing else can write to a table between its own startup read and its
own `open()` call. Option (c) is a legitimate, honest fallback if the
owner judges this not worth spending review budget on yet — named here
as a real choice, not a strawman, given no real deployment currently
depends on closing this.

## Requirements (if option (b) is picked)

- `OPENHOOK-FR-001` — Each of `MemoryConnectionStore`/
  `EntityConnectionStore`/`RelationConnectionStore` gains
  `open_with_mvcc(path: &Path) -> Result<Self, DurabilityError>`
  (naming open to bikeshedding), performing, in order: read the complete,
  raw insert-log entry list; open the production stack via the existing
  portable-open helper; construct `Self` and fold in `.mvcc`'s persisted
  state plus the pre-read entries (skipping `insert_log_entries_reflected`
  of them), exactly as `with_mvcc`'s existing logic already does once the
  entries are in hand.
- `OPENHOOK-FR-002` — `with_mvcc` itself (the existing, already-merged
  builder) is **not removed** — it stays the correct choice for a caller
  that already has an open, freshly-created (never-yet-reopened) stack,
  exactly its current, tested use (including every existing test in
  `memory.rs`/`entity.rs`/`relation.rs`). `open_with_mvcc` is additive,
  for the reopen case specifically.
- `OPENHOOK-FR-003` — No change to `src/generic/mmap_store.rs`, any other
  domain's `create_.../open_...` helpers, or `Dog`/`Order`/`Employee`/
  `Reminder` — verified by `git diff --stat` showing changes confined to
  `src/server/{memory,entity,relation}.rs` (plus tests).
- `OPENHOOK-FR-004` — Idempotent under the same discipline
  `MVCC2-FR-010` already requires elsewhere: if `open_with_mvcc` reads N
  pending entries but a crash prevents `.mvcc` from being re-flushed
  before the *next* reopen, that next reopen's own pre-read must not
  double-count what the previous one already folded — already handled
  by `insert_log_entries_reflected`'s existing skip-count mechanism,
  confirmed to still apply unchanged.

## Acceptance criteria

1. A table with pending insert-log entries (some ordinary `Insert`/
   `Replace`/`Delete` calls made after the last `.mvcc` flush) at the
   moment of a simulated restart: `open_with_mvcc` on the reopened table
   correctly answers a snapshot that needs one of those entries' history
   — the scenario `with_mvcc` alone cannot handle today.
2. The identical scenario chained after a journaled table's own
   `with_journal`-equivalent replay (if/when that gap is picked up
   together — see Open questions) does not regress further.
3. Every existing `with_mvcc`/`Compact`-then-reopen test in `memory.rs`/
   `entity.rs`/`relation.rs` continues to pass unmodified.
4. `Dog`/`Order`/`Employee`/`Reminder`'s full existing test suites pass
   completely unmodified; `git diff --stat` confirms zero lines changed
   in `src/generic/mmap_store.rs` or any of their own production-stack
   helpers.
5. `cargo fmt`/`cargo clippy --all-targets -- -D warnings`/`cargo test
   --features server` all clean.

## Open questions

- This document scopes to the insert-log-remainder gap only, matching
  `ADR-0072`'s own "Not yet complete" wording. The *journal* remainder
  gap `with_mvcc`'s own doc comment also named (chaining after
  `with_journal`) is structurally the same shape but touches
  `journal.rs`'s replay path instead of `mmap_store.rs::open` — worth a
  follow-up decision on whether to fold both into one round or keep them
  separate; not decided here. *Closed by `ADR-0113` (replayed
  transactions) and `ADR-0114` (every replayed batch, the pending log
  read ahead of a journaled reopen — `open_with_mvcc_journaled`).*
- Whether `open_with_mvcc` should also subsume `Compact`'s own existing
  flush-before-clear responsibility, or remain purely an open-time
  concern — this document assumes the latter (no change to `compact()`
  in any adapter), since `Compact`'s own path was already verified
  correct in PR #234 and is not part of this gap.
- Naming: `open_with_mvcc` vs. threading a `bool`/enum flag through the
  existing `with_mvcc` to detect "was this a fresh construction or a
  reopen" automatically — the latter would remove the two-function
  surface at the cost of `with_mvcc` needing to know things about the
  caller's own construction history it doesn't today; not resolved here.
