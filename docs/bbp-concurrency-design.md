# BBP concurrency: parallel posting and co-edited artifacts (design, no implementation)

Step 6, design only. Waits for owner approval. Builds on `docs/bbp-role-audit.md` (the nine Role fields), `crates/libs/protocol/rusty_bbp/docs/spec/bbp-v0.5-implementation-spec.md` (branch `claude/stoic-pasteur-gg75fs`, `37542c4c`), and the event-driven note (branch `claude/bbp-event-driven-design`, `fc972cbe`, accepted as Phase 1 only).

**Line references** are to `rusty_bbp/src` at `37542c4c` (v0.4.1 engine; the v0.5 engine does not exist yet). Claims I did not verify in source are labelled **[unverified]**.

## Recommendation

1. **Split the protocol into two planes.** The control plane (state transitions, human gates, budget reservation, `final_decision` gate) stays serialized by the single log's revision check. The content plane (messages, immutable artifacts, co-edited drafts) admits concurrent writers.
2. **Sequence it in two steps.** 2a: several live turns, at most one per principal, with append-only content only. 2b: co-edited `spec` as op-based CRDT ops appended to the existing event log, bound to verdicts by snapshot hash.
3. **No new Role field.** The nine fields already say who may write what; concurrency limits are task configuration, like `turn_ms`.
4. **CRDT only where merge is needed.** Appends commute and need no CRDT. A minimal in-tree line-granular sequence type (ported from the first-party `nexus-crdt` algorithm, no new dependency) covers 2b.
5. **Budgets: reserve at grant.** A turn receives a slice of the task totals when it is granted. Writes inside a turn spend the slice; concurrent turns cannot jointly overshoot.

## What changes in the premise

The event-driven note rejected CRDT merge because exactly one turn is live (`grant` returns early when `w.st.turn.is_some()`, `engine.rs:1320-1322`). That rejection was conditional, not a decision. This note drops the premise: it allows several live turns, so the merge question is open again, but only for the one case where two writers touch the same mutable thing. Everything else in the event-driven note (the `wake_set` function, shadow mode, the `pick_next` seam) stays valid, with one change: `pick_next` returns a set of roles, not one (see c).

## a. Content plane versus control plane

| Plane | What | Ordering | Concurrency |
| --- | --- | --- | --- |
| Control | State transitions T1 to T17; `request_decision` and gate opening; `verdict` (it drives T9 to T11); `human_*` events; `candidate` submission; turn grant/end; budget reservation and escalation; the `final_decision` check at the merge gate (R17) | Total order by log revision. Every control command is re-validated against fresh state after a revision conflict | None. Serialized |
| Content | `ask`, `answer`, `propose`, `finding`, `pass` messages; immutable artifacts `diff`, `spec` versions; co-edited artifact ops (2b) | Message ids and ops get their position from the log at append; effect does not depend on position (appends commute) | Allowed, bounded by live turns and slices |

**Why the log is already the serialization point.** `Store::append(task, expected_rev, events)` is a compare-and-swap on the log length, and the driver reloads and recomputes on `StoreError::Conflict` (`store.rs:15-19`, `:60-70`, `:187`). The engine is a pure function of folded state (`handle(st, cmd, now)`, `engine.rs:45`), so a retry re-runs every rule against the winner's state. That is optimistic concurrency, and it already exists. Concurrent posting therefore adds no new serialization mechanism; it adds more conflicts and more retries.

**Rule for linearization.** A control command invalidates content in flight by changing state. The existing `transition` ends the live turn with cause `revoked` (`engine.rs:1375-1380`); with several turns it revokes all of them. A content write from a revoked turn is then rejected `stale_turn` at its retry, exactly as a late write after a revoked turn is today (TR-04).

## b. Classification per kind

