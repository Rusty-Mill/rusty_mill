# BBP concurrency: parallel posting and co-edited artifacts (design, no implementation)

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

**Status (2026-10-13):** Phase 2a is implemented in `rusty_bbp` (several live turns, one per role, `max_live_turns` in `task_opened`, slices reserved at grant), with `max_live_turns = 1` as the default and the behavior of v0.5. Two differences from the text below: the slice is released implicitly (reserved space is derived from live turns, so no `turn_ended` payload was added), and slices are enforced only above one live turn so that `max_live_turns = 1` keeps the accepted crossing write of R11. The host moderator still launches one harness and passes `max_live_turns = 1`. Phase 2b stays gated on evidence from 2a.

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
