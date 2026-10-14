# BBP event-driven participation: design

Status: Phase 1 accepted, 2026-10-13; reconciled with the merged v0.5 engine the same day (see "Reconciliation"). Docs only; nothing here is implemented yet.
Inputs: `crates/libs/protocol/rusty_bbp/docs/spec/bbp-v0.5-implementation-spec.md` (v0.5), `docs/bbp-role-audit.md` (the nine Role fields), engine and host sources cited inline. Line numbers are as of `main` at `e2605657`.

## Amendments (owner decisions, 2026-10-13)

1. **Accepted as Phase 1 only.** Shadow-mode `wake_set` logging "would wake X" next to the real grant. No `EventDriven` scheduling is built until that evidence exists.
2. **The CRDT rejection is conditional, not a decision.** Where this note rejects concurrent live turns and CRDT merge (the Recommendation's "Why" and Option 3's verdict), it does so on the premise that exactly one turn is live. That premise is now open; see `docs/bbp-concurrency-design.md`.
3. **`pick_next` returns a set of roles, not one.** Its cardinality is capped by the scheduling policy: `TurnBased` and Phase 1 cap it at one, which is today's behavior. A larger cap is a concurrency-design decision (`max_live_turns`). Read `Option<RoleId>` below as "a set of at most one role".
4. **`max_chain` is deferred** until shadow-mode evidence. Open question 3 is answered: loop control stays at `turns` only until measured.

## Reconciliation with the merged v0.5 engine (2026-10-13)

