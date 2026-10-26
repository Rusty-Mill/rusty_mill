# BBP: from loop-based to event-based

Status: accepted by the owner, 2026-10-22. Steps 1, 2 and 3 merged (#880, #919, #932, #938, #1005); step 4 approved 2026-10-25; 4a merged (#1052).
Goal (owner): BBP reacts to events instead of running a loop. `rusty_crdt` is a means, not the goal.
Builds on `docs/bbp-event-driven-design.md` (accepted Phase 1) and `docs/bbp-concurrency-design.md` (2a/2b).

## Why this note exists

This account of the 2b evidence run (2026-10-22) is an **external report, not verifiable from this
repository**: its records (run logs, `bbp pairs` output, blind scores) are in the owner's project files
(`bbp-2b-run/`), not committed here. It grants no permission to run providers or spend; every phase keeps
its own owner approval. As reported, the run tested the loop:
`TurnBased` policy, a 500 ms moderator poll, a one-shot `claude -p` process per turn, and whole-spec
`put_artifact` snapshots. Its two planners each wrote a spec from scratch at the same time, so nothing was
ever co-edited, and its c4/c5 results say nothing about a CRDT. It also showed that a 500 ms poll is too
coarse to time an 11 s spec (c3 was unmeasurable). Both point at the same fix: stop polling.

## Where the loop is today

| Loop | Where | Event-based replacement |
| --- | --- | --- |
| The moderator sleeps `cfg.poll` (default 500 ms), then ticks the engine and checks every harness | `rusty_bbp_host/src/moderator.rs:710` | Block until the task log or a harness changes |
| The store cannot tell anyone it changed: `refresh` re-reads, there is no subscribe | `rusty_bbp/src/store.rs` (`Store`) | A change signal per task |
| Each turn is a fresh one-shot harness; an agent sees the board only when granted | `agents*.json`, `moderator.rs` launch | Long-lived agents that wait for a wake |
| Who acts next walks `turn_order` | `TurnBased` in `pick_next` | `EventDriven` (`wake_set` + `pick_from`): the default for one-live-turn tasks since step 2; `--policy turn-based` opts out, and tasks with several live turns stay turn-based (`engine.rs:298`) |

## Steps (one PR each, no stacking, each behind its own owner approval)

### 1. A push-based moderator (no protocol change)

- `FsStore` (`rusty_bbp/src/fs_store.rs`) is the only writer of a task log. The host adds a per-task change signal: the writer bumps it after
  a committed append; the moderator waits on it (inotify on the log file on Linux, a condition variable for
  `MemStore`, and the poll kept as a timeout fallback, not as the clock).
- Harness exits and runner reports already arrive as process events; the moderator waits on those too
  instead of checking them each poll.
- Expected effect: decisions follow the event that caused them, and moderator timing stops carrying poll
  error, which is what made c3 unmeasurable. The latency is to be measured in the implementation, not
  assumed from push delivery.
- Turn deadlines, `--max-wall-secs` and the cost cap are checked today on each poll. With pushes they must
  still fire when nothing happens: the moderator's wait has a timeout set by the nearest deadline.
- Tests: the existing moderator end-to-end tests with the poll set very long must still pass (proves the
  poll is no longer the clock); a test that an append wakes a waiting moderator.

### 2. Event-driven scheduling as the default (policy change, small code)

- `EventDriven` exists and passed the shadow comparison (50 agree, 3 fallback, 0 disagree over the 31
  traces). What remains: run it live on the proving tasks (one live turn), compare with `bbp why` and
  `bbp shadow`, then make it the default. `TurnBased` stays for replaying old logs.
- No CRDT: one live turn means one writer.

### 3. Long-lived agents woken by events (host and launcher change)

- A new bbp MCP tool, `wait`: blocks until the role is granted a turn or the task ends, then says which.
  Acting still needs a granted turn, so R14 and token binding are unchanged. **Status:** 3a (the tool and
  session-bound `bbp mcp`) is implemented; it wakes on grants only, since reading also needs a turn and a
  message addressed to the role is what makes `wake_set` grant one. 3b keeps the agent's context, not its
  process: an opt-in `"session": true` launcher gives every turn of a role one provider session id to
  resume, while each turn stays its own process, the only inference boundary the host can enforce (spec
  v0.5 §110). A long-lived process would need a bbp-owned adapter that drives the model API itself; the
  session `bbp mcp` and `wait` are kept for that. 3c lets a session run confined (one home per session,
  a fresh workspace at one path per turn) and under the cost cap, and the Claude launcher resumes its
  session; the Codex launcher refuses one, since Codex names its own sessions.
- Cost accounting (`bbp tokens`, the cap) stays per turn: each turn still has its own log. A resumed
  turn is marked beside its log and priced from what its own process counted
  (`model_usage_since_last_result`), since a resumed Claude Code session's totals include its earlier turns.
- No CRDT: still one live turn at a time, but agents now react to events instead of being polled into
  existence.

### 4. Several live agents on shared content (where a CRDT may belong)

Messages, verdicts and events are append-only, so concurrent agents never conflict on them (the
event-driven note, b1). The card is a fold and converges by construction. The only shared mutable thing is
an artifact several agents edit, today the spec. That is the one place `rusty_crdt` can fit, and only for
artifact content: the admission rules (R16, R17, T7) stay with the single transactional writer, because a
CRDT makes data converge but cannot make order-dependent rules hold (b1 stands).

**Design so a CRDT can slot in, without betting on it now:**

- Today an artifact is a snapshot: each `ArtifactStored` event holds a whole version, and a spec is immutable
  per version (`docs/bbp-concurrency-design.md`). There is no edit op, no section-level last-write-wins and
  no stale-base refusal; all three below are **proposed**, not existing behavior.
- Proposed: artifact edits become **ops appended to the task log** (insert/delete against a stated base
  version, plus the author's turn), and the artifact's text is a fold over its ops, like the card. The
  baseline fold would be last write wins per section, with a base check that refuses a stale edit.
- An RGA fold from `rusty_crdt` would be a second fold over those ops, but only if the op format carries
  what RGA needs: stable element identities and their parents, not just positions relative to a base.
  Base-relative edits alone do not give that. Whether one op format serves both folds, and whether old
  snapshot logs replay under the new schema, are **to be validated** in step 4's design, not assumed here.
- A stale-base refusal is the measurable event: each one is a merge a CRDT would have done automatically.

**Measure at realistic agent counts, not with two agents.** Contention grows with the number of concurrent
editors and their edit rate, so a two-agent probe cannot rule a CRDT out (owner, 2026-10-22). Instead:

- A load harness of **scripted editors** (no model, no API spend) driving the real engine and store:
  N = 2, 4, 8, 16, 32 editors on one artifact, with section overlap and edit rate as parameters. Edit
  rates and sizes are taken from real model traces (the 2b logs give planner timings and spec sizes).
- Report per N: stale refusals per edit, the extra work to rebase each one, lost edits under
  last-write-wins, and the same under an RGA fold (built in the harness, not in BBP).
- A small live-model check (a few runs at N = 4) confirms the scripted rates are realistic.
- Decision rule, agreed before running: if stale refusals stay rare and cheap at the agent counts the
  factory will actually use, last-write-wins with a base check is enough. If they grow so that rebasing
  costs a large share of work or edits are lost, build `rusty_crdt` as the artifact fold.

### Step 4a: what was built, and the rule fixed before the first harness run

Approved 2026-10-25. No provider runs and no spend: the live N = 4 check waits for its own approval.

**Op format (built).** A co-edited spec version is an `ArtifactStored` whose payload is
`SpecEdit { brief, base, ops }` (`rusty_bbp/src/coedit.rs`); `base: None` starts a spec. An op is
`Insert { after, lines }` or `Delete { line }`. Every line has a stable id, the version that inserted
it and its index there, and a deleted line stays as a tombstone. Ops name lines by id, so an edit
written against an old base still applies to the head. Each version is its own `ArtId` with its own
immutable text (`TaskState::doc`), so a plan-gate request and the owner's approval still bind one
exact text, and immutable-per-version holds.

**One fold serves both.** The fold inserts directly after the anchor, newest first. When ids follow
the log, a new line is always the newest sibling, which is exactly where RGA puts it; a property test
checks the fold against a textbook RGA fed the same edits in random causal orders. So the op format
already carries what RGA needs (stable ids and parents), and an RGA fold of these ops gives the same
text. What `rusty_crdt` would change is **admission**, not the format or the fold: accept every edit
instead of refusing a stale one. Building it as "the artifact fold" therefore means removing a check,
not adding a crate.

**Admission (built).** An edit is refused with `stale_base` when an op is anchored in a section that a
version accepted after its base also touched. A section is a heading line and the lines up to the next
heading; both sides are measured against the base's headings, so two edits of one line always clash
however headings moved. An edit in an untouched section is applied to the head (rebased by id). A
refused edit is logged as `rejected`, charges no bytes, and is the measurable event.

**Old logs.** Snapshot specs are unchanged and never an edit base (`wrong_kind`), so every existing log
replays as before. A build without `SpecEdit` fails loudly on a log that has one (an unknown variant),
so no schema bump is needed to keep an old build from misreading it.

**Gate binding (built, 2026-10-26).** A plan-gate request on a co-edited version is refused with
`stale_subject` unless that version is the head of its spec, since approving an older version would
drop every edit accepted after it. Specs are writable only in planning, so the head cannot move once
the gate is open.

**Harness (`rusty_bbp/tests/coedit_load.rs`).** Scripted editors drive the real `Driver`, engine and
`MemStore` (one smoke run uses `FsStore`) in virtual time, so runs are deterministic and cost nothing.
Each editor holds a planner role in an N-turn task. It reads the head through `get_artifact`, thinks,
then submits one edit to one section: insert 1 to 3 lines, or replace one line. On `stale_base` it
re-reads and redoes the edit, a full new think (the conservative cost of a rebase). The fixed workload:

- Spec: 4 sections of 8 lines (32 lines), the median shape of the 2b specs (22 to 49 lines, 3 to 5
  headings, about 1.1 to 3 KB).
- Think time: uniform 10 to 17 s per edit, the 2b planners' spec times. Editors work back to back
  (no idle time), the worst case for contention.
- Section choice: editor i owns section i mod S; with probability `overlap` it edits a uniformly
  random section instead.
- Grid: N = 2, 4, 8, 16, 32; overlap 0, 0.25, 1; S = 4 and S = N; 20 accepted edits per editor;
  5 seeds per cell.
- Reported per cell: refused edits per accepted edit; wasted work (think time of refused attempts over
  all think time); same-line clashes per accepted edit (refusals where both edits name one line, what a
  line-level check would still refuse). Under blind last-write-wins each refusal is a lost edit; under
  RGA each is an automatic merge, and a same-line one is a merge of two edits of one line.

**Decision rule, fixed before the first run** (decision cell: N = 4, S = 4, overlap 0.25):

1. Refused per edit at most 0.10 **and** wasted work at most 10%: the section check is enough. Keep it.
2. Otherwise, same-line clashes per edit at most 0.10: refine the check to lines (same ops, same fold,
   still no CRDT).
3. Otherwise: accept every edit (the RGA admission) and flag same-line merges for review.

The thresholds are this PR's proposal and need the owner's confirmation; the results are reported
against them as written here.

### Step 4a: result of the harness (2026-10-25)

Run at this PR's head with the workload above (`report` in `tests/coedit_load.rs`, 5 seeds per cell,
deterministic, about 13 minutes):

| N | sections | overlap | accepted | refused/edit | wasted work | same-line/edit |
| ---: | ---: | ---: | ---: | ---: | ---: | ---: |
| 2 | 4 | 0.00 | 200 | 0.00 | 0% | 0.00 |
| 2 | 4 | 0.25 | 200 | 0.05 | 4% | 0.01 |
| 2 | 4 | 1.00 | 200 | 0.20 | 17% | 0.04 |
| 2 | 2 | 0.00 | 200 | 0.00 | 0% | 0.00 |
| 2 | 2 | 0.25 | 200 | 0.15 | 13% | 0.01 |
| 2 | 2 | 1.00 | 200 | 0.41 | 30% | 0.03 |
| 4 | 4 | 0.00 | 400 | 0.00 | 0% | 0.00 |
| 4 | 4 | 0.25 | 400 | 0.21 | 18% | 0.02 |
| 4 | 4 | 1.00 | 400 | 0.74 | 43% | 0.04 |
| 8 | 4 | 0.00 | 800 | 0.69 | 43% | 0.03 |
| 8 | 4 | 0.25 | 800 | 0.97 | 50% | 0.04 |
| 8 | 4 | 1.00 | 800 | 1.49 | 61% | 0.07 |
| 8 | 8 | 0.00 | 800 | 0.00 | 0% | 0.00 |
| 8 | 8 | 0.25 | 800 | 0.24 | 19% | 0.02 |
| 8 | 8 | 1.00 | 800 | 0.77 | 44% | 0.05 |
| 16 | 4 | 0.00 | 1600 | 2.12 | 70% | 0.08 |
| 16 | 4 | 0.25 | 1600 | 2.45 | 73% | 0.10 |
| 16 | 4 | 1.00 | 1600 | 3.17 | 77% | 0.10 |
| 16 | 16 | 0.00 | 1600 | 0.00 | 0% | 0.00 |
| 16 | 16 | 0.25 | 1600 | 0.26 | 21% | 0.02 |
| 16 | 16 | 1.00 | 1600 | 0.81 | 46% | 0.05 |
| 32 | 4 | 0.00 | 3200 | 5.15 | 85% | 0.10 |
| 32 | 4 | 0.25 | 3200 | 5.44 | 86% | 0.13 |
| 32 | 4 | 1.00 | 3200 | 6.25 | 88% | 0.12 |
| 32 | 32 | 0.00 | 3200 | 0.00 | 0% | 0.00 |
| 32 | 32 | 0.25 | 3200 | 0.26 | 21% | 0.02 |
| 32 | 32 | 1.00 | 3200 | 0.89 | 48% | 0.06 |

**Against the rule, at the decision cell (N = 4, S = 4, overlap 0.25):** 0.21 refused per edit and 18%
wasted work, so the section check is **not** enough (rule 1 fails). Same-line clashes are 0.02 per
edit, under 0.10, so **rule 2 applies: refine the check to lines**, with the same ops and fold and no CRDT.

What the grid says beyond the decision cell:

- Refusals follow editors per section, not editors. With a section per editor (S = N) they stay flat
  from N = 4 to 32 (0.21 to 0.26 per edit at overlap 0.25); four sections shared by 16 or 32 editors
  waste 70% to 88% of the work.
- Same-line clashes stay at or under 0.13 per edit everywhere, and at or under 0.06 whenever S = N.
  Most section refusals are edits that did not touch the same line, which a line check would accept.
- Caveat: same-line clashes are counted among the attempts the section check refused, in its timeline.
  A line check refuses less, so its own runs have fewer retries; the count is an estimate, not a
  measurement of the line check. A rerun after the refinement measures it directly.
- Under blind last-write-wins every refusal above would be a lost edit; under RGA admission every one
  would be a silent merge, the same-line ones being merges of two edits of one line.

### Step 4b: the line check (2026-10-25)

The owner confirmed the thresholds and rule 2 on 2026-10-25. An edit is now refused with `stale_base` only
when an op is anchored on a line (or the start of the spec) that an edit accepted since its base also
anchored on: deleted it, or placed lines against it. Sections are gone from admission; a heading is an
ordinary line. Same ops, same fold, no CRDT. The same harness and workload, rerun on this check, with
`merged/edit` the share of accepted edits whose base was not the head (concurrent work kept by id):

| N | sections | overlap | accepted | refused/edit | wasted work | merged/edit |
| ---: | ---: | ---: | ---: | ---: | ---: | ---: |
| 2 | 4 | 0.00 | 200 | 0.00 | 0% | 0.89 |
| 2 | 4 | 0.25 | 200 | 0.01 | 0% | 0.89 |
| 2 | 4 | 1.00 | 200 | 0.03 | 3% | 0.85 |
| 2 | 2 | 0.00 | 200 | 0.00 | 0% | 0.89 |
| 2 | 2 | 0.25 | 200 | 0.02 | 2% | 0.86 |
| 2 | 2 | 1.00 | 200 | 0.02 | 2% | 0.86 |
| 4 | 4 | 0.00 | 400 | 0.00 | 0% | 0.98 |
| 4 | 4 | 0.25 | 400 | 0.03 | 3% | 0.98 |
| 4 | 4 | 1.00 | 400 | 0.04 | 4% | 0.98 |
| 8 | 4 | 0.00 | 800 | 0.04 | 4% | 0.99 |
| 8 | 4 | 0.25 | 800 | 0.06 | 6% | 0.99 |
| 8 | 4 | 1.00 | 800 | 0.09 | 8% | 0.99 |
| 8 | 8 | 0.00 | 800 | 0.00 | 0% | 0.99 |
| 8 | 8 | 0.25 | 800 | 0.03 | 3% | 0.99 |
| 8 | 8 | 1.00 | 800 | 0.06 | 6% | 0.99 |
| 16 | 4 | 0.00 | 1600 | 0.09 | 8% | 0.99 |
| 16 | 4 | 0.25 | 1600 | 0.11 | 10% | 1.00 |
| 16 | 4 | 1.00 | 1600 | 0.11 | 10% | 1.00 |
| 16 | 16 | 0.00 | 1600 | 0.00 | 0% | 1.00 |
| 16 | 16 | 0.25 | 1600 | 0.03 | 3% | 1.00 |
| 16 | 16 | 1.00 | 1600 | 0.06 | 6% | 0.99 |
| 32 | 4 | 0.00 | 3200 | 0.12 | 11% | 1.00 |
| 32 | 4 | 0.25 | 3200 | 0.14 | 12% | 1.00 |
| 32 | 4 | 1.00 | 3200 | 0.15 | 13% | 1.00 |
| 32 | 32 | 0.00 | 3200 | 0.00 | 0% | 1.00 |
| 32 | 32 | 0.25 | 3200 | 0.02 | 2% | 1.00 |
| 32 | 32 | 1.00 | 3200 | 0.08 | 7% | 1.00 |

**At the decision cell (N = 4, S = 4, overlap 0.25):** 0.03 refused per edit and 3% wasted work, within
rule 1's bars (0.10 and 10%), against 0.21 and 18% for the section check. 98% of accepted edits were
made on a base that was no longer the head and still landed. Refusals pass the bars at every N with a
section per editor; the four-section spec crosses them only at 16 editors (overlap 0.25 and up) and
32. The line check stays; a CRDT would remove the remaining 3% by accepting edits of one line, which
then need someone to reconcile them.

## What this needs from the owner

1. Approve this plan's order (1, 2, 3, then 4).
2. For step 4: the agent counts that matter. Answered 2026-10-23: 4 concurrent editors on the same
   artifact.
3. Whether the 2b record stays as is (a loop-mode run) with a short note that it did not test this question.

## Out of scope

Character-level co-editing UIs, a networked multi-host store, and any change to admission rules.
