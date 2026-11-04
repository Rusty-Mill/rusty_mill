# #599 build plan: lessons, track records, maintenance, decision replay

Status: build plan, 2026-11-04, after one Goldfish review pass (12 findings, folded in below). The approved design (`599-gaps-design.md`, posted
2026-11-02 in project files) is no longer available; this plan rebuilds it from
what survives: card #599's design comment, Nano's Q1–Q4 answers on the card, and
DECISIONS D53 and D58. Where this plan fills a gap the design left, it says so
and marks it **[filled]**. The design's approval (D58) covers the commands,
tables and port named there; nothing here adds to that list.

Consumers: #722 (hash-chained claim log) reuses `claim_history` and the track
records, so both live in libraries, not in a CLI.

## Ownership

| Piece | Owner | Why |
|---|---|---|
| `claim_history`, `Step`, `Rule` | BBP (`rusty_bbp`) | Pure fold over the log; the log is BBP's. |
| `bbp why --claim` | BBP host | Reads the store; prints JSON. |
| Track records (`lens_record`) | orch (`orch-lens` rule, `orch-store` table) | Jobs, blind passes and lenses are orch's. |
| Lessons (`rusty_orch lessons`, `lesson_written`) | orch | orch decides what a lesson is and writes it. |
| Lesson storage and search | remind-me | No change (D58): written over its HTTP add route. |
| Watcher (`rusty_orch watch`, `Watch` port, `watch_finding`) | orch | Posts claims into BBP tasks. |
| `canary` run on a released ref | BBP host | Runs a profile against a ref; the result is a report like any other. |

## 1. Decision replay (PR 1)

### `rusty_bbp::history`

`claim_history(task, events, claim, at) -> Option<History>`. Pure: no I/O, no
clock. Folds `TaskState` event by event (the same `apply` the store uses) and
records a `Step` for each event that changed the claim. `None` when the claim
was not posted by `at`. See `crates/libs/protocol/rusty_bbp/src/history.rs`.

- `History { claim, author, body: ClaimBody, steps }`: the whole body, so
  lessons get domains and `answers` without a second read.
- `Step { rev, change, status, score, confidence, epoch }`, the claim just
  after the event. `confidence` is for JSON readers; no rule reads it.
- `Change`: `Posted`, `Evidence { msg, author, kind, direction, delta,
  scoring }` (`scoring` as appended; a later reopen can wake it), `Status {
  from, rule }`, `Failing { profiles }`.
- `Rule`: `AcceptLine { accept_at, support_authors }`, `DropLine { drop_at,
  counter_authors }`, `Human { accept }`, `Choose { question }`,
  `Escalation`, `Reopen`, `Void { by }`, `Superseded { by }`.

Steps come from the claim's message, its evidence messages, `claim_failing`,
and the status events naming it. Budget, run-debit and action-mark events
change no score or status and make no step.

Rule attribution, from the log alone (no log change, D58). Every human
command ends with its `op_accepted` (engine.rs), so the events between two of
them are one command:

- An accept or drop is `Human` only if its command decided that outcome on
  that claim. A human reopen that the line then settles in the same command
  is the line's (Goldfish B1).
- A command that decided accept on one candidate of a question and drop on
  another is a `Choose`, both sides. A `choose` with no other live candidate
  writes only the accept and reads as a decide (Goldfish B2).
- Escalate, reopen, void and supersede carry their cause in the event.

`--at REV` stops the fold after that rev: the claim as it stood then.

### `bbp why --claim ID [--at REV]`

`bbp why --dir D (--task T | T) --claim ID [--at REV]` prints the `History` as
one line of JSON. `--claim` and `--turn` are exclusive. Unknown claim and
non-claims tasks are errors naming the cause. The turn replay is unchanged.

Tests: accept line, drop line, human decide, choose, reopen by evidence,
escalation, supersede, `--at` truncation, non-scoring evidence, unknown claim.

## 2. Track records

Informational only (D29, Q2): never a weight. Replaces #594's unimplemented
counts with `orch-store` table `lens_record`.

Scoring rule (`orch-lens::record`), per lens. Revised after the Goldfish
review (B3, S1–S4):

- A scoring job is a blind `Slot::Review` pass. Its direction is the net sign
  of its items; a mixed pass is unscored. `Propose`/`Rebel` post claims and do
  not score. `LookHarder` is a second blind job; it counts, since skipping it
  would otherwise be free. `Skipped` (infra) leaves the denominator.
- The denominator is blind review jobs assigned, so a job the lens answers
  with nothing counts as 0.
- A job scores on the claim's **latest** settlement, recomputed each fold:
  `Accepted` or `Dropped`. `Void` (a dep closed) and `Superseded` have no
  right direction and leave both the scored count and the denominator.
  Open and escalated claims are pending.
- **Leave-one-out.** `contested`, `upset` and the outcome are judged with the
  job's own items removed. If removing them would change whether the claim
  crossed its line, the job was pivotal and is unscored: a lens cannot earn
  by causing the outcome it bet on.