The note was written before the v0.5 engine landed (#643, #644, #645). The sections below are updated to the code on `main`. Owner decisions that shaped the update:

- **A.** `wake_set(st: &TaskState) -> Vec<(RoleId, WakeReason)>`. The roster is `st.roster`, so no second argument.
- **B.** `pick_next` is the best role in `wake_set`, else the state's `default_holder`. Without the fallback a passing default holder is never woken again (the watermark suppresses the state trigger) and the task stalls with no live turn.
- **C.** Shadow decisions are returned from `handle()` in `Handled.shadow` and are **not logged**: a log event would change `rev` on every trace. The host logs them, deduped on `rev`, so real runs give evidence and not only tests.
- **D.** Priority: `Addressed` > `State` > `Artifact` > `Observer`; ties by `turn_order`; applied after self-exclusion, cooldown, visibility and capability gating. (Revised after the shadow comparison; the first order was `Addressed` > `Artifact` > `State`. See "Decisions after the shadow comparison".)
- **E.** T-final is dropped, and T-consult and T-reply are explicit grants that bypass `pick_next`.
- **Sequencing:** one PR per step, no stacking: this reconciliation and `wake_set` in parallel from `main`; the `SchedulingPolicy` seam, then shadow mode with host logging, each after the previous merge. Live mode waits for the owner's approval of the shadow comparison.

What changed against the original text:

| Original | On `main` | Effect |
| --- | --- | --- |
| The nine fields, `Roster`, `default_holder`, `turn_order` are proposals | They exist (`role.rs`) | Names and lines updated |
| T-state: `build` needs `write ∋ candidate`; `review` needs `verdict_at ∋ review` | `REQUIREMENTS` is richer: `planning` also needs `post ∋ request_decision`; `build` needs `write ⊇ {diff, candidate}`; `test` and `review` need `post ∋ verdict`; `review` needs `final_decision` | T-state reuses `Requirement::served_by` |
| T-final | Redundant: the task enters `review` exactly when a test approve exists, and the `review` row already requires `final_decision` | Dropped (E) |
| T-consult and T-reply are triggers | Yield, human `ask`, request settlement and the consultation return are direct `grant()` calls that never pass `schedule()`; T-reply is not derivable from state because the request is cleared on settle | Explicit grants (E) |
| `seen_rev` "derivable from `turn_ended`, no new record" | True for the log, but `TaskState` held no per-role turn-end rev, no append rev per message or artifact, and `TurnEnded` has no role | Four fold-derived fields (not logged): `last_turn_end`, `msg_rev`, `art_rev`, `ready_rev` |
| `pick_next` "highest priority" | Undefined | D and B |
| `SchedulingPolicy` in `task_opened` | Verified: a field marked `#[rusty_serde(default)]` on an enum variant deserializes when absent, so the field is additive, needs no schema bump, and old logs replay as `TurnBased` | Spec gets a short paragraph in the seam PR |
| Capability gating "reuses R5" | `state_accepts` is private to `engine.rs` | Made `pub(crate)` |
| `Command::Tick` | Calls `schedule()` on every moderator poll | Shadow logging dedupes on `rev` |

Decided after the shadow comparison: see "Decisions after the shadow comparison" below.

## Recommendation

Add **BBP-side wake derivation, with delivery owned by the host/orch**:

1. BBP (pure core) gains a pure function `wake_set(st: &TaskState) -> Vec<(RoleId, WakeReason)>` that says which roles are *eligible* for a turn given the board and each role's capability set. BBP stays I/O-free; it emits no callbacks.
2. The single live turn stays. What changes is *who gets the next grant*: a second scheduling policy (`EventDriven`) picks among eligible roles instead of walking `turn_order`. Turn-based remains the default and every current test stays valid.
3. Delivery (telling a sleeping agent process "look now") lives in the host adapter (`rusty_bbp_host`, and orch's equivalent), as a wake on the append-only log's `rev`. It is not a BBP concern, not a CRDT concern, and needs no new store.

Why: what stops replays, stale writes and ping-pong today is "only one turn is live" plus token binding (R14). Keeping one live turn and changing only *who is chosen* keeps those guarantees without re-deriving them. Concurrent live turns and CRDT merge buy parallelism BBP does not yet need, and cost the single-writer log the spec calls load-bearing ("A freely writable replicated log alone cannot enforce BBP's admission rules; one transactional authority is required", spec Appendix A).

Honest limit: this gives "jump in when relevant" at turn granularity. An agent interrupts the *schedule*, not a live peer. Simultaneous editing is out of scope (see end).

## What exists today (grounded)

| Fact | Where |
| --- | --- |
| Store is a single-writer, transactional, append-only log; `append(task, expected_rev, events)` is optimistic on `rev`. | `rusty_bbp/src/store.rs:31-37` |
| `Store::refresh` lets one writer see another's appends; there is no subscribe or notify. | `store.rs:41` |
| Exactly one turn is live: `grant()` returns early if `w.st.turn.is_some()` or the role is unassigned. | `engine.rs:1359-1362` |
| One scheduling check decides every default grant: no live turn, no pending request, task opened, the state has a `default_holder`, and in `test` the selected run has a final report. `Tick` calls it on every poll. | `engine.rs:1374-1385`, `:159` |
| Other grants are direct: a yield (`post`), a human `ask` and a request settlement (`human_post`), the consultation return (`end_turn`). None passes `schedule()`. | `engine.rs:1273-1350`, `:1394-1410` |
| Token check: role equals the live turn's role, token equals that turn's, and `now < deadline`. | `engine.rs:238-258` |
| The moderator loop polls: `sync()`, `dispatch(Tick)`, reap, launch one harness per granted turn, `sleep(cfg.poll)`. | `rusty_bbp_host/src/moderator.rs:262-353` |
| Agents pull (`read`, `get_artifact`); the moderator never forwards content. | spec "Design principles", "Reads and visibility" |
| The nine Role fields, the roster and the requirement table that derives each state's default holder. | `role.rs` (`REQUIREMENTS` `:163`, `default_holder` `:335`) |

So today: a *pull* model with a *poll* wake. The host loop is the only thing that wakes anyone, and it wakes whoever the state machine names.

The turn loop exists because separate model processes share no runtime or bus. That is a property of the host (it launches a harness per turn, `moderator.rs:334`), not something the log forces: the store already tolerates several processes appending via `expected_rev` + `refresh`.

## a. Trigger conditions

An agent is *eligible* when board state matches a trigger derived from its capability set. Triggers use only the nine Role fields, so BBP names no role (consistent with v0.5: roles are roster data).

| Trigger | Fires when | Role fields | Maps to today's code |
| --- | --- | --- | --- |
| Addressed | A message whose `to` contains the role's id or `*`, newer than the role's last turn end, authored by someone else, not a `pass`. | `id`, `read_messages` | R9 recipient check; `visible()` |
| Artifact | A spec, diff, candidate or test report of a kind in `read_artifacts` was stored, newer than the role's last turn end, by someone else. | `read_artifacts` | `get_artifact` read capability |
| State | The role serves the current working state (`Requirement::served_by`: the capabilities of its `REQUIREMENTS` row) and that state's turn became available since the role's last turn end. | `write`, `post`, `verdict_at`, `final_decision` | `Roster::default_holder`; the `test` run guard in `schedule()` |

Explicit grants are not triggers. A yield, a human `ask`, a settled request and a consultation return name their grantee directly, bypass `schedule()` and so bypass `pick_next`. The request-settlement regrant cannot be derived from state anyway, because the request is cleared when it settles.

Rules that keep a trigger honest:

- **Visibility first.** A trigger fires only on records the role may read. A `scoped` reader is never woken by a message it cannot read; otherwise the wake leaks the existence of hidden records (spec Goal 4, fresh reviews by construction).
- **Capability-gated, not curiosity-gated.** Addressed and Artifact wake a role only if it can *act*: it holds some `post` kind other than `pass` that the current state accepts (`state_accepts`, the by-state table). A `verdict` counts only if the role also holds `verdict_at` for the current stage, the same admission as `check_verdict` (R6); a post-only role would be woken and then rejected. A role whose every postable kind would be rejected is not eligible.
- **Watermark.** Each role has a `seen_rev`, the card `rev` of the event that ended its last turn (0 if it has had none). A trigger counts only for events with `rev > seen_rev`. `TaskState` derives it, with the append rev of every message and artifact and `ready_rev` (the rev of the last event that made the state's turn available), while folding the log. They are not logged, so the log format is unchanged.
- **Human gates dominate.** In `plan_gate`, `merge_gate`, `approved` and `escalated` there is no default turn (spec "Turns"), so `wake_set` is empty; a human `ask` is an explicit grant. Event-driven mode does not change that.

Order: `Addressed` > `State` > `Artifact` > `Observer`, ties by `turn_order`, one entry per role (its best reason).

`WakeReason` is a closed enum with four variants, in priority order. It is computed, not stored, so the log format is unaffected.

## b. Two separate concerns

### b1. MERGE: what happens when writes collide

| Option | Mechanism | Lives in | Fit |
| --- | --- | --- | --- |
| M-A (today, keep) | `append` with `expected_rev`; the loser gets a conflict, re-syncs, re-validates. `op` replay (R15) makes the retry safe. | BBP core + `Store` trait | Matches "one transactional authority". Collisions are rare because one turn is live. |
| M-B | Serialise appends through one writer task. | Host adapter | Same semantics as M-A with less retry churn. Implementation detail, not a protocol change. |
| M-C | CRDT-style conflict-free merge of card and messages. | A new store leg | Does not fit (below). |

Why M-C does not fit: the admission rules are *order-dependent invariants*. R16 allows one final report per run; R17 needs the current candidate, the selected run and a matching approve; T7 stales earlier verdicts. Two concurrent replicas can each pass validation locally and jointly violate them (two final reports for one run, two "current" candidates). A CRDT makes data converge; it does not make the rules hold.

Decision: keep M-A. Messages and events are already append-only with store-assigned monotonic ids, so collisions are ordering, not content. The only mutable thing is the card, and it is a fold, so it converges by construction. A CRDT would solve concurrent edits to shared content, which BBP does not have, and break admission, which it does.

### b2. DELIVERY: how an agent learns something changed

A shared store does not push. Some wake mechanism always exists; the question is where it lives.

| Option | Mechanism | Lives in | Cost |
| --- | --- | --- | --- |
| D-1 Poll `task_card()` | Agent or host compares `rev`. | Agent/host | Free per spec (no token, not charged). Latency = poll interval. This is how the moderator works today (`moderator.rs:346`). |
| D-2 Host tails the log and wakes | Host adapter watches `rev` (poll `Store::events(task, after)`, or fs notify), runs `wake_set`, launches or resumes the chosen agent. | **Host adapter / orch**, not BBP | No BBP API beyond `wake_set`. One watcher serves all agents. |
| D-3 BBP subscription trait | Core exposes `trait Subscriber { fn on_events(&mut self, &[Event]) }`. | BBP crate API | Stays pure only if the caller drives callbacks after `append`; that is D-2 with glue in the library. One implementer today. |
| D-4 Dirty flag per readable section | Store keeps `(role, kind) -> last_changed_rev`. | Store | Mutable store state beside an append-only log; derivable from events, so a cache, not truth. |
| D-5 Store-native push | Store feeds changes to clients. | Store | `rusty_multimodal_db` has a change log a standby *tails* (pull replication), not a client push API. See option 3. |

Decision: D-2, with D-1 as fallback and test double. `wake_set` is the only new BBP surface. Delivery is the host's job because the host already owns process lifecycle (spawn, `kill_harness`, `AbortTurn` on harness exit; `moderator.rs:176-220, 374-415`), and the BBP/orch boundary (decided 2026-10-11) puts factories and rosters in orch.

D-3 is deferred. Named trigger to revisit: a second delivery adapter that D-2's tailing cannot express (for example in-process orch delivery). One implementer does not justify a trait.

## c. Scheduling: keep turn-based as one policy

The seam already exists in practice. `schedule()` (`engine.rs:1374`) is "the one scheduling check" and chooses the grantee with `Roster::default_holder(state)` (`role.rs:335`). `grant()` (`engine.rs:1359`) is policy-free.

```
schedule(w, now)               // guards unchanged: no live turn, no pending request, opened, test-run ready
  -> pick_next(w.st)           // the seam: a set of roles, capped by policy (1 for TurnBased and Phase 1)
  -> grant(w, role, Default, None, now)   // unchanged
```

| Policy | `pick_next` | Status |
| --- | --- | --- |
| `TurnBased` | `Roster::default_holder(state)` | Today's behavior. All current tests exercise this. |
| `EventDriven` | The first role of `wake_set`; if the set is empty, `default_holder` (liveness fallback) | New. |

The fallback matters: a default holder who passes is regranted at once today, but the watermark stops the state trigger firing again for a role that has already ended its turn, so without the fallback `wake_set` is empty and the task stalls. With it, an idle task behaves like turn-taking, and `turns` still bounds a loop.

Is the seam justified? Yes, narrowly. The two policies differ in exactly one decision (who is picked), so: **no trait**. A `SchedulingPolicy::{TurnBased, EventDriven}` enum is recorded in `task_opened` as an additive field with `#[rusty_serde(default)]` (default `TurnBased`): old logs and tests are unchanged and no schema bump is needed. Recording it in the log keeps replay deterministic.

Test impact: none for `TurnBased` (the default). The seam must NOT change who is *allowed* to write; authorization stays in R5/R14 validation.

**Shadow mode.** Captured in `schedule()`, where `pick_next` runs, not in `grant()`: `grant()` also serves explicit grants (yield, human `ask`, settlement, consultation return) whose grantee is forced, so a `wake_set` comparison there would flag disagreements that are by design. `schedule()` computes what `EventDriven` would have picked and returns it in `Handled.shadow` beside the real grant: not logged. Explicit grants are labelled `Explicit` in `Handled.shadow` and carry the forced grantee as their counterfactual, so they are counted separately and never as disagreements. The host logs the decisions, deduped on `rev`, so real runs give evidence and not only the traces. The comparison (agreements, disagreements and why) is shown to the owner before live mode is enabled.

Planned comparison tests beyond the 31 traces: a human `ask` at a gate (explicit grant, labelled, forced grantee preserved); a yield and its return (both explicit; no `pick_next` comparison in between); a `Tick` at the same `rev` (one shadow record after host dedupe).

## d. Human gates, budgets, evidence, and runaway ping-pong

### d1. What "only one turn is live" provides today

| Invariant | Provided by | Under `EventDriven`, one live turn (this design) | Under concurrent live turns (not chosen) |
| --- | --- | --- | --- |
| A write must come from the live turn | R14, `check_token` (`engine.rs:238-258`): role, turn id, deadline | Unchanged | `st.turn` is an `Option<Turn>`; becomes a set, token lookup per role, every stale-turn trace re-derived |
| Stale-turn rejection after the turn ends | `turn_ended` kills the token | Unchanged | A late write from turn A could land during turn B; needs per-turn fences |
| Verdict binds to the selected run | R3, R6, R17 | Unchanged | R17's "an approve exists" check races a concurrent `revise` |
| Deadline per turn | `Turn.deadline`, `fire_deadlines` (`engine.rs:1468`) | Unchanged | One deadline per live turn; recovery must revoke each |
| `reads_per_turn` | R11, card `reads_this_turn` | Unchanged | Per-role counters |
| Recovery revokes the live turn | spec "Recovery" | Unchanged | Revoke N turns |

Decision: **max concurrent live turns = 1.**

### d2. Anti-ping-pong controls

Two eager agents answering each other is a loop in which every message is individually legal (each `ask`/`finding` passes R2 to R10). Controls, each hooked to an existing rule:

| Control | What it does | Hooks into | New state? |
| --- | --- | --- | --- |
| Charge on wake | Every wake is a `turn_granted`, which already charges `turns`. A wake that finds nothing to say still costs one turn when the agent `pass`es. | R11 `turns` | No |
| Turn budget as hard stop | `turns` crossing its limit emits `escalated` (T3, phase 4). Ping-pong cannot outrun it. | R11, T3 | No |
| Self-wake exclusion | Events authored by role X never wake X. Under single-turn operation cooldown already implies it, because an author's events precede the end of its own turn. | trigger rules (a) | No |
| Per-agent cooldown | A role that ended a turn is ineligible until `rev` advances by something it can read and did not author. | `seen_rev` watermark (`TaskState.last_turn_end`) | Fold-derived, not logged |
| Wake only on content | `pass` has no refs or evidence (R5 table) and wakes nobody. | R5 | No |
| Debounce | Eligibility is evaluated once per transaction in phase 4, after all effects. Several events in one transaction yield one wake. | processing model phase 4 | No |
| `reads_per_turn` | A woken agent that only reads is bounded. | R11 | No |
| Message and byte budgets | Unchanged. | R11 | No |
| Reply-chain depth cap | **Deferred until shadow-mode evidence** (amendment 4). Candidate new budget field `max_chain`: consecutive agent-to-agent `ask`/`answer` on one `reply_to` chain with no human message and no state change; beyond N, escalate or force `request_decision`. | New R11 field, evaluated like the others in phase 4 | One counter, derivable from messages |
| Wall-clock rate limit | Minimum interval between two default grants to one role. | Host (the `Tick` is already time-driven) | Host only |

The first rows do most of the work for free: `turns` was designed as the loop bound (spec Risks: "`turns` budget bounds the loop"). The reply-chain cap is a candidate, not a requirement; measure first (spec Q1: never raise a limit to make a task pass).

### d3. Human gates

- No change. Gate states have no default turn, and `wake_set` is empty there (a human `ask` is an explicit grant). R13 (no Agent API creates a human event) and R17 are untouched, so event-driven mode cannot open or approve a gate.
- A wake cannot bypass the scheduling check: `schedule()` returns early on `pending_request.is_some()` and `pick_next` runs after those guards, so an open request still holds the task.
- A human `ask` stays a consultation grant and outranks any `EventDriven` pick.

### d4. Evidence rules

R3 is state, not scheduling. An eager Tester cannot jump in before the report exists: `schedule()` already refuses a `test` grant until `selected_run().status` is set, and `wake_set` reproduces that guard. A role woken by an `Artifact` trigger on a `diff` during `build` may post `finding`, not `verdict` (R5, R6 state checks). No change.

## e. Options

### Option 1: in-BBP notify

Variants: (1a) `wake_set` pure function plus host delivery; (1b) a `Subscriber` trait inside `rusty_bbp`.

| | 1a | 1b |
| --- | --- | --- |
| Pros | No new I/O in the core; testable as a pure function over `TaskState`; reuses `Store::events`; delivery replaceable. | Library-level callback; tidy for in-process embedders. |
| Cons | The host implements the tail loop (it already runs a comparable loop, `moderator.rs:255-346`). | One implementer today; callbacks invite state mutation mid-transaction; blurs "pure library, no I/O". |
| Wake-up lives in | Host / orch | BBP API surface; host still has to drive it |
| Verdict | **Recommended** | Defer |

### Option 2: remind-me as shared state plus an external trigger

Remind-me is the user's MCP memory/reminder server. Verified in `crates/apps/rusty_remind_me`:

- Saved-search "watch" is **polling**: `poll_saved_search` returns matches not seen before (`remind_me_core/src/saved_searches.rs:156`, module header).
- The folder watcher is **polling by design**, with a `(mtime, size)` debounce (`remind_me_core/src/watcher.rs:1-25`).
- A webhook exists (`remote.rs:15`, `importer.rs:1503`, `watcher.rs:316` reference it). The references I read describe it as **inbound** (a push into remind-me that ingests memories). I did not read the webhook source.
- **Assumption, unverified:** that remind-me can call out to an agent when a saved search matches.

Sketch: the host mirrors board changes into remind-me memories; an agent registers a saved search; remind-me's poll finds a hit; something external wakes the agent.

| | |
| --- | --- |
| Pros | Reuses an existing tool agents can already call over MCP. |
| Cons | Two sources of truth for board state. Delivery is still polling (verified), so it adds a hop and no push. A woken agent reading mirrored content bypasses BBP's visibility matrix and read budgets (reads are charged only via BBP's `read`). Memory becomes a participant in a protocol whose rule is "enforce in the store". |
| Wake-up lives in | Remind-me's poller, outside BBP and outside the store |
| Verdict | Reject as the agent wake path. Fine as a human-facing reminder channel (orch: "ping me when the plan gate opens"). |

### Option 3: `rusty_multimodal_db` as the board store, CRDT merge as a new access mode

Verified in source:

- `GenericProductionStore<S>` holds `inner: RwLock<S>` (`crates/libs/storage/rusty_multimodal_db_engine/src/generic/production.rs:163`, built at `:175`), and its module doc says "concurrent reader/writer access via one global `RwLock`" (`production.rs:2-3`). The server layer says concurrency "collapses onto whatever `RwLock` the wrapped store already manages internally" (`crates/apps/rusty_multimodal_db/src/server/mod.rs:19-22`), and one process per data directory is enforced by an advisory file lock (`server/data_lock.rs:1-12`).
- **No CRDT exists there.** A case-insensitive search for `crdt` across `*.rs`, `*.md`, `*.toml` under `crates/apps/rusty_multimodal_db` and `crates/libs/storage` returned no files. CRDT merge would be a new access mode and a new storage leg, not a flag.
- It has a change log that a standby tails (`server/changelog.rs:1-12`) with epoch/resync semantics. That is pull-based replication, not a client subscription. The `notify_all` at `changelog.rs:385` wakes fsync waiters inside the server, not clients.
- Not verified: whether its transactions give `Store::append(expected_rev)`-equivalent semantics. I saw the "Atomic transactions" reference in the `server/mod.rs` header and did not read it through.

| | |
| --- | --- |
| Pros | Durable store with a network server and auth already (ADR-0012, ADR-0014 per the module header). The change log could back a delivery feed. |
| Cons | The global lock means no concurrency gain over today's single writer. CRDT merge conflicts with BBP's order-dependent admission rules (b1). A new leg needs a CRDT type, tests, and a story for R16/R17. Adds a heavy dependency edge from a protocol crate to a storage app. |
| Wake-up lives in | The store's change log at best, plus a host consumer; the DB does not push to clients today |
| Verdict | Not now. If BBP ever needs a networked multi-host store, implement the existing `Store` trait over it with `expected_rev` as a transaction precondition (single-writer semantics, no CRDT). That is a smaller, separate decision. |

### Zed DeltaDB (conceptual reference only)

CRDT merge is confirmed for DeltaDB. Its delivery mechanism is not public, so this design does not depend on it and nothing above assumes how it notifies clients. The spec's Appendix A mentions "Delta"; I did not establish that it is the same product and make no claim about it.

### Comparison

| Criterion | 1a `wake_set` + host delivery | 2 remind-me | 3 multimodal_db + CRDT |
| --- | --- | --- | --- |
| Keeps single-writer admission | Yes | Yes (BBP unchanged) | No (needs a new rules story) |
| New dependency | None | Cross-app coupling | Large |
| Keeps one-live-turn invariants | Yes | Yes | Only if also serialised |
| Wake-up location | Host / orch | Remind-me poller | Store change log + consumer |
| True push | No (host tails: poll or fs notify) | No (verified polling) | No (tail only; client push unverified) |
| Existing tests | All valid | All valid | Large rework |
| Effort | Small | Small | Large |

## Phased plan

| Phase | Scope | Gate to proceed |
| --- | --- | --- |
| 0 (prereq) | v0.5 Role refactor. **Done** (#643, #644, #645). | |
| 1a | `wake_set` as a pure function with tests, plus the four fold-derived `TaskState` fields; no scheduling change. | Merged. |
| 1b | `SchedulingPolicy` (additive, default `TurnBased`) and the `pick_next` seam, cardinality capped at one. | 1a merged. |
| 1c | Shadow mode: `Handled.shadow`, the comparison tests over the 31 traces and new cases, and host-side logging deduped on `rev`. | 1b merged. **Stop: the owner reviews the comparison.** |
| 2 | Live `EventDriven` behind the policy; the real trace test for the R17 `final_decision` revise block. | Owner approval of the shadow comparison. |
| 3 | Reply-chain cap (`max_chain`), only if measurement shows loops. | Proving-run data. |

Shadow mode is on purpose: it falsifies the trigger table before it can affect scheduling.

## Non-goals

- Simultaneous live agents (parallel authoring). Needs per-turn fences and a different recovery story. Revisit only if a measured task is blocked by serial turns (spec Risks: "Serial turns slow work. Accepted.").
- Push latency better than the host's tail interval.
- Wake-time prompts telling an agent what to do given a `WakeReason` (orch).

## Follow-ups for orch (not touched here)

- Run the delivery loop: tail the log, call `wake_set`, start or resume the agent for the chosen role (`moderator.rs` is the template).
- Own the roster fields that feed triggers (`read_artifacts`, `consultable`, ...) and any human-facing notification (e.g. remind-me pings at gates).
- Wake-time prompts and per-agent launch policy (wall-clock cooldown, fresh-session rule).
- Pick the default `SchedulingPolicy` per task template.

## What I could not verify

- Whether remind-me's webhook can emit outbound to an agent (only inbound-ingestion references were read).
- `rusty_multimodal_db` transaction semantics against `expected_rev`; whether any consumer needs a client subscription.
- DeltaDB's delivery mechanism (not public), and whether spec Appendix A's "Delta" is Zed's DeltaDB.
- Behavior of any `EventDriven` run: no code exists and the trigger table is untested against the traces. Phase 1 exists for this.
- The cited line numbers are as of `main` at `e2605657` and will drift; the symbol names are the stable references.

## Open questions for the owner

Resolved on 2026-10-13: the policy lives in `task_opened` (question 2); `max_chain` is deferred (question 3); the Role refactor merged first (question 5).

1. Is "interrupt the schedule" (one live turn, event-chosen) enough, or is true concurrent authoring a requirement? Superseded by `docs/bbp-concurrency-design.md`, which treats concurrent authoring as a separate design.
2. Who owns the log-tail delivery loop: `rusty_bbp_host` (next to the engine) or orch (per the BBP/orch boundary)? Host-side shadow logging is in scope for phase 1c; the production delivery loop is not.
3. ~~What baseline should `seen_rev` have for a role with no turn yet?~~ Decided: its assignment rev, floored at the state's entry rev for artifacts. See "Decisions after the shadow comparison".

## Decisions after the shadow comparison

The first comparison over the 31 traces (53 automatic grants) was 26 agree, 2 fallback, 25 disagree. In 24 of the 25 the granted role was eligible but outranked, almost always by the lowest-`turn_order` role, which had seen nothing yet and so counted every earlier artifact and broadcast as new. The owner decided:

1. **Baseline.** A role that has never had a turn has `seen_rev` = its assignment rev, not 0. `Artifact` and `Observer` wakes are also floored at the current state's entry rev: only an artifact stored in this state, after the role's last turn end (or assignment), counts. `Addressed` uses the raw end of the last turn (0 if none), so a message sent before a role was assigned still addresses it. `State` needs no floor, because `ready_rev` is never before the state's entry; being assigned makes the state's turn available to its server, so a server assigned after the state began is still woken.
2. **Priority.** `Addressed` > `State` > `Artifact` > `Observer`, ties by `turn_order`.
3. **Observers.** An `Artifact` wake of a role that does not serve the current state is recorded as `Observer`. It is in the set, as evidence, but is not selectable (`WakeReason::selectable`) unless `max_live_turns` is above one. If no selectable entry exists, the rule falls back to the default holder.

Result on the same 31 traces: 39 agree, 3 fallback, 11 disagree. All 11 have one cause: a message addressed to `all` (the draft default, which includes every `verdict`) addresses every role that can act, so the planner and coder outrank the reviewer at `review`. With `all` not addressing anyone for scheduling, the same traces give 50 agree, 3 fallback, 0 disagree. Whether `all` should address is the next open question (4) and is not decided here.
