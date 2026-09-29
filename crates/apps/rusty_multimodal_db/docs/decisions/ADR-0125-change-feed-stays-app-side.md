# ADR-0125: The Change Feed Stays App-Side, Built From Existing Pieces

- Status: **Proposed** (2026-09-29), awaiting the owner. No code change to
  the engine; a recipe and a test.
- Date: 2026-09-29
- Deciders: baileyrd
- Related: issue #382 (gap 4, "Change feed for sync and multi-device"),
  `rusty_remind_me` ADR-0021 (`hub_seq`) and ADR-0024 (tombstones),
  ADR-0124 (the engine crate).
- Supersedes/Superseded by: none.

## Context

A task manager with web, mobile and desktop clients needs "everything
changed since version N", deletes included. The engine's `journal.rs` is a
redo journal, emptied at every checkpoint, so it is not a readable change
log. Issue #382 asked whether the engine should grow a per-store change
sequence and `changes_since(seq)`, or whether that stays app-side.

`rusty_remind_me`'s hub already serves this feed. Its `hub_seq` is a field
each write stamps, an `Ordered` layer keyed by it pages it, a delete is a
soft delete that takes a new `hub_seq` (ADR-0024 empties the text and never
purges the row), and the counter's floor survives compaction.

## Decision

**Keep the change feed in the application, built from three engine parts,
and add no engine API.**

1. **Stamp.** The record carries a `seq` field. The app takes each value
   from `Journal::allocate`, which is durable with the next commit and
   survives a checkpoint, so a sequence is never issued twice.
2. **Read.** An `Ordered` layer keyed by `seq` answers
   `page_by(Some((cursor, max_id)), limit)`: changes after the cursor,
   oldest first, costing the page.
3. **Delete.** A delete is a `replace` that sets a `deleted` flag and takes
   a new `seq`. The row is purged later, once the app knows every client has
   passed that `seq`; the engine cannot know that.

`tests/change_feed_recipe.rs` pins the recipe: edits and a tombstone
appear in order, paging by cursor, and a reopen after a checkpoint never
reissues a sequence.

## Why not an engine API

- The pieces that matter are policy the engine cannot own: what a change
  is (which field edits bump `seq`), when a tombstone may be purged, and
  the client cursor contract.
- The one consumer with a real feed today (`rusty_remind_me`) already
  does this, and its record shape (`hub_seq` beside `updated_at`) would
  not change under an engine `changes_since`.
- An engine-stamped sequence would have to be a hidden field of every
  record or a new on-disk file, which is a format change (a version
  bump) for a benefit two callers have not yet asked for.

## Consequences

- A second app that wants the feed copies about twenty lines. If a
  **third** does, or one needs the sequence stamped atomically with the
  record write (today the app allocates, commits, then writes), revisit
  with a `Stamped` layer; that is the point to spend a format version.
- The recipe does not make the allocate-then-write pair atomic: a crash
  between them can skip a sequence number. Skips are harmless to a
  "greater than N" reader; a client must not expect gap-free numbers.
- The feed is per store. One directory per user (gap 6) means one feed
  per user, which is what sync wants.

## Alternatives considered

- **An engine `changes_since(seq)` with a hidden sequence.** Rejected for
  now, per above.
- **Reading the journal.** Rejected: it is emptied at checkpoint and holds
  only unapplied batches.