| Kind | Class | Merge rule | Notes |
| --- | --- | --- | --- |
| `ask`, `answer`, `propose`, `finding` | append-only | None needed. Ids are assigned by the store at append, so two concurrent posts get distinct ids. Optimistic append with `expected_rev` retry | Causality is carried by `reply_to` and `refs`, which R4/R10 already validate |
| `pass` | append-only, **with a control effect** | Ends the issuing turn only | Ending one turn must not end others (c) |
| `verdict` | **control** | Serialized; binds to a snapshot (d) | Drives T9 to T11 |
| `request_decision` | **control** | Serialized; ends the requester's turn; plan gate T4 | A pending request blocks default grants (scheduling check) |
| `decision` | control (human) | Serialized | Unchanged |
| `diff` | immutable, append-only | None. Several writers store several diffs concurrently; the candidate manifest lists them | This is the natural parallel-work unit for 2a |
| `candidate` | immutable, control (T7) | Submission is the join point: one writer, serialized | Stamps the approved spec |
| `test_report`, `log` | immutable, runner only | One per run (R16) | Unchanged |
| `brief` | immutable, human | Unchanged | |
| `spec` | **co-edited** (2b) | Op-based CRDT (f) | Until 2b it stays immutable-per-version |

**No CRDT for plain appends.** The merge of two appended messages is "both, in log order"; a CRDT would add machinery for a problem that does not exist.

**Per-writer sequence.** The request names it for the append-only class. `op` is already the idempotency key, scoped to (principal, task) (R15), and the log gives every writer a total order. A monotone per-writer `wseq` would only add gap detection for a client that needs its intents applied in order. **Recommendation: do not add it now.** The need is **[unverified]**; add it when a trace shows a reordered or lost intent. It would be a field on the command, not on Role.

## c. Several live turns: R14, stale-turn, deadlines, `turns` budget

**Proposed rule: many live turns, at most one per principal.**

| Item | Today (one live turn) | With several live turns |
| --- | --- | --- |
| State | `st.turn: Option<Turn>` (`state.rs:211`) | `turns: map principal to Turn`. Cap `max_live_turns` in `task_opened` (default 1) |
| R14 token | `token_for(principal, turn_id)`; `check_token` compares role and token against the single live turn (`engine.rs:224-243`, `state.rs:332`) | Unchanged per principal: the token is already derived from (master, principal, turn). A principal has one live turn, so its lookup is unambiguous. The "late write after the same role's next grant" test (TR-04) keeps its meaning |
| Stale-turn | A write with a non-live token gets `stale_turn` | Same, per principal. Plus: a transition revokes every live turn, so any content write from an old state is `stale_turn` |
| Deadlines | One deadline, checked in `check_token` (`now >= turn.deadline`) | One per turn; each expires independently with `turn_ended(deadline)`. A tick closes every expired turn |
| Return-to / consultation | `end_turn` regrants the yielder if state, candidate and run are unchanged (`engine.rs:1355-1372`) | The check is per turn and unchanged. A yield still ends the yielder's turn and starts one consultation turn for the target |
| `reads_this_turn` | One counter in `spent` (`state.rs:121`) | Per turn, in the `Turn` record. `reads_per_turn` is unchanged |
| R11 `turns` budget | Each `turn_granted` charges `turns` | Unchanged: N concurrent turns cost N. This is the main ping-pong brake and it survives as is |
| "Only one turn is live" | Spec invariant in Turns | Replaced by "at most one live turn per principal" and "at most `max_live_turns` live turns" |

**Why one per principal.** It keeps the host requirement "one Agent API process per role per task" and the token derivation unchanged, so the whole R14 test set carries over. Two live turns for one principal would force the token to carry a turn id and would reopen the zombie-write problem that R14 closes.

**Which roles may hold turns together** is scheduling policy, not protocol structure. The event-driven note's `pick_next` seam is where it lives, and it has to return a set of roles instead of one. That is the one amendment this note makes to that design. With `max_live_turns = 1` the seam degrades to today's behavior and the 31 trace tests are unaffected.

## d. Conflict-free is not coherent: the binding rule

