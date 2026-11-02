# BBP claims addendum: notes after the freeze

Nov 2, 2026 · Card: Mill Board #593. Amends [`bbp-claims-addendum.md`](bbp-claims-addendum.md) (PR #1386, frozen at a1b741b2). The engine is in PR #1389. Fold these notes into the addendum the next time it is edited.

## A claim resting on a closed dep stays dead (owner, 2026-11-02)

Found by the CL-24 property test. Executed evidence against a settled outcome reopens the claim (Evidence after settlement). Without this rule it could also reopen a `dropped` claim whose own dep is `dropped`, `void` or `superseded`, leaving an `open` claim resting on a closed dep.

- **Evidence is recorded, not reopening.** Executed evidence against the outcome of a claim with a closed dep is recorded only. The item is marked **dormant**.
- **Human reopen is refused while a dep is closed.** C-R8 `reopen` on such a claim is refused `dep_closed`, and the detail names the closed dep and its status. Reopen the dep first.
- **After the dep reopens.** Once the dep is reopened, the dependent is eligible again. A human `reopen` starts its new epoch with every dormant item as scoring evidence, with no rerun needed, and the board evaluates the claim at once. Replaying the log gives the same result, because dormancy is decided by the fold from the claim and dep statuses at each item's commit.
- **Trace CL-14b.**
  1. B depends on A, and both are dropped.
  2. A passing run of B's candidate is cited as executed support. It is recorded and dormant, and B stays `dropped`.
  3. Human `reopen(B)` is refused `dep_closed`, naming A as `DROPPED`.
  4. Human `reopen(A)`, then `reopen(B)`: B is `open` in epoch 2 with score +4.
  5. Further support leaves B waiting on A. Once A is accepted, B is accepted.
  6. The fold equals replay.

## `tests_changed` carries the change kind

This is already in the addendum at a1b741b2, listed here for the engine's record. `Report.tests_changed` is `[{path, change: added | modified | deleted}]`.

- **Clearing a block.** A passing report clears `failing(c)` only if the list is empty or the candidate is endorsed.
- **Executed support.** Executed support citing a passing report needs an endorsement when either:
  - some entry is `modified` or `deleted`, or
  - some entry is `added` and `failing(c)` is non-empty.

  Adding tests alone needs no endorsement (owner, 2026-11-02).

## A superseded dep collapses like a dropped one (owner, 2026-11-02)

A dep that becomes `superseded` is treated exactly like one that is `dropped` or `void`:

- An unacted dependent is voided.
- An acted-on dependent escalates `dep_collapsed`. It is never voided automatically, even if a human has reopened it since it was acted on.

Trace CL-70 covers both cases for a superseded dep and for a dropped one.

## Credentials end with the task (owner, 2026-11-02)

After `close` or `cancel`, the session credential is invalid (C-R1). Every claims-mode agent call is refused `bad_credential`, including views and replays of earlier ops. Retries during the task are authenticated before a cached response is returned. Humans keep read access to the log through the host. Traces: CL-64 and CL-67.

## `evidence_base` is bounded (owner, 2026-11-02)

`evidence_base` must be between 1 and 2^20 (`ClaimsConfig::MAX_EVIDENCE_BASE`), or the open is refused `bad_claims_config`. Extending a budget uses checked arithmetic and refuses `claim_budget` rather than wrapping. Trace: CL-69.
