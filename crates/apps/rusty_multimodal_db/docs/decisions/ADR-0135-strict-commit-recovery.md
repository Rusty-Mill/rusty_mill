# ADR-0135: Strict Commit Recovery — a Durable Acceptance, Then Redo

- Status: **Accepted** — the monorepo design review remediation (Tranche 2, rows 2.1–2.2), on the owner's "go with your recommendations".
- Date: 2026-09-30
- Deciders: baileyrd
- Related: `ADR-0133` (strict commit), `ADR-0063` (atomic `WriteBatch` journal),
  `ADR-0025`/`ADR-0026` (redo journal, group commit), `ADR-0112` (`RVL-FR-004`),
  `docs/Monorepo_Reviews/DESIGN-REVIEW-2026-09-30-PLAN.md` at the repo root.
- Journal format 3 (version 2 still opens).

## Context

ADR-0133 journals a strict batch before its check runs, and replay decided
again by re-running the check, assuming replay sees the same pre-state. The
design review found two defects from that assumption.

1. **A refused batch could apply after a restart.** `Memory` and `Entity`
   also checked every link's left endpoint against the store *before* the
   batch. So `[Insert(A), Link(A, B)]` passed the strict overlay but was
   refused live (`RecordNotFound`). Its entry stayed in the journal. Replay ran
   only the overlay check, which passes, and applied the batch the client was
   told failed.
2. **A partly applied batch was skipped on replay.** The store's own insert
   log makes each insert durable as it lands. A crash inside
   `[Insert(A), Insert(B)]` after A leaves A on disk. Replay's check then saw
   A as a duplicate and skipped the whole batch, leaving B missing.

No check can tell "refused live" from "accepted, partly applied" by looking at
a state that may already hold part of the batch. So the decision has to be
recorded, not recomputed.

## Decision

1. **A strict-accepted marker.** Journal entry kind `3` carries the byte offset
   of a strict entry. The batch's apply writes and syncs the marker after its
   check passes and before its first write (`CommitGroup::accept_strict`). A
   refusal writes nothing. The batch's own apply turn keeps a checkpoint from
   truncating the entry first. Offsets identify entries until the next
   truncate, which drops the markers too.
2. **Replay:** a strict entry with no marker is skipped: it never wrote
   anything, and the client was told it was refused, or never got an answer.
   An entry with a marker is **redone whole**: every op is re-applied in
   order, from whatever part of it reached the store. Soft outcomes of the
   redo are ignored, because they are the ops that already landed. A `Link`
   whose endpoint a later op of the same batch deleted is `NotFound` rather
   than a replay error, since the edge went with the record (the same idea as
   `RVL-FR-004`). This applies to loose atomic `Write` entries too, where the
   same crash window would otherwise stop the server opening.
3. **The live check is the overlay alone for strict batches.** The pre-state
   link-endpoint check stays for loose atomic batches only. The overlay
   already sees each endpoint as the batch's earlier ops leave it.
4. **Format 3.** A version-2 journal still opens. It has no markers, so its
   strict entries replay under the old rule (re-run the check). The truncate
   that follows every replay rewrites the header as version 3.

Why redo converges: each op sets a record's state (insert, replace, field
write, delete) or adds an edge. An accepted batch applied from any durable
prefix of itself, skipping ops that already hold, reaches the same final
state. The property test runs random accepted batches over six ids through
every crash prefix and reopens the same files.

## Consequences

- One extra `fsync` per accepted strict commit, which is opt-in. Loose and
  update-only commits are unchanged.
- A hard error after acceptance (`Storage`) is indeterminate to the client.
  After a restart the batch completes, which is what "all or nothing" means
  for an accepted batch. The same holds when the marker's own sync fails
  after the write reached the disk.
- `[Insert(A), Link(A, B)]` now commits strictly, and links to a record
  inserted earlier in the same batch.

## Tests

- `insert_then_link_commits_strictly_and_a_restart_agrees` (`Memory`,
  `Entity`): live success, then the same files reopened agree. Refused
  batches (a duplicate, a self-loop) never appear.
- `an_accepted_strict_batch_is_redone_whole_from_any_crash_prefix`: 40 random
  accepted batches, every crash prefix, reopened; an unaccepted entry applies
  nothing. It fails if replay re-runs the check, or if the link tolerance is
  removed.
- Journal: a marker accepts only the entry it names, and one naming no entry is
  `Format`; a version-2 journal opens undecided and its truncate upgrades it.