A CRDT guarantees replicas converge. It does not guarantee the converged document is what a reviewer approved. So a verdict must say which exact content it judged, and any later change must void it.

**Definitions.**

- **Snapshot.** For a co-edited artifact A at log revision R, `snapshot(A, R)` is the SHA-256 of A's canonical materialization (the rendered content after applying every op with revision at most R, in the CRDT's canonical order). The hash is a function of the applied op set, so any replica holding the same ops computes the same hash.
- **Binding.** A verdict, a gate request, or a human approval that today names an artifact id names `(artifact id, snapshot hash)`. For several artifacts it names the sorted list of pairs and carries their hash.
- **Immutable artifacts** (`diff`, `candidate`, `test_report`, `log`, `brief`) have a fixed snapshot equal to their blob hash. Nothing in this section changes them.

**The rule.**

1. **Record.** Every binding records the snapshot hash of each bound co-edited artifact at the revision the binding command was validated against.
2. **Check at use.** A binding is valid only while, for every bound artifact, the current snapshot hash equals the recorded one. The check runs wherever the binding is relied on: T5 (plan approval), T7 (candidate stamps the approved spec), T9 and T11 (verdict), R17 (merge gate), and the resume guards. A mismatch is `stale_subject`, the existing code.
3. **Later edits invalidate.** Any op applied to a bound co-edited artifact after the binding changes its hash, so the binding becomes invalid without any event being written. This mirrors the v0.4 rule that a new candidate stales verdicts on the old one, extended from "new artifact" to "changed artifact".
4. **Atomic at append.** The binding command is validated and appended under the same `expected_rev` check. An op that lands between the agent's read and its verdict forces a retry, and the retry sees the new hash and rejects. A verdict can never be appended against a hash that was already stale.
5. **Re-judgment.** After an invalidating edit, the verdict must be reissued. The stage's default holder is regranted by the usual entry action. No automatic carry-over of approvals.
6. **Gate stability option.** Opening the plan gate may *seal* the spec (reject further ops, `sealed`) instead of voiding on edit. Sealing is a task option, off by default. It trades flexibility for a calmer review. I recommend leaving the choice to the owner (open question 2).

**What this does not do.** It does not decide whether a merged document is *good*, only that the judged bytes are the bytes now in place. Semantic coherence of concurrent edits stays with the agents and the evidence rules (R3).

## e. Budgets: reserve before spend

**Problem.** R11 charges after the fact and accepts the write that crosses a limit (spec, Processing model, phase 4). With one writer the overshoot is one write. With N concurrent writers it is up to N writes per field, and read-modify-check cannot prevent it because each writer validated against the same prior state.

**Rule.** The control plane reserves before it grants.

1. **Reserve at grant.** `turn_granted` carries a slice `{messages, bytes, reads}`. The grant is refused if `task_total - spent - reserved < slice` for any field, so reservations never exceed totals. Refusal records no turn and falls into the existing escalation path when nothing can ever fit.
2. **Spend inside the slice.** A write inside a turn is checked against the turn's remaining slice only, not the shared total. Concurrent turns cannot jointly overshoot because the sum of slices already fits.
3. **Release at turn end.** `turn_ended` returns the unspent part. `charged` stays the record of actual spend.
4. **Unchanged.** `turns` and `iterations` are control-plane counters charged serially; the crossing-write-is-accepted rule and its one escalation per field stay for them.
5. **Slices are task configuration**, next to `turn_ms` and `reads_per_turn`: `messages_per_turn`, `bytes_per_turn`. Not Role fields.

**Cost.** Two new event kinds (`reserved`/`released`, or fields on `turn_granted`/`turn_ended`), and a slice that is too small ends a turn early with `budget`. At `max_live_turns = 1` the slice can equal the remaining total and behavior matches today.

## f. Storage and the CRDT type

**Op-based CRDT with ops in the existing event log, or a separate store?**

