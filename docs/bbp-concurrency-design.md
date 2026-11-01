# BBP concurrency: parallel posting and co-edited artifacts (design; implemented, see Status 2026-10-26)

> **2026-10-26:** the `rusty_crdt` parts of this note (§f, Appendix A, Appendix B) are revisited in `docs/rusty-crdt-extraction-note.md`. Since #1052 a private RGA-ordered fold ships in `rusty_bbp` (`coedit.rs`), and Appendix A's id contract misses a freshness check.

Step 6, design only. Waits for owner approval. Builds on `docs/bbp-role-audit.md` (the nine Role fields), `crates/libs/protocol/rusty_bbp/docs/spec/bbp-v0.5-implementation-spec.md` (branch `claude/stoic-pasteur-gg75fs`, `37542c4c`), and the event-driven note (branch `claude/bbp-event-driven-design`, `fc972cbe`, accepted as Phase 1 only).

## Decisions (owner, 2026-10-13)

Direction accepted. Recorded decisions, each applied below:

1. **Serde: option A.** `rusty_crdt` gets a `rusty_serde` feature (first-party), following `rusty_uuid`. `rusty_bbp` does not enable it.
2. **Location:** `crates/foundation/rusty_crdt`. The private RGA inside `rusty_bbp` stays a documented fallback only.
3. **`Pending { missing }` is a protocol error in BBP.** It is rejected `ref_unresolved`, never buffered.
4. **`pick_next` returns a set**, with its cardinality capped by the scheduling policy. `TurnBased` and event-driven Phase 1 cap it at one.
5. **No `messages_per_turn` or `bytes_per_turn` in v0.5.** Phase 2a derives slices from `max_live_turns` (section e, with the justification).
6. **No per-writer sequence** (`wseq`).

**Amended 2026-10-13 (storage):** the CRDT is no longer a private in-tree type. It is a shared first-party crate `rusty_crdt`, seeded from the `nexus-crdt` algorithm (read in full), with a sync zero-dependency core. See f, Appendix A (API sketch) and Appendix B (ADR draft).

