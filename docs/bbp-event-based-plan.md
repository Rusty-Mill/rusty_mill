# BBP: from loop-based to event-based

Status: accepted by the owner, 2026-10-22. Step 1 merged (#880); step 2 approved.
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

- A new bbp MCP tool, `wait`: blocks until something the role may read is appended after its cursor, or the
  role is granted a turn, then returns what changed. The agent decides whether to act; acting still needs a
  granted turn, so R14 and token binding are unchanged.
- Launchers start one agent session per role for the whole task instead of one per turn. Cost accounting
  (`bbp tokens`, the cap) moves from per-turn logs to per-session logs with turn markers.
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

## What this needs from the owner

1. Approve this plan's order (1, 2, 3, then 4).
2. For step 4: the agent counts that matter (how many concurrent editors the factory is expected to run),
   so the load harness tests the right range.
3. Whether the 2b record stays as is (a loop-mode run) with a short note that it did not test this question.

## Out of scope

Character-level co-editing UIs, a networked multi-host store, and any change to admission rules.