| | Ops in the event log | Separate store |
| --- | --- | --- |
| Atomicity with control events | One append, one revision, one transaction. A verdict and the ops it binds to are totally ordered | Two durable systems; a verdict can reference a hash the log never saw. Needs a commit protocol |
| Snapshot hash by revision | `snapshot(A, R)` is a fold over the log prefix. Trivial | Must be reconstructed or versioned separately |
| Replay and audit | Already guaranteed (card equals fold of events) | Must be added |
| Cost | Log grows with every op; a long edit session is many events | Op storage is cheaper and compactable |
| Coupling | None beyond the existing `Store` trait | New port or a new backend |

**Recommendation: ops in the event log.** Mitigate growth with a *snapshot event* (a content-addressed blob of the materialized text plus the op cursor) once measurements show it matters; do not build it speculatively. The growth claim is **[unverified]**: nothing here has been measured.

**Which CRDT.**

| Option | Dependencies | Verdict |
| --- | --- | --- |
| External Rust CRDT crates (the well-known ones are Automerge, Yjs ports such as `yrs`, Loro, `diamond-types`; descriptions from memory, **[unverified]** in this session) | New third-party dependency, large API surface | **Rejected.** `rusty_bbp` is Tier S, first-party only (ADR-0002; `Cargo.toml` comment; `docs/review/sovereignty.md`). Nothing here justifies leaving that tier |
| Depend on the first-party `nexus-crdt` | Pulls `serde`, `uuid`, `thiserror`, `tokio`, `tracing` and `nexus-editor`/`nexus-kernel`/`nexus-types` (`crates/apps/nexus/crates/nexus-crdt/Cargo.toml`) | **Rejected.** App-layer crate, external dependencies, wrong direction (a protocol library cannot depend on an app) |
| Minimal in-tree type in `rusty_bbp`, algorithm ported from `nexus-crdt` | None. Uses `rusty_serde` | **Recommended** |

**What the in-tree type is.** A sequence CRDT (RGA) whose elements are **lines**, not characters, with op ids `(site, lamport)`, a version vector for idempotent apply, tombstones for deletes. `nexus-crdt` has the proven shape: `RgaText` with `build_insert`/`build_delete`/`apply` (`text.rs`, 486 lines), `OpId`/`Lamport`/`SiteId` (`id.rs`, 159 lines, built on `Uuid`), `OpLog` (`log.rs`). Port target: about 250 lines with a site id taken from the principal id, no `Uuid`. Line granularity is a proposal: Markdown specs are edited mostly by whole lines, and it keeps ids and tombstones an order of magnitude smaller than per-character. Whether it is sufficient is **[unverified]**; the fallback is per-character with the same type.

**Do not couple to `rusty_multimodal_db` now.** The type lives behind the existing `Store` trait (ops are `Event`s), so a CRDT-aware backend can come later without a change here. The multimodal store's global lock and absence of CRDT code are noted in the event-driven design; neither affects this plan.

**Rejected alternatives for 2b**, for completeness: (1) section-level leases (one writer per section, serialized) is simpler and may be enough, but it is not "concurrent co-editing of the same artifact"; (2) last-writer-wins per section is conflict-free but silently drops one writer's change, which is the wrong failure for a spec.

## g. Sequencing, and Role fields

| Phase | Scope | Needs | Does not need |
| --- | --- | --- | --- |
| 2a | `max_live_turns > 1`, one turn per principal; append-only content (messages, `diff`); control-plane revocation of all turns on transition; reserve-before-spend slices | Turn map in state; per-turn `reads_this_turn`; slice fields in `task_opened`; `pick_next` returning a set (from the event-driven note) | CRDT, snapshot hashes, any new Role field |
| 2b | Co-edited `spec`: op events in the log; in-tree line RGA; snapshot-hash binding rule (d) | 2a; one new artifact payload for ops; verdict/gate/approval records carry hashes | A new store, a new dependency |