**Status (2026-10-13):** Phase 2a is implemented in `rusty_bbp` and the host (several live turns, one per role, `max_live_turns` in `task_opened`, slices reserved at grant, per-turn abort, one harness per live turn, `bbp open --live-turns N`), with `max_live_turns = 1` as the default and the behavior of v0.5. Differences from the text below: the log schema is 3 when `max_live_turns > 1` (2 otherwise, byte-identical to before); the slice is released implicitly (reserved space is derived from live turns, so no `turn_ended` payload was added); slices are enforced only above one live turn so that `max_live_turns = 1` keeps the accepted crossing write of R11 (the overshoot invariant is mode-dependent); finalizers are serialized and `turns` is checked in the grant that charges it. Known gap: a verdict is not bound to the revision the judge read (spec, Known gaps). Finalizer serialization is not an all-finalizers verdict barrier. The host steps and metrics tooling were integrated into the Phase 2a branch (#657), not `main`. The instrumentation tests show the measurements work; they do not establish a Phase 2b benefit and are not owner adoption approval. Phase 2b stays gated on evidence from 2a and on the owner's approval of the gate criteria.

**Status (2026-10-26): 2b delivered by BBP step 4, without `rusty_crdt`.** The co-edited `spec` landed through `docs/bbp-event-based-plan.md` step 4, not through this note's crate plan. Against sections b, d, f and Appendix A:

- **Co-edited artifact.** A spec is co-edited when its first version is a `SpecEdit` with no base; snapshot specs are unchanged (#1052). There is no task flag.
- **Ops in the log (f).** Each edit is one `artifact_stored` whose payload carries line ops with stable `LineId { art, n }` ids and tombstones. Ids come from the log, not from `(lamport, site)`. A proptest checks that the fold equals RGA order. The fold is in `rusty_bbp::coedit` and does not use `rusty_crdt`. The owner approved building that crate separately on 2026-10-26 (#1113, #1122). Moving `rusty_bbp` onto it is a later, separate step.
- **Admission differs on purpose.** An edit anchored on a line edited since its base is refused `stale_base`, not merged (#1067, the owner's rule 2 of 2026-10-25).
- **`Pending { missing }` (decision 3).** An anchor not visible in the base is refused `ref_unresolved`. A logged version that does not apply on replay is recorded in `TaskState::unapplied`, and edits or gates on it are refused by name.
- **Binding (d).** Specs are writable only in planning, so opening the plan gate seals the spec (the d.6 option, de facto). The gate binds a co-edited version only when it is the head of its spec, otherwise `stale_subject`. Each version carries `snapshot`, the SHA-256 of its text, and the human view shows the proposal's snapshot. The approval itself binds by version id.
- **Agent surface.** `bbp mcp` writes edits by line id and reads versions with their lines, head and snapshot.
- **Not done.** A live run with several co-editing agents, which needs the owner's spend approval.

**Line references** are to `rusty_bbp/src` at `37542c4c` (v0.4.1 engine; the v0.5 engine does not exist yet). Claims I did not verify in source are labelled **[unverified]**.

## Recommendation

1. **Split the protocol into two planes.** The control plane (state transitions, human gates, budget reservation, `final_decision` gate) stays serialized by the single log's revision check. The content plane (messages, immutable artifacts, co-edited drafts) admits concurrent writers.
2. **Sequence it in two steps.** 2a: several live turns, at most one per principal, with append-only content only. 2b: co-edited `spec` as op-based CRDT ops appended to the existing event log, bound to verdicts by snapshot hash.
3. **No new Role field.** The nine fields already say who may write what; concurrency limits are task configuration, like `turn_ms`.
4. **CRDT only where merge is needed.** Appends commute and need no CRDT. For 2b, extract only the RGA into a new Tier S crate `rusty_crdt` (zero dependencies, sync, no I/O, generic over id and element), seeded from `nexus-crdt`. Build it at the start of 2b, not before; 2a needs none of it.
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

**Per-writer sequence.** The request names it for the append-only class. `op` is already the idempotency key, scoped to (principal, task) (R15), and the log gives every writer a total order. A monotone per-writer `wseq` would only add gap detection for a client that needs its intents applied in order. **Decided: no per-writer sequence.** `op` plus the log's total order cover idempotency and ordering. It would be a field on the command, not on Role, if a trace ever shows a reordered or lost intent.

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

**Which roles may hold turns together** is scheduling policy, not protocol structure. The event-driven note's `pick_next` seam is where it lives, and it has to return a set of roles instead of one. That is the one amendment this note makes to that design. The policy caps the set's size: `TurnBased` and the event-driven note's Phase 1 cap it at one, which is today's behavior and leaves the 31 trace tests unaffected. A cap above one exists only in 2a, through `max_live_turns`.

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

1. **Reserve at grant.** `turn_granted` carries a slice `{messages, bytes, reads}` (derived as below). The grant is refused if `task_total - spent - reserved < slice` for any field, so reservations never exceed totals. Refusal records no turn and falls into the existing escalation path when nothing can ever fit.
2. **Spend inside the slice.** A write inside a turn is checked against the turn's remaining slice only, not the shared total. Concurrent turns cannot jointly overshoot because the sum of slices already fits.
3. **Release at turn end.** `turn_ended` returns the unspent part. `charged` stays the record of actual spend.
4. **Unchanged.** `turns` and `iterations` are control-plane counters charged serially; the crossing-write-is-accepted rule and its one escalation per field stay for them.
5. **No new budget configuration in v0.5.** v0.5 is `TurnBased` with at most one live turn, so a slice would equal the whole remaining headroom and nothing changes. In 2a, slices are **derived** from the one setting `max_live_turns = N`, not configured.

**Derived slice (2a).** At a grant with `live` turns already open (`live < N` always, because the cap is checked first), for `f` in `messages` and `bytes`:

`slice_f = floor((total_f - spent_f - reserved_f) / (N - live))`

`reads` keep the existing `reads_per_turn`. The slice is written into `turn_granted`, so replay is deterministic and nothing is recomputed from configuration. Unspent slice returns at `turn_ended`. At `N = 1` the slice is the whole headroom, which is today's behavior.

**Two fields versus one: why derive.**

| | Derive from `max_live_turns` (chosen for 2a) | Add `messages_per_turn`, `bytes_per_turn` |
| --- | --- | --- |
| Knobs | One, which must exist anyway | Three |
| Hard bound | Yes: each grant takes `1/(N-live)` of the headroom left, so the sum of slices never exceeds it | Yes, but only if validated against totals at task open, an extra rule |
| Behavior at `N = 1` | Identical to today | Identical only if the fields default to the totals |
| Evidence | None needed | Needs data on what a good per-turn cap is. Spec Q1 says record why a limit was hit before changing it; there is no such record for these |
| Weakness | Even sharing: the first turn gets `1/N` of the headroom even if its peers stay idle, so one heavy writer can hit its slice early | Predictable caps, matches the `reads_per_turn` precedent |

Reasoning: two new fields are two guesses with no data behind them, and a derived slice already gives the hard bound that motivated the design. The weakness is observable: it shows up as `turn_ended(budget)` while other live turns hold unspent slice. **Trigger to add explicit fields:** 2a evidence that such early exhaustion happens on legitimate turns. Until then, no new fields. This also matches the decided rule against speculative generality.

**Cost.** Two new event payloads (the slice on `turn_granted`, the release on `turn_ended`), and a slice that is too small ends a turn early with `budget`. At `max_live_turns = 1` the slice equals the remaining headroom and behavior matches today.

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

**Which CRDT: a shared `rusty_crdt`, RGA only.**

| Option | Dependencies | Verdict |
| --- | --- | --- |
| External Rust CRDT crates (Automerge, Yjs ports such as `yrs`, Loro, `diamond-types`; from memory, **[unverified]**) | New third-party dependency, large API surface | **Rejected.** `rusty_bbp` is Tier S (ADR-0002) |
| Depend on `nexus-crdt` as is | `serde`, `uuid`, `thiserror`, `tokio`, `tracing`, `nexus-editor`, `nexus-kernel`, `nexus-types` | **Rejected.** App-layer, external dependencies, and Nexus is frozen and outside the workspace (`crates/apps/nexus/FROZEN.md`) |
| Private RGA inside `rusty_bbp` | None | Superseded by the next row. Works, but the law tests and the type would be buried in a protocol crate |
| **Shared `rusty_crdt`, RGA only, seeded from `nexus-crdt`** | None in the core | **Recommended** |

**Why a separate crate for one live consumer.** The repo's own consolidation audit says to extract `nexus-crdt` "only if a second consumer appears" (`docs/research/CONSOLIDATION-AUDIT.md:210`). BBP is that second consumer. The honest accounting: Nexus is frozen, so today there is one live consumer and one frozen one, which is thinner than the two-call-site rule normally wants. The case for the crate rests on isolation (zero-dependency Tier S core, CRDT law tests that do not live in a protocol crate), not on deduplication. If you would rather wait for a second *live* consumer, the fallback is the private RGA row, with the same API so it can be lifted later.

**Where it sits.** `crates/foundation/rusty_crdt`, layer `foundation`, beside `rusty_uuid`, `rusty_json`, `rusty_serde`. `rusty_bbp` depends on it by workspace path, which ADR-0002 allows (see Appendix B).

**What was read and what changes.** I read `text.rs` (the RGA), `id.rs`, `log.rs`, `merge.rs` and the causal-delivery path of `doc.rs` in full. The algorithm is a tree RGA: every element has a parent (the element to its left when authored, or the head); siblings sort by descending id; delete is a tombstone; rendering is an iterative pre-order walk. The extraction keeps that and changes these things:

| Finding in `nexus-crdt` | Treatment in `rusty_crdt` |
| --- | --- |
| `apply` returns `bool`; an `Insert` with an unknown parent and a `Delete` with an unknown target are silently dropped, and `CrdtDoc` pre-checks causality (`doc.rs:185`, `:233-260`) | `apply` returns `Changed`, `Duplicate` or `Pending { missing }`. The caller (BBP validation, or a nexus buffer) decides |
| `.expect("parent presence checked above")` (`text.rs:258`) | Arena indices; no `expect` or `unwrap` |
| Tombstones keep the character | Delete drops the element value, keeps the node |
| `len()` is O(n) (`text.rs:119`); position lookup is O(n) per local edit | `len()` O(1) via a counter; position lookup stays O(n) and is documented |
| `Lamport::next` is an unchecked `+ 1` (`id.rs:50`) | Not extracted: ids are the caller's. BBP allocates its own |
| `VersionVector::contains` means "max lamport seen"; it is only sound with in-order delivery per site (`id.rs:106`) | Not extracted. BBP's log already orders ops; Nexus keeps its own |
| First op with a given id wins; two different ops with one id make replicas diverge | Documented caller contract: ids unique. BBP rejects a reused id with a different payload at validation. Laws are tested over unique ids only |
| `Uuid` site ids; v5 synthetic baseline ids (`merge.rs`) | Ids are generic. Baseline-seeding stays in Nexus. In BBP, seed content is ordinary ops by the first author |

**Stays out of `rusty_crdt`:** `OpLog` and its pruning, `VersionVector`, `CrdtDoc`, conflict types, wire/sync, counters, sets and maps. None has a second call site.

**Granularity.** The element type is generic, so BBP instantiates it with a line (`String`) and Nexus with `char`. Line versus character is a BBP choice made at the instantiation, not a property of the crate.

**Do not couple to `rusty_multimodal_db` now.** Ops are `Event`s behind the existing `Store` trait, so a CRDT-aware backend can come later without a change here.

**Rejected alternatives for 2b**, for completeness: (1) section-level leases (one writer per section, serialized) is simpler and may be enough, but it is not "concurrent co-editing of the same artifact"; (2) last-writer-wins per section is conflict-free but silently drops one writer's change, which is the wrong failure for a spec.

## g. Sequencing, and Role fields

| Phase | Scope | Needs | Does not need |
| --- | --- | --- | --- |
| 2a | `max_live_turns > 1`, one turn per principal; append-only content (messages, `diff`); control-plane revocation of all turns on transition; reserve-before-spend slices | Turn map in state; per-turn `reads_this_turn`; `max_live_turns` in `task_opened`; slices derived from it and carried on `turn_granted`; `pick_next` returning a set capped by policy (from the event-driven note) | CRDT, snapshot hashes, any new Role field |
| 2b | Co-edited `spec`: op events in the log; `rusty_crdt` RGA (built first, with its law tests); snapshot-hash binding rule (d) | 2a; the ADR in Appendix B accepted; one new artifact payload for ops; verdict/gate/approval records carry hashes | A new store, an external dependency |

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
| 2. 1 + 2b shared `rusty_crdt` RGA, ops in the event log | Co-editing; a single audit trail; deterministic binding; reusable, independently law-tested core | A new crate and an ADR; log growth | **Recommended second step**, gated on evidence from 2a |
| 3. 1 + external CRDT crate | Less code to write | Breaks Tier S; large surface | Rejected |
| 3b. 1 + private RGA in `rusty_bbp` | Smallest change | Law tests buried in a protocol crate; harder to lift later | Fallback if no second live consumer is wanted |
| 4. 1 + section leases instead of CRDT | Simple | Not co-editing of one artifact | Fallback if 2b's cost is not justified |
| 5. CRDT in `rusty_multimodal_db` | Reuse of a store | Couples BBP to it; new storage leg | Deferred, per the brief |

**Recommendation: Option 1 now, Option 2 after 2a evidence.** The decisive reasons: the log's compare-and-swap already serializes the control plane, so only the turn model and budgets need to change for parallel posting; and putting ops into that same log is what makes "the verdict judged these bytes" checkable by a hash comparison.

## Tests this design implies (not written)

- `max_live_turns = 1` runs the existing 31 traces unchanged.
- Two principals post concurrently: both accepted, distinct ids, order follows the log; a conflict retry produces the same result.
- A transition revokes all turns; the late writer gets `stale_turn`.
- Reservation: sum of slices never exceeds totals; an oversized request is refused; unspent is released.
- 2b property tests: ops commute (any order of delivery converges); idempotent apply; a verdict bound to hash H is rejected once any op changes the hash; a retry after a revision conflict rejects a verdict on a stale hash.

## 2b gate criteria (approved 2026-10-20, set before any measured run)

Status: **approved by the owner on 2026-10-20, after the owner's review of the first draft (four corrections), the second review of 2026-10-15 (see the review record) and the owner's decisions on killed-builder pairs and the grader's residual limit.** The thresholds are judgment, not derived from data. They are written down first so that no run can pick its own pass mark. The owner may change any number; any change after the first measured run must be recorded as such. Approval fixes the pass marks only: nothing here authorizes building 2b, and the evidence run remains unqueued and requires separate owner authorization.

**Scope: the evidence must come from the artifact 2b co-edits.** 2b co-edits the `spec`; `diff` and `candidate` stay immutable (section b). Contention between builders' code diffs is therefore **not** evidence for 2b, and no threshold below counts it: a pass on code-diff contention would justify a CRDT for an artifact that 2b does not touch. If the owner wants co-edited code, that is a different artifact class and a separate, separately approved scope change with its own design note; it is not proposed here. (The code-diff metrics of the metrics tooling stay available as context and are not used by the gate.)

**What 2b must beat.** Option 4 (section leases) and plain 2a (parallel versions, one approved, the others unused) are the alternatives. 2b is justified only if real spec authoring shows contention that those two handle badly.

**The runs.** At least 10 tasks with real agents, paired: each task runs once at `max_live_turns = 1` (control, one spec author) and once at 2 (two roles that hold `write` ∋ `spec` and serve `planning`), on the same roster otherwise, brief, budget, model and acceptance rubric. At least 5 are *shared-section* tasks, written so that the spec must cover two intertwined concerns that two authors would naturally edit in the same sections; the rest are *separable* tasks whose concerns map to disjoint sections.

**Definitions, fixed with the task set.** The owner records, per task and before the first run, two concern lists (one per author) and the section each rubric item belongs to. A task is **separable** if every rubric item maps to exactly one concern and the two concerns own disjoint sections of the expected spec: no section is owned by both, and no rubric item needs content from both. A task is **shared-section** if at least 30% of its rubric items, and at least 2, need content in a section that both concerns own. A task that is neither (some overlap, below that line) is not used. The classification is frozen with the briefs; it is never changed after any run has been seen. The set, the briefs and the rubrics are fixed before the first run. The owner classifies the tasks and writes or reviews the briefs and rubrics; this note and its tooling do not draft them. In addition, **3 null pairs**: three of the separable tasks run a second time at `max_live_turns = 1` against their own control, to measure run-to-run noise (see criterion 3). The run is therefore 23 runs, in two phases (below): the 6 runs of phase 1, then 7 more controls and 10 treatments, because the first run of each null task is that task's phase 2 control.

**Order: the noise gate runs first.** Phase 1 is the six runs of the three null pairs (each of the three tasks twice at N=1, the first run being that task's control for phase 2). After phase 1 the null ratios (second over first `spec_wall`) are computed. **If any is outside 0.85 to 1.15, stop and report: no phase 2 run is started** and the owner decides whether to lengthen the tasks, repeat, or abandon the screen. Phase 2 is the 10 paired tasks, N=2 against the control (for the three null tasks, the phase 1 first run). Phase 1 spends 6 of the 23 runs' tokens before anything is learned about the hypothesis; that is the price of not building a threshold inside the noise.

**Comparable outcomes are frozen with the inputs.**

- **Rubric.** Each task has an acceptance rubric fixed in advance: a list of requirements the approved spec must satisfy (for example 8 to 12 items), scored by the same grader for both runs of a pair, blind to which run is which. **The rubrics, briefs, task classification and pass marks are committed to the repository as final, in their own PR, before the first run;** the run output records the hash of that committed set, and a run whose set differs from it is void. **Blindness is mechanical:** the grader receives the approved spec text and the rubric only, under a neutral id; arm, pair, turn counts and logs are withheld; the order is shuffled; the id-to-run key is sealed until every score is written, and scores are not edited after the key is opened. The grader tool (`bbp grade`) builds the packs and joins the scores; whoever scores, a person or a model, is named in the output. A run *succeeds* only if it reaches `closed` and the rubric score is at least a pre-declared pass mark, and the build passes the task's frozen profile set.
- **Failed pairs.** A pair fails if either run does not succeed: cancelled, escalated, wall limit, timeout, or below the pass mark. A failed pair is reported, excluded from every median below, and never counts as fast. At least 8 of the 10 pairs must succeed or the screen is inconclusive and no decision is taken. The N=2 failure count must not exceed the N=1 count by more than 1.
- **Gates.** The human gates are played by a scripted human with a fixed policy (approve a spec that meets the pass mark, immediately). **Every gate's answer is recorded in the run output with the principal that gave it, and a run whose gates were not answered by the declared owner is labelled `self_approved`.** A scripted gate is `self_approved` by definition, so every run in this screen is, and the label is reported with the result: it says no person reviewed any approved spec, and nothing here is evidence about the quality of human review. (Mill Board card #29 is the enforcement side: a sandbox flag in `task_opened`, the self-approval marked in the log, and a refusal of gate answers from agent principals. It is not built. The label here, from `bbp human --as` and `bbp metrics --owner` (#721), is the metrics side only and rests on a claim by whoever ran the command, not on the log. Until the card lands, the run's `self_approved` label is not enforced.) Wall clock excludes gate waits: `spec_wall` is the time from the first `planning` turn to the plan-gate request. The moderator reports only total elapsed today; per-state timing is a tooling follow-up.

**Measured, per pair** (from the log, the blobs and the rubric; nothing is added to the protocol):

| Metric | Definition |
| --- | --- |
| `spec_wall` ratio | `spec_wall` at N=2 over N=1 for the same task, successful pairs only |
| `unused_spec_versions` | `spec` artifacts stored in the task and not the approved one, whose changed lines are less than half present in the approved spec (a version whose content was carried into the approved one is *retained*, not unused) |
| `unused_share` | bytes of unused spec versions over bytes of all spec versions stored |
| `spec_conflicts` | an unused spec version and the approved one that edit the same lines of a common base, measured by running a three-way merge on the last version both could have read (empty base if neither could read an earlier one, reported separately as `parallel_rewrites`). **The base is assumed (the latest version stored before both), not observed, and attribution compares each version with the one stored just before it, not the one its author read. `unused_conflicts` (conflicting and unused) is therefore an upper bound, not a count.** The direction of error is not proven in every case, so a pass on criterion 4 is confirmed by reading the flagged versions (below). **Hunk-header ranges are not used:** they include unchanged context lines, so disjoint edits within a few lines of each other would read as overlapping |
| `lost_rubric_items` | rubric items met by an unused spec version and not met by the approved one. This is the independent loss evidence: work that was real and was thrown away, not work merely charged in a revoked turn |
| `revocations`, `revoked_turn_actions` | informational only. A revoked turn's work is not necessarily discarded (it may be retained in another author's artifact), so these are an upper bound and never gate |
| `budget_refusals` | `rejected(turn_budget_exhausted)`; each also ends its turn, so the two are not added |
| `tokens_per_outcome` | **dollar cost** of every turn of a run, revoked turns included, divided by the number of successful runs it produced (0 or 1 per run, so per pair: the run's cost if it succeeded, otherwise the pair is failed and excluded). Each turn is priced from the `modelUsage` of its last `result` line (per model, subagents included) at the dollars-per-million-token prices in `docs/bbp/2b-criteria.json`, which carries the price-page date and is frozen with these criteria; a model with no price is an error, never zero. A turn with no result line, an incomplete turn, a crash result (`is_error`) or a killed or interrupted turn has an unknown cost, known only from below (what its printed messages show). That lower bound counts toward the run's total and toward the per-run cap, never as zero. The run counts as failed (a killed turn is a failed turn, as a revoked builder's harness is in an N=2 run), so its pair is out of criterion 7's ratio and counts against criterion 2 (at most 2 of 10 pairs may fail, and N=2 may fail at most 1 more than N=1). When every N=1 control succeeds, this permits at most one pair lost to a killed builder in N=2, bounding how far criterion 7 can be read off the survivors; overlapping N=1 failures can permit additional N=2 failures under the unchanged `f2 <= f1 + 1` rule. The failed pair's cost is reported apart (`failed_pair_costs` and `total_usd_at_least` in the `bbp pairs` output); the priced total is cross-checked against the line's `total_cost_usd` within 2%. `bbp tokens` reports the raw parts, the plain sum and the dollars side by side. Reported per pair as the N=2 over N=1 ratio. Not visible to the core: `bbp tokens` reads each harness's usage from the output the moderator keeps (#723) |
| `stale_verdicts` | verdicts accepted although a message or artifact the author could read, appended by another principal (not the runner) during the author's verdict turn, was never returned to the author by a read the log records (2a known gap). Exact, from the responses of accepted reads: a `get_artifact` is not a view of the messages and a short page does not count as seeing what it omitted. **This run cannot exercise the gap** (criterion 6) |
| `overshoot` | any N=2 run with `spent.messages` or `spent.bytes` above its total |

**Go to 2b only if all hold:**

1. **Safety (precondition).** `overshoot` is zero in every N=2 run, and replay equals live in every run. A violation stops everything and is fixed in 2a first.
2. **Comparable outcomes.** At least 8 of 10 pairs succeed, and the N=2 failure count exceeds the N=1 count by at most 1.
3. **Parallelism pays at all.** On the separable tasks, the median `spec_wall` ratio is at most 0.75 **and** at most one separable pair has a ratio above 1.0. If two authors are not clearly faster even without contention, co-editing one spec cannot help. Two conditions make the number trustworthy:
   - *Variance.* Every pair's ratio is reported, not only the median. The three null pairs (N=1 against N=1) give the run-to-run spread; if any null ratio is outside 0.85 to 1.15, the threshold is inside the noise and criterion 3 is **inconclusive**, not passed or failed. Five separable tasks is a screen, not an estimate, and the medians are read with that in mind.
   - *Measurement error.* `spec_wall` is read from `state_ms`, which has a resolution of one poll. A pair's error bound is `poll x (state boundaries inside the window)`, summed over both arms. The poll is fixed at no more than 1 s, and a pair whose bound exceeds 5% of its N=1 `spec_wall` is excluded as unmeasurable (reported, like a failed pair). If fewer than 4 of the 5 separable pairs remain, criterion 3 is inconclusive. The 5% bound is a fifth of the 25-point margin between 0.75 and 1.
4. **Contention is real.** In at least 3 of the 5 shared-section tasks, at least one unused spec version has a `spec_conflict` with the approved spec. `unused_conflicts` is an upper bound (above), so each flagged version is read by the owner or a named reviewer against the approved spec before it counts; a version the reader judges merged into the approved spec is removed.
5. **The loss is real and attributed.** In at least 3 of the 5 shared-section tasks, `lost_rubric_items` is at least 1, and the median `unused_share` over those tasks is at least 0.15. Work that a merge would have kept, not work that was merely revoked, has to be what is lost.
6. **The known gap is closed before 2b, not measured by this run.** The gap needs two non-final judges. This run's roster has one tester, so `stale_verdicts` is reported but is expected to be near zero for a structural reason, and a low value is **not** evidence that the gap is harmless. The criterion is therefore a build precondition: the verdict-binding gap is closed (the proposal "Log revision at grant", below, or a `read_rev` on verdicts) before 2b merges, whatever this run shows. (Option: a roster variant with two testers, below.)
7. **The cost is bounded.** The median `tokens_per_outcome` ratio (N=2 over N=1) over the successful pairs is at most 1.6. Two authors plan twice as much by construction; a 25% speedup bought at more than 1.6 times the tokens is not a win. The owner has accepted 1.6.

**Decision.**

Rows apply from the top and the first match decides; a later criterion never rescues an earlier failure. Criterion 3 holding and criterion 7 failing is the "7 fails" row whatever 4 and 5 show: a speedup that costs more than the bound is not built on.

| Outcome | Action |
| --- | --- |
| Noise gate fails (phase 1) | Stop; no phase 2 run; the owner decides |
| 1 fails | Fix 2a; no decision |
| 2 fails | Inconclusive; no decision; rerun with a corrected rubric or task set |
| 1 or 2 not yet known to hold (fewer than 10 pairs, or fewer than 8 of 10 succeeding) | Incomplete; no decision until 1 and 2 are known to hold |
| 3 inconclusive (noise or measurement error) | No decision; rerun the separable pairs with longer tasks or more repetitions |
| 3 fails | Stop concurrency work; keep `max_live_turns = 1` as the default and the only tested mode |
| 3 holds, 7 fails | Parallelism pays in time but not in cost; keep `max_live_turns = 1` the default, 2a stays opt-in; no CRDT |
| 3 and 7 hold, 4 fails | 2a alone is enough for spec authoring; no CRDT |
| 3, 7 and 4 hold, 5 fails | Contention exists but loses nothing the rubric values; section leases (option 4) or 2a, no CRDT |
| 1 to 5 and 7 hold | Build `rusty_crdt` and 2b (Appendix A and B), after criterion 6's precondition: the verdict-binding change lands first |
| 3 holds, but 7, 4 or 5 is not yet known to hold | Incomplete; no decision until 3, 7, 4 and 5 are known to hold |

**Grading order.** `bbp grade pack` writes `pack.json` (the approved specs: exactly two per task) and `key.json`, and holds the unused versions in `unused.held.json`. The grader scores `pack.json` first. `bbp grade release --key K --held H --scores S --out DIR` checks those scores cover every approved id (and no unused one), records their sha256 in `release.json`, and only then writes `unused.json`, which is scored separately. `bbp grade join` refuses unless the approved scores still hash to the recorded value, so they cannot be edited after the unused versions were seen.

**Tooling.** Built and merged: `spec_conflicts` and the retained-versus-unused attribution with `unused_spec_versions` and `unused_share` (#676, #677), and per-state wall timing in `bbp mod` (#678). Built, open for review: token capture `bbp tokens` (#723), the pair report `bbp pairs` (#724), the blind grader `bbp grade` (#725), gate answers with the `self_approved` label (#721). The rubrics, briefs, task classification and pass marks are committed and frozen in `docs/bbp/2b-run/` (owner approval 2026-10-21), with pinned control and treatment agents files (Sonnet 5.5 for every role). The `overlap` metric (file and hunk-header ranges on code diffs) is a screening proxy that overcounts context and is not used by the gate.

**Limits, stated up front.** Ten tasks is a small sample and the medians are coarse; this is a go/no-go screen, not an effect size. Rubric scoring is a judgment, even blinded. Blinding is mechanical, not absolute: the grader sees neutral ids in hash order, and the unused spec versions (which only a treatment run has) are held back until the approved scores are sealed, but an unused version is a draft of the same spec and shares much of its text with the approved one, so a grader who scores both can match them by similarity and so tell which run is the treatment. Sealing keeps the unused versions from informing the approved scores; it does not remove that residual limit, which is stated here and not otherwise mitigated. Results depend on the model and on how the briefs are written. Model tokens are not visible to the core; they come from the harness's own usage output, read by `bbp tokens` (#723). `spec_wall` on scripted agents measures host overhead only; scripted agents prove the tooling and do not count toward the 10.

**Review record.** The owner's review of the first draft required: (1) evidence in the artifact actually co-edited, not code diffs; (2) frozen comparable successful outcomes and predeclared failed-pair handling; (3) changed-line conflicts on a common base, not hunk-header ranges; (4) attribution of genuinely unused work. Each is applied above. A second review (2026-10-15) required: (5) the stale-verdict gap either exercised or declared uncovered (declared uncovered, criterion 6, with a two-tester roster variant offered); (6) `unused_conflicts` documented as an upper bound (done, criterion 4); (7) variance across paired tasks, token cost per accepted outcome, and a wall-clock threshold above poll error (criteria 3 and 7). The separate finding on the generic RGA id contract (from the earlier design review) is not addressed here.

## Run plan proposals (not approved, not built)

### Option: a roster variant with two testers

To exercise the verdict-binding gap, run a subset on a roster with a second non-final judge: `tester2`, the same role definition as `tester` with its own principal, `turn_order` after `tester`, serving `test`. Four extra N=2 runs (two shared-section, two separable), reported separately and never mixed into the 23. With two testers live, `stale_verdicts` can occur; criterion 6 would then read "`stale_verdicts` below 20% of verdicts across these four runs". Four runs are too few to estimate a rate, so a result above 20% is decisive and a result below it is not. **Recommendation: do not run it.** The gap's fix (below) is small and closes the question without a sample; the extra runs cost tokens to learn what the fix makes moot. The owner decides.

### Verdict binding: built as #720, without `at_rev`

The proposal here was `at_rev` on `TurnGranted`. The refusal does not need it: the fold already derives what a turn has been shown (appends and accepted read responses), so a verdict is refused `stale_subject` while content another principal appended during the judge's turn is unread (above one live turn only; the turn stays live, no turn is minted, no log format changes). That is [#720](https://github.com/Rusty-Mill/rusty_mill/pull/720) (merged), with a follow-up in [#727](https://github.com/Rusty-Mill/rusty_mill/pull/727): the read base of a judge is the entry into the stage, so a turn starts with everything others appended in the state unread, and a judge that reads nothing, or a replacement turn after an expiry, is bound too. #727 lands before 2b merges. `at_rev` would still give the spec metrics an observed read base (so `unused_conflicts` stops being an upper bound); it is a separate, optional schema 3 change and is not built.

### Cost in dollars, per model (owner decision)

Raw token sums mislead because the four counts are priced very differently, and a roster that mixes models cannot be normalized to one unit. **The cost figure is dollars.** The committed file `docs/bbp/2b-criteria.json` stores, per model id as the harness prints it, the dollars per million tokens for input, output, cache creation and cache read, plus `prices_as_of` (the date the owner read them off the price page); the run output records the file's sha256, and a run under a different file is void.

**Which the planned roster needs.** The shipped proving config (`docs/proving/agents.json`) launches every role as `claude -p` without pinning a model, so the model is whatever the harness defaults to and a roster can mix models without anyone choosing to. A single-model roster could be normalized, but a mixed one cannot, and an unpinned one is not known to be either. So the run needs dollars unless every role is pinned to one model, and the file prices exactly the models the owner lists: `bbp tokens` fails on any model not in it. Opus 5.5 and Sonnet 5.5 are filled in from the Claude API documentation and confirmed by the owner against the price page on 2026-10-19; any other model the roster uses must be added by the owner.

**Cache writes.** A cache write costs 1.25x input for a 5-minute lifetime and 2x for a 1-hour one, so the price file has `cache_creation_5m` and `cache_creation_1h` per model (1h >= 5m is checked). Captured against Claude Code 2.1.300: `modelUsage` does not split writes by lifetime; `usage.cache_creation` does, but only for the main agent. Subagents are forbidden in an evidence run, so that split is the whole run's: the 5-minute writes it reports are priced at the 5-minute rate and the rest at the 1-hour rate. With no split, or one that cannot be assigned (more than one model in `modelUsage`), every write is priced at the 1-hour rate, so cost is never understated. The moderator pins `CLAUDE_CODE_PROMPT_CACHE_TTL` and `CLAUDE_CODE_SUBAGENT_PROMPT_CACHE_TTL` to `1h` in both arms, `bbp tokens` logs the effective lifetime of every turn and of the run (`cache_ttl`: `1h`, `5m` or `mixed`) so the pilot shows it, and the priced total must agree with the harness's `total_cost_usd` within 2%. Fast mode, priority tier and regional inference are refused: their price is not in the file. A flag value the shell computes (`--tools "$t"`) is refused rather than read. `--tools default` (every tool, the Agent tool included) is refused, and so is a checked flag given more than once, since the launcher would say two things. A launcher may not set these variables or a model-routing one itself.

**Pinned models, checked before the run.** Every role the agents file launches pins a model with `--model ID` on its `claude` command (the program is `claude` by that name, or exactly `sh -c SCRIPT` over such commands (`sh ./wrapper.sh -c '...'` runs the wrapper, and options before `-c` are refused too); prompt text, comments, anything after `--` and other commands do not count; `--fallback-model`, `--agents` and `--settings` are refused), every pinned model has a price in the criteria file, and the control and treatment files pin the same model per role (an extra N=2 role pins one the control also uses). `bbp agents-check --control A --treatment B --criteria C` checks this; `bbp mod --criteria FILE --max-usd C` refuses to start otherwise. `bbp pairs` repeats the check on the models the harnesses actually used.

**What each harness must also do (captured 2026-10-15, Claude Code 2.1.300).** It is exactly one harness session (sessions share one log, and the last result accounts for the last session only), and it prints `--output-format stream-json` to its own log, unredirected and unpiped, because the cap reads that log. It cannot start a subagent (owner decision: subagents are forbidden in the evidence run, and Haiku stays unpriced and refused): `--disallowedTools Agent,Task` (or a `--tools` list without them) stopped a subagent in the capture, and the moderator also sets `CLAUDE_CODE_SUBAGENT_MODEL` to the role's pinned model, which the harness honoured (the subagent ran on it). And it loads no settings source at all, `--setting-sources ""`: the capture showed that `env` in a project's `.claude/settings.json`, or in `--settings`, overrides the process environment (a 5-minute cache lifetime beat the moderator's 1-hour pin), while `--setting-sources` kept it out; `user` is refused too, since a user settings file can carry an `env` as well. A launcher may not set the model or cache-lifetime variables itself.

### Pinning the run to a commit

The run uses a commit that contains the verdict-binding fix (#727). `min_commit` in the criteria file names the merge commit that contains both that fix and the run tooling (#763): `735f1b25`; `bbp pairs` refuses a criteria file with no `min_commit`, a checkout whose HEAD does not contain it, or a working tree that is not clean, and records `git_sha` (HEAD) and `criteria_sha256` in its output. It pins the checkout the manifest names, and every run's `bbp mod` output records the commit its binary was built from (`bbp_build_sha`, recomputed on every build, `-dirty` or `unknown` when not clean); `bbp pairs` refuses a run whose build is not that clean HEAD.

### Pilot reconciliation

The pilot pair runs on a dedicated API key. Before any counted run, `bbp tokens` for the pilot is reconciled against that key's usage as the Usage and Cost API reports it, per part (input, output, cache creation, cache read). **A part (or the dollars) differing by more than 2%, or any turn with no harness output, stops the run:** the capture is not trustworthy and the cap and criterion 7 would rest on it. The 2% is accepted by the owner and is not loosened: a failed reconciliation is reported as a finding about the capture, not tuned away. a killed harness prints less than it spent, and a retried request the harness never printed is spent but unseen, so a real mismatch is likely and the pilot is the place to find it. `bbp tokens --api-usage FILE` does the comparison (FILE holds the four numbers from the usage report, and `cost_usd` when it has one); a mismatch is a non-zero exit.

### Cost cap for the run

The run spends money and the harness may loop, so the cap is set before it starts, in dollars, and **matches the spend limit of the dedicated API key**: the key's limit is the hard stop. There is no measured cost for a BBP planning turn yet, so the cap is a rule, not a guess at a number:

1. **Pilot (not counted).** One N=1 pair on a task outside the set measures the dollar cost per run, reconciled as above. The pilot sets only this figure; it changes no criterion, threshold or price.
2. **Per-run cap** `C = 3 x` the pilot's N=1 cost per run, rounded up. `bbp mod --max-usd C --criteria FILE` checks it every poll, including after the final one, and kills the harnesses over `C`; a run over `C` counts as failed under the failed-pair rule (`cap_usd` in the manifest). The check lags by a poll and by what a harness has printed, so the key's limit stays the hard stop. The check fails closed: a harness log with JSON the tool cannot read, or plain text and no stream-json at all, stops the run, and a log that holds no assistant or result line at the end of the run does too. Text that is not valid JSON (stderr shares the log, and a line still being written or cut off by a kill is no object) is noise; valid JSON the tool cannot read as a stream-json line stops the run, since its usage would be skipped. A stopped run loads in `bbp pairs` as a failed run even when its log cannot be read, and needs no model evidence. While the run is live a harness that has printed nothing yet, or only the preamble a real harness prints first (hook, goal and compaction lines and the `system` init line, captured), is left alone, and an assistant message printed after the last result counts. A stopped run (cap or `--max-wall-secs`) still prints its `bbp mod` JSON with `stop` (`cap` or `wall`) and `stop_reason`, exits non-zero, and loads in `bbp pairs` as a failed run. A granted turn with no log at the end, or a log that exists and cannot be read, also stops the run (a log not yet created while the run is live is left alone). The moderator lock is taken before the evidence marker is inspected or written, so a refused startup leaves none. Only a task nothing has run on can start as an evidence run; one started without a cap cannot be resumed with one, one started with a cap cannot be resumed without it, and `bbp pairs` counts only runs that carry the cap's marker. A resume under the same cap stays capped, but each capped `bbp mod` records its segment (the build it ran) beside the marker before it acts on the task, and `bbp pairs` refuses a run with no segment, more than one, or one built from another commit than its saved output reports: it judges a run by that one output, so a resumed run, whose earlier stops, builds and state timings that output does not show, is never evidence (#1326). A marker written before segments were recorded is refused, and an older `bbp` refuses the new one.
3. **Total cap** `23 x C` (one `C` per unique run), set as the key's spend limit, and an early stop: if half the total is spent with fewer than half of the pairs complete, stop and report; the screen is inconclusive and nothing is decided.
4. **Enforcement** is outside the protocol: the key's spend limit, plus `--max-usd` (a live check that lags a poll) and `--max-wall-secs`. No code in `rusty_bbp`.
5. **Ended turns finish, they are not cut.** A harness's `result` line comes after its terminal call is answered, so a harness killed as soon as its turn ends leaves that turn's cost unknown and fails its run. `bbp mod --end-grace-secs` (default 60) lets a harness whose turn ended, by its own terminal call or by revocation, exit on its own and print that line before it is killed; cap and wall stops still kill at once. Spend in the grace is in the `result` and counts. A revoked N=2 author is a failed pair unless its harness stops within the grace, so the spec authors' prompts must say that a `stale_turn` refusal ends the turn.

The owner approves the resulting `C` and the key limit after the pilot. Until the pilot exists there is no number to approve, and none is proposed here.

## Open questions for the owner (max 5)

1. **Phase gate.** Approve 2a as the next implementation step, and is 2b conditional on evidence from it?
2. **Sealing.** Should the plan gate seal the spec by default (calmer review) or void on edit (more flexible)?
3. **Granularity.** Line-granular RGA to start, with per-character as the fallback?
4. **`max_live_turns` default and cap.** Default 1 is assumed; what upper bound is acceptable for a first release?
5. ~~Scheduling.~~ **Decided:** `pick_next` returns a set capped by policy; `TurnBased` and Phase 1 cap at one.

## Open questions for the `rusty_crdt` extraction (new)

6. ~~Serde feature.~~ **Decided:** option A, a `rusty_serde` feature (Appendix B).
7. **Nexus re-point.** `FROZEN.md` says no new integrations or adapters and removes Nexus from the workspace. The thin adapter is blocked until Nexus is unfrozen, and moot if it is retired. Recorded as a conditional follow-up; nothing is done now.
8. ~~Location.~~ **Decided:** `crates/foundation/rusty_crdt`.

## Follow-ups for orch (not touched here)

- Choose `max_live_turns`, slice sizes and the sealing option per factory, and pass them with the roster at `task_opened`.
- Decide which roles are worth running concurrently; the protocol only enforces the cap.
- If a host wants character-level co-editing UIs, that adapter lives above BBP.

## What I could not verify

- Contention behavior of `expected_rev` retry with many writers (no benchmark; the A3 retry path exists but I did not measure it).
- Whether `fs_store` is safe for several writer processes beyond what `refresh` provides.
- The three external crates' properties (stated from memory) and that none fits Tier S.
- Whether line granularity is enough for real specs.
- Performance of the extracted RGA on real specs (the O(n) position lookup and the `BTreeMap` index are choices made on reading, not on measurement).
- That the proposed law tests find no counterexample: they are specified in Appendix A, not run.
- Log growth under real edit sessions.

## Appendix A: `rusty_crdt` API sketch (not code, unbuilt)

Built at the start of 2b, gated on 2a evidence. Sketch only; nothing here compiles yet.

```toml
# crates/foundation/rusty_crdt/Cargo.toml
[package]
name = "rusty_crdt"
version = "0.1.0"
edition = "2021"
description = "Sequence CRDT (RGA): sync, dependency-free, generic over id and element"
license = "MIT OR Apache-2.0"

[package.metadata.rusty_mill]
layer = "foundation"

[dependencies]          # none

[features]
default = []
# Mirrors rusty_uuid: serialization through the workspace's own rusty_serde.
rusty_serde = ["dep:rusty_serde"]

[dev-dependencies]
proptest = "1"          # independent oracle, dev-only (ADR-0002 Tier S)

[lints.rust]
unsafe_code = "forbid"
[lints.clippy]
unwrap_used = "deny"
expect_used = "deny"
```

```rust
//! Contract the caller owns: ids are unique across all replicas and totally
//! ordered. Among siblings the GREATER id renders first (newest to the left).
//! Two different ops with one id make replicas diverge; the caller prevents it.

pub struct Rga<I, T> { /* arena of nodes, BTreeMap<I, usize> index, roots, visible count */ }

#[derive(Clone, Debug, PartialEq, Eq)]
pub enum Op<I, T> {
    /// Insert `value` after `parent` (`None` = head).
    Insert { id: I, parent: Option<I>, value: T },
    /// Tombstone `target`. `id` identifies this delete op to the caller; the RGA ignores it.
    Delete { id: I, target: I },
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub enum Applied<I> {
    Changed,
    Duplicate,
    /// A dependency (parent or target) is unknown. State is unchanged.
    Pending { missing: I },
}

impl<I: Clone + Ord, T> Rga<I, T> {
    pub fn new() -> Self;
    pub fn len(&self) -> usize;                       // visible elements, O(1)
    pub fn is_empty(&self) -> bool;
    pub fn contains(&self, id: &I) -> bool;           // live or tombstoned
    pub fn iter(&self) -> Iter<'_, I, T>;             // visible (&I, &T), RGA order, iterative
    pub fn id_at(&self, pos: usize) -> Option<&I>;    // O(n)
    /// Build the op for a local insert at visible position `pos` (`pos == len()` = tail).
    pub fn insert_at(&self, pos: usize, value: T, id: I) -> Option<Op<I, T>>; // None if pos > len
    pub fn delete_at(&self, pos: usize, id: I) -> Option<Op<I, T>>;            // None if out of range
    pub fn apply(&mut self, op: &Op<I, T>) -> Applied<I> where T: Clone;       // idempotent; never panics
}
impl<I, T> Default for Rga<I, T> { /* empty */ }
```

**Id types.** No trait to implement: any `Clone + Ord` type works. `Ord` defines the sibling tiebreak. Nexus passes its existing `OpId { lamport, site: Uuid }` (the derived order is already lamport-first). BBP passes `(lamport: u64, site: principal index)`, with the lamport allocated by the BBP engine from the log.

**BBP use.** The engine calls `apply` while validating a co-edit command. `Pending` is rejected `ref_unresolved` (BBP's log is already causal, so no buffering); `Duplicate` is idempotent success (R15 replay). The snapshot hash is SHA-256 over `iter()` joined canonically, computed in `rusty_bbp` with `rusty_rsa`, so the crate needs no hashing.

**CRDT law tests, in `rusty_crdt` (proptest):**

1. **Convergence.** For any causally valid op set, applying it in any causally valid order yields the same `iter()` (a generator builds ops from a random multi-site history; replays shuffle while retrying `Pending`).
2. **Idempotence.** Re-applying any op returns `Duplicate` and leaves state unchanged.
3. **Commutativity of independent ops.** Two ops with no dependency between them commute.
4. **Union.** Replicas holding sets A and B converge after exchanging the differences, in any order.
5. **Delete versus insert.** A delete delivered before its target returns `Pending` and succeeds when retried after the insert; the result matches in-order delivery.
6. **Single-site model.** A sequence of positional edits from one site renders the same as the same edits on a `Vec`.
7. **Intention.** An element inserted after parent P renders after P and before any element that followed P at authoring time, absent concurrent siblings.
8. **Stack safety.** A 300,000-element linear chain renders without overflow (the regression `nexus-crdt` already fixed).
9. **No-panic.** Arbitrary garbage ops (unknown ids, duplicate ids with different payloads) never panic. The same-id divergence is asserted as documented, not as a law.

**Differential test with `nexus-crdt`** (same random op streams through both implementations) belongs to the later adapter PR on the Nexus side, not here: `rusty_crdt` must not depend on an app crate.

**Out of scope for the crate:** op logs, version vectors, gossip, compaction, counters, sets, maps, any I/O or async.

## Appendix B: ADR draft (number provisional: ADR-0010)

**ADR-0010 (draft): `rusty_crdt`, a shared sequence CRDT**

Status: Proposed (direction accepted by the owner 2026-10-13; filed as an ADR when 2b is approved)
Date: 2026-10-13

**Context.** BBP v0.5 needs concurrent co-editing of a spec (this note, 2b). The workspace has one CRDT, `nexus-crdt`, an app-layer crate coupled to the editor and carrying `serde`, `uuid`, `thiserror`, `tokio` and `tracing`. Nexus is frozen and outside the workspace. `rusty_bbp` is Tier S. The consolidation audit says to extract `nexus-crdt` only when a second consumer appears; BBP is that consumer.

**Does Tier S allow a first-party workspace crate? Yes, with no amendment.** ADR-0002 defines "external" as crates.io or git sources in `[dependencies]` or `[build-dependencies]`, says a crate's tier is a claim about its own manifest, and says a Tier A crate may depend on Tier S crates. `rusty_bbp` already depends on `rusty_serde`, `rusty_rsa` and `rusty_atomic_file` this way. The only requirement is that the dependency resolve by path or `{ workspace = true }` against a path entry (`ATLAS-RWC-0050`, checked by the `dependency-policy` job), and that `rusty_crdt` itself be Tier S so the claim holds transitively.

**Decision (proposed).**

1. Create `crates/foundation/rusty_crdt`: the RGA only, generic over id and element, sync, no I/O, zero dependencies in the default build, `unsafe` forbidden.
2. Tier S. Serialization, if wanted, is a feature on the workspace's own `rusty_serde`, following `rusty_uuid`. `rusty_bbp` does not enable it: it serializes ops through its own event types.
3. Add `rusty_crdt` to `[workspace.dependencies]` by path. `rusty_bbp` depends on it with default features.
4. CRDT laws are tested in `rusty_crdt` with `proptest` as a dev-dependency.
5. Counters, sets and maps are not added until each has two call sites.
6. `nexus-crdt` is untouched. Re-pointing it through a thin adapter is a separate change on the Nexus side and is conditional on Nexus being unfrozen (`FROZEN.md`).

**Decided (owner, 2026-10-13): option A.** The table is kept as the record of the alternatives.

| Option | Tier S by the letter? | Cost | Fit |
| --- | --- | --- | --- |
| **A. `rusty_serde` feature (decided)** | Yes | Nexus's real `serde` types need their own derive in the adapter | Precedent: `rusty_uuid` |
| B. No serialization in the crate; public plain-data `Op`; adapters own wire types | Yes | Each consumer writes a few derives | Smallest crate; the cleanest Tier S claim |
| C. Optional external `serde` feature (as briefed) | No: an optional external dependency is still declared in `[dependencies]`. Defensible under ADR-0002's "explicitly scoped shipping profile" sentence (default profile sovereign), but that sentence is not a rule | Needs a one-line ADR-0002 clarification that a crate is Tier S in its default feature set when optional external dependencies are off | Matches the original ask; spends policy |
| D. Vendor the RGA into `rusty_bbp` | Yes | Duplicates code if Nexus is ever revived | Documented fallback only (same API) |

**Consequences.** One more Tier S crate and one ADR. The RGA gets isolated law tests. BBP gains co-editing without an external dependency. If Nexus is never unfrozen, the extraction removes no duplication; its value is isolation and a reusable core.

**Not decided here.** Whether to build it at all before 2a evidence exists (it is gated on that), and the Nexus adapter.