- Right is `+w`, wrong is `−w`, no post is 0. Points do not depend on strength.
- `w = 1 + contested + 2·upset` (1 to 4). `contested`: other authors' scoring
  items in both directions at settlement. `upset`: the score just before the
  job, without its own items, was non-zero and its sign is against the
  outcome. Score 0 has no tide.
- With `w` and the outcome independent of the job's own direction, a random
  direction nets 0 in expectation; safe bets earn 1 each.
- Report: `points`, `assigned`, `scored`, `pending`, `calibration_due` once
  `scored ≥ 20` (Q2).
- Replaces the `aligned`/`misaligned` counts in `orch-lens/src/record.rs`
  (whose `Void` counted as a counter win). `found_problems`,
  `found_nothing_twice` and `refused` stay as reported counts. The lens's one
  line becomes `record: P points over N blind reviews (S scored)`.

`claim_history` supplies "score just before the post" and "contested at
settlement" for each job.

## 3. Lessons

A lesson is a fact read from the log, not a model's judgment. Three triggers:

1. A candidate (a claim that answers a question) dropped by the drop line with
   executed counter evidence.
2. An accepted claim reverted within 14 days. **[filled]** The log has no wall
   time, so orch supplies the revert: a merged change for the claim reverted on
   the forge, timed from the merge receipt.
3. A human `choose` (`Rule::Choose` on the dropped siblings' histories).

`rusty_orch lessons --task T` writes each new lesson to remind-me as a
`learning` from a fixed template, with `project`, tags (`lesson`, trigger,
domains) and metadata (task, claim, trigger, rev). Sensitive when the claim's
data class is. `lesson_written (task, claim, trigger)` makes writing
idempotent. Written automatically; Nano deletes unwanted ones (Q1).

Before building, verify that remind-me's HTTP add route carries `project`
(D58); if it does not, stop and flag the additive fix.

The research lens finds lessons by remind-me search over the domains and cites
them as `cited` evidence (weight 2): a lesson informs and never decides.

## 4. Maintenance watcher

`rusty_orch watch --factory DIR --bbp-dir DIR [--bbp PATH] [--app NAME]
[--dry-run]` (`crates/apps/rusty_orch/crates/rusty_orch/src/watch.rs`) runs
daily on the BBP host (BaileyAi, D53; deploying is a separate step). It reads
`factory/apps/<app>.json` (D80) and `factory/watched-versions.json`
(`{ "rust": "1.99.0" }`; absent means nothing watched).

- Adapters (the `Watch` port), all CLI calls with no new crates:
  - `cargo-deny`: one finding per advisory and package, and one per yanked
    crate version.
  - `cargo-update`: one finding for the whole `cargo update --dry-run` set,
    keyed by its digest. One `cargo update` takes them all, and a finding per
    crate would post hundreds of claims.
  - `toolchain`: the app's `rust-toolchain.toml` channel against the watched
    Rust version.
  - `canary`: a re-run of `profile` on `released_ref`, capped by `watch_usd`.
    This is a later step, since it needs the BBP host's canary run.
- Findings are posted by the task's `watcher` role (D79) as low-stakes,
  reversible claims citing the brief. A claim needs a second author to
  accept, and a human still merges.
- **Posted once, through the approved `watch_finding` table (D82).** Each
  finding has a stable key and a slot, the thing it is about:

  | Finding | Key | Slot |
  |---|---|---|
  | Advisory | `advisory:<id>:<crate>` | the key |
  | Yanked crate | `yanked:<crate>:<version>` | the key |
  | Update set | `updates:<digest of the set>` | `updates` |
  | Toolchain | `toolchain:rust:<watched version>` | `toolchain:rust` |

  `watch_finding` (orch-store, `--state DIR`) keeps the key last posted and
  its claim for each app and slot.
  - The same key posts nothing, and the report gives the existing claim.
  - A new key in a slot that already has a claim posts a claim with
    `supersedes` naming the old one. Accepting the new claim retires the
    stale one. Examples: a new update set, or a new watched Rust.
  - An empty slot posts a new claim.

  Details that change under a key are reported in `detail` and never
  posted: an advisory's title or severity, or the app's current pin. The
  claim text comes from the key alone, so it never goes stale.

  BBP R15 covers a crash between the post and the table write:
  - The op id is a digest of the app and the key.
  - Every posted field comes from the key and the slot's previous claim.
    `refs` cites the opening brief `art:1`, not the current brief.
  - So the retry is byte-identical and replays the same claim. A changed
    payload under one op would be refused with `OpConflict`.
- An adapter that fails is reported in the JSON with its reason, the others
  still run, and the exit status is 1.

## 5. `rusty_orch lens why`

For one claim: the `bbp why --claim` history joined with orch's job records:
the job, lens, model and cost behind each evidence post.

## PR order

1. BBP: `history` module + `bbp why --claim/--at`. Merged as #1483.
2. orch: the watcher with three adapters, plus the expert's D79 to D81 and Q17.
3. orch: track records, lessons and `lens why`. This waits for the #594 stack
   (#1467, #1479), which owns the lens run state these read. Lessons also wait
   on the remind-me HTTP `project` check (D58).
4. BBP host + orch: the canary run.

Out of scope: the Mill Board timeline (Q4: later), using track records as
weights (Q2), deployment.