2a ships value alone: several agents post and several authors store diffs in parallel, joined by the candidate.

**Does `Role` need a new field? No.** Recommendation: none.

- Who may submit spec ops is `write` ∋ `spec` (the same capability that today lets a role store a spec).
- Who may read a co-edited artifact is `read_artifacts`.
- How many may be live at once is task configuration (`max_live_turns`), because it is a property of the task's scheduling policy, not of any one role.
- Whether a role is *eligible to be woken together with another* is derived from the existing fields by `wake_set`, per the event-driven note.

One field I considered and rejected: `concurrent: bool` on Role. It would let a roster mark roles that may hold a turn alongside others, but every use I can construct is expressible as a policy over `post`, `write` and `turn_order`. Add it only if a real roster cannot be expressed without it.

## h. Options, trade-offs, recommendation

| Option | Gives | Costs | Verdict |
| --- | --- | --- | --- |
| 0. Stay single-turn | Nothing new; all current guarantees | No parallelism | Baseline, kept as the default policy |
| 1. Two planes, 2a only | Parallel posting and diffs; no new data structure; budget safety | Turn map; slices; set-valued scheduling | **Do first** |
| 2. 1 + 2b in-tree line RGA in the event log | Co-editing; a single audit trail; deterministic binding | About 250 lines of CRDT to own and test; log growth | **Recommended second step**, gated on evidence from 2a |
| 3. 1 + external CRDT crate | Less code to write | Breaks Tier S; large surface | Rejected |
| 4. 1 + section leases instead of CRDT | Simple | Not co-editing of one artifact | Fallback if 2b's cost is not justified |
| 5. CRDT in `rusty_multimodal_db` | Reuse of a store | Couples BBP to it; new storage leg | Deferred, per the brief |

**Recommendation: Option 1 now, Option 2 after 2a evidence.** The decisive reasons: the log's compare-and-swap already serializes the control plane, so only the turn model and budgets need to change for parallel posting; and putting ops into that same log is what makes "the verdict judged these bytes" checkable by a hash comparison.

## Tests this design implies (not written)

- `max_live_turns = 1` runs the existing 31 traces unchanged.
- Two principals post concurrently: both accepted, distinct ids, order follows the log; a conflict retry produces the same result.
- A transition revokes all turns; the late writer gets `stale_turn`.
- Reservation: sum of slices never exceeds totals; an oversized request is refused; unspent is released.
- 2b property tests: ops commute (any order of delivery converges); idempotent apply; a verdict bound to hash H is rejected once any op changes the hash; a retry after a revision conflict rejects a verdict on a stale hash.

## Open questions for the owner (max 5)

1. **Phase gate.** Approve 2a as the next implementation step, and is 2b conditional on evidence from it?
2. **Sealing.** Should the plan gate seal the spec by default (calmer review) or void on edit (more flexible)?
3. **Granularity.** Line-granular RGA to start, with per-character as the fallback?
4. **`max_live_turns` default and cap.** Default 1 is assumed; what upper bound is acceptable for a first release?
5. **Scheduling.** The event-driven `pick_next` returns a set here. Agree that this amends that note's single-turn premise?

## Follow-ups for orch (not touched here)

- Choose `max_live_turns`, slice sizes and the sealing option per factory, and pass them with the roster at `task_opened`.
- Decide which roles are worth running concurrently; the protocol only enforces the cap.
- If a host wants character-level co-editing UIs, that adapter lives above BBP.

## What I could not verify

- Contention behavior of `expected_rev` retry with many writers (no benchmark; the A3 retry path exists but I did not measure it).
- Whether `fs_store` is safe for several writer processes beyond what `refresh` provides.
- The three external crates' properties (stated from memory) and that none fits Tier S.
- Whether line granularity is enough for real specs.
- That `nexus-crdt`'s RGA ports cleanly without `Uuid`: I read its module list and dependencies, not the algorithm in full.
- Log growth under real edit sessions.
