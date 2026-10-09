# Blackboard Protocol (BBP) v0.3: Draft Design

Oct 8, 2026 · revision of v0.2 after a third round of three independent reviews (`review/v0.2-merged.md`)

## Summary

**Status:** draft v0.3, protocol only. Nothing is built. Round three found no architectural problems; it found event sequences the state machine did not cover. v0.3 therefore makes the transition table the normative core and fixes the sequences the reviewers traced.

BBP lets several AI agents collaborate on one software task through a shared store without a human or script relaying messages. It defines the records the store holds, who may write which record and when, what each role may read, how a task moves between states, and which transitions need a human. It defines no transport, discovery, authentication or encryption.

**Why it exists.** [MCP](https://modelcontextprotocol.io/specification/2025-11-25/server/resources), [A2A](https://a2a-protocol.org/latest/specification/) and [AG-UI](https://docs.ag-ui.com/concepts/architecture) cover tools, agent-to-agent tasks and agent-to-user events. None defines shared-task policy: per-role visibility, evidence rules, budgets, revision-bound approvals and human gates. BBP is that policy layer, exposed first through MCP.

**The main idea.** Agents annotate a shared store and refer to things by id. Large content is stored once as an immutable blob. Every agent mutation is authorized by a short-lived execution token the adapter injects. Every approval is bound to an exact revision: a spec at the plan gate, a candidate and the test run that judged it at the merge gate.

**Context.** Claude, ChatGPT/Codex, Gemini, Ollama and Hermes agents work on one repository. A human approves the plan and the merge. Retries are bounded, verdicts carry evidence from a sandboxed runner, review contexts are fresh, and no agent or workload holds push or deploy credentials.

### Changes from v0.2

- **Runs.** Each runner execution has a run id and a frozen profile digest. Verdicts and merge approval name the run. A new run for the current candidate stales its verdicts and returns the task to `test` (all three reviews).
- **Transition table** with guards and precedence is now the normative lifecycle (ChatGPT E).
- **Cut attempt generations.** Tokens and deadlines fence stale writes on their own (Nano, Oct 8; Claude, Gemini).
- **Cut `changes_spec`.** A spec change is a human rejection to `planning`, which clears the approved spec (Nano; all three).
- **Cut the `escalate` verdict value.** `request_decision` covers it. `revise` requires a blocking item (Nano; Claude F12).
- **Yield ends the yielder's turn**; return is guarded (Claude F7, ChatGPT F3, Gemini F2).
- **Continuation**: `escalated` records its origin, settling a decision request regrants the requester, entering `test` with a current run grants the Tester (Claude F4, F9; ChatGPT F4; Gemini F3).
- **Reads are token-fenced**, one charge per artifact per turn, plus task-level `reads` and `turns` budgets; resuming exhausted work needs a recorded extension (Claude F8, ChatGPT F5, F6, Gemini F5).
- **`op` lookup precedes every other rule** (Claude F3, ChatGPT F8).
- **Token injected by the adapter**, absent from tool schemas and the card (Claude F11, Gemini F6).
- **Manifest drops `repo`**; the task owns it. Report consistency is validated (Claude F5, ChatGPT F7, Gemini F4).
- Reviewer reads human messages addressed to it; gate requests carry a `gate` flag and no deadline; artifact size pre-flight; host `abort_turn`; Planner and Reviewer stubs in the proving run.

## Goals, non-goals and principles

### Goals

1. Agent-to-agent collaboration without a relay.
2. Reduce repeated context transfer. Measured, not assumed.
3. Evidence-backed claims: a verdict on a candidate names the sandboxed run that judged it.
4. Fresh reviews by construction: a role's read capability decides what it sees.
5. Human authority at the gates, bound to an exact revision and run.
6. Bounded work: messages, bytes, reads, turns and iterations have limits.
7. Vendor neutrality: any agent that can call an MCP tool can participate.

### Non-goals for v0.3

- Transport, discovery, authentication, encryption.
- Free-form chat.
- Long-term memory.
- Replacing MCP, A2A or AG-UI.
- Autonomous merge or deploy. The human merges after the merge gate and records a receipt.
- Agent-recorded decisions (Appendix B) and test waivers.

### Design principles

- **Records over conversation.**
- **Pull over push.** Agents fetch; the moderator grants turns and never forwards content.
- **Enforce in the store, not in the prompt.** Every rule is checkable from data the store holds.
- **Make illegal states unrepresentable.** Typed bodies, a closed transition table, one capability matrix, one gating table.
- **Coherence over counts.** A rule earns its place by an adversarial test and a cited failure scenario.
- **Protocol before implementation.**

## Principals and roles

A **principal** is anything that writes to the store. Each principal record carries `id`, `kind` (`human`, `moderator`, `runner`, `agent`) and for agents a `role` and a `vendor`.

| Principal | Writes | Through |
| --- | --- | --- |
| Human | human events, `decision`, `ask`, `answer` | Human channel |
| Moderator | events, accounting | Internal |
| Runner | `test_report`, `log`, bound to a candidate and run | Runner API |
| Planner, Coder, Tester, Reviewer | messages and artifacts per the matrices | Agent API, with a live token |

### Host requirements

The store cannot verify these; they are stated so the host can be audited.

- **Fresh sessions.** One agent session per role per task. The Reviewer's session is new for each candidate it judges.
- **Role fixed at launch.** One Agent API process per role per task. No field an agent can set changes role, task or principal.
- **Token injection.** The adapter attaches the current execution token to every call from the process it serves. The model never sees or emits it. The adapter never substitutes a newer token for an older process.
- **Runner is a supervisor outside the sandbox.** The supervisor holds the runner credential and writes reports. The workload has no BBP, push or deploy credentials, no network egress, a read-only mount of the base, bounded output, and time and resource limits. The host contract lists the dependencies available inside the sandbox.
- **Envelope limits.** `max_artifact_bytes` and `max_body_bytes` are enforced before any store work.
- **`abort_turn`.** The host may end a turn early (cause `aborted`) when an agent process dies or an inference call times out.

## Data model

Five record types. Blobs, artifacts, messages and events are append-only. The task card is a fold over events.

### Blob

Content addressed by the full SHA-256 of its bytes. No metadata, no access policy, no raw read.

### Artifact record

| Field | Type | Meaning |
| --- | --- | --- |
| `id` | `art:<ulid>` | Record id |
| `task` | task id | Owning task |
| `author` | principal id | Who stored it |
| `kind` | enum | `spec`, `diff`, `candidate`, `test_report`, `log` |
| `blob` | sha256 | Content |
| `candidate` | `art:` or null | For runner artifacts: the candidate judged |
| `run` | run id or null | For runner artifacts: the run that produced them |

A reference may carry a fragment, `art:01H...#L40-L52`, at most 128 characters, opaque to the store.

#### Gating table for artifact writes (R16)

| Kind | Content | Writer | State | Token |
| --- | --- | --- | --- | --- |
| `spec` | Specification, Markdown | Planner | `planning` | required |
| `diff` | Unified diff against the candidate base | Coder | `build` | required |
| `candidate` | Manifest below; storing it is submission | Coder | `build` | required |
| `test_report` | Typed result below | Runner | `test`, `review`, `merge_gate` | none; bound to candidate and run |
| `log` | Raw workload output | Runner | same | none; bound to candidate and run |

#### Candidate manifest

```json
{
  "spec": "art:01HSPEC",
  "base": "a1b2c3d4e5f6...40 hex",
  "diffs": ["art:01HDIFF1", "art:01HDIFF2"]
}
```

The repository is the task's, fixed at `task_opened`. `spec` must equal the approved spec (R17). `base` is a full commit id in that repository. `diffs` apply in index order. The task has one **current** candidate, the most recently stored. Storing one records `candidate_submitted` and stales every verdict and approval on earlier candidates.

#### Run and test report

A **run** is one supervised execution of the required profile set against one candidate. The moderator assigns the run id when it triggers the runner. The profile set and its digest are fixed at `task_opened`.

```json
{
  "candidate": "art:01HCAND",
  "run": "run_03",
  "profile_digest": "sha256 of the frozen profile set",
  "status": "passed",
  "profiles": [
    {"name": "test", "command": "cargo test --workspace", "exit_code": 0, "failed": 0}
  ],
  "tree": "sha256 of the resulting tree, algorithm named in the profile set",
  "sandbox": {"network": "none", "timeout_s": 900},
  "log": "art:01HLOG"
}
```

`status` is `passed`, `failed` or `error`. It is `passed` only if every required profile has `exit_code: 0` and `failed: 0`; the store rejects a report that says otherwise (R16). The **current run** of a candidate is the latest run with a stored report.

### Message

```json
{
  "v": 3,
  "id": 41,
  "task": "T06",
  "from": "tester",
  "to": ["coder"],
  "kind": "finding",
  "reply_to": 39,
  "body": "AC2 has no failing case for HTTP 429.",
  "refs": ["art:01HSPEC#L12-L18", "art:01HDIFF1#L40-L52"],
  "evidence": ["art:01HREP"]
}
```

`id` is a per-task monotonic integer assigned by the store. `to` holds assigned roles, `human`, or `*`. `refs` and `evidence` hold `art:` or `msg:<id>` ids on the same task. No edit, delete or supersede; a correction is a `finding` with `reply_to`.

### Message kinds

| Kind | Purpose | `refs` | `evidence` | `reply_to` | Agents may post |
| --- | --- | --- | --- | --- | --- |
| `ask` | Request information | optional | no | optional | yes |
| `answer` | Reply to an `ask` | optional | optional | required, names an `ask` | yes |
| `propose` | Suggest a change or approach | required | optional | optional | yes |
| `finding` | Defect, fact or objection | required | required | optional | yes |
| `verdict` | Judge the current candidate | required | required | no | Tester, Reviewer |
| `request_decision` | Ask the human; `gate: "plan"` opens the plan gate | required | optional | optional | yes |
| `decision` | Record a decision | required | optional | optional | human only |
| `pass` | End the turn; may `yield_to` a role | no | no | no | yes |

#### Verdict body

```json
{
  "subject": "art:01HCAND",
  "run": "run_03",
  "verdict": "approve",
  "blocking": [],
  "non_blocking": [{"id": "N1", "ref": "art:01HDIFF1#L40-L52", "issue": "...", "fix": "..."}]
}
```

`subject` is the current candidate and `run` its current run. `verdict` is `approve` or `revise`. `evidence` includes that run's report. `approve` requires the run `passed`; `revise` requires at least one `blocking` item (R3).

#### Decision body

```json
{"subject": "msg:37", "outcome": "accept", "note": "..."}
```

Human-only. `subject` is the `request_decision` or `propose` settled. A spec change is not a decision; it is a `human_rejection` to `planning`.

### Event

| Event | Writer | Payload |
| --- | --- | --- |
| `task_opened` | moderator | repo, profile set and digest, budget |
| `assigned` | moderator | role, principal |
| `turn_granted` | moderator | role, turn id, deadline |
| `turn_ended` | moderator | turn id, cause: `pass`, `yield`, `verdict`, `candidate`, `request`, `deadline`, `budget`, `aborted`, `revoked` |
| `candidate_submitted` | moderator | candidate id |
| `run_started` | moderator | candidate id, run id |
| `test_report_stored` | moderator | candidate id, run id, status |
| `state_changed` | moderator | from, to, cause id |
| `human_approval` | human | gate, subject: `spec` id at `plan_gate`; `candidate` id and `run` id at `merge_gate` |
| `human_rejection` | human | gate or state, subject, reason, target: `planning` or `build` |
| `merge_receipt` | human | candidate id, merged revision |
| `cancelled` | human | reason |
| `escalated` | moderator | reason, from state |
| `resumed` | human | target state |
| `budget_extended` | human | field, new limit |
| `charged` | moderator | principal, turn id, messages, bytes, reads |
| `rejected` | moderator | principal, code |
| `usage` | moderator | host-reported tokens, advisory |

### Task card

```json
{
  "id": "T06",
  "state": "test",
  "turn": {"role": "tester", "deadline": "2026-10-08T16:10:00Z"},
  "iteration": 2,
  "spec": "art:01HSPEC",
  "candidate": "art:01HCAND",
  "run": {"id": "run_03", "report": "art:01HREP", "status": "passed"},
  "pending_request": null,
  "budget": {"messages": 40, "bytes": 5000000, "reads": 150, "reads_per_turn": 30, "turns": 60, "iterations": 4},
  "spent": {"messages": 11, "bytes": 318000, "reads": 54, "reads_this_turn": 3, "turns": 17},
  "rejections": 3,
  "advisory": {"tokens": 31800}
}
```

Reading the card is free and needs no token.

## Rules

### Evaluation order

1. Authenticate the principal and its bound task. Unknown principal or wrong task: `forbidden`.
2. Terminal state (R18) for anything that is not an `op` replay.
3. `op` lookup (R15). An accepted `op` with an identical payload returns the original result and stops here. This is not a write and is not charged.
4. All remaining rules. The first failure is the rejection code.

A rejection writes a `rejected` event and nothing else, except where R11 says the rejection ends the turn or escalates the task.

| ID | Rule | Rejects with |
| --- | --- | --- |
| R1 | `kind` is one of the eight kinds; `v` is a known integer. | `unknown_kind`, `unknown_version` |
| R2 | `propose`, `finding`, `verdict`, `request_decision`, `decision` carry at least one `refs` entry. | `refs_required` |
| R3 | `finding` and `verdict` carry `evidence`. A `verdict` names its subject's current run and cites that run's report; `approve` requires the run `passed`; `revise` requires one `blocking` item. | `evidence_required`, `evidence_unbound`, `not_passed`, `blocking_required` |
| R4 | Every id in `refs`, `evidence`, `reply_to`, `subject`, `candidate.spec`, `candidate.diffs`, `report.candidate`, `report.log` resolves to a record on the same task of the expected kind. Fragments are opaque and at most 128 characters. | `ref_unresolved`, `wrong_kind`, `fragment_too_long` |
| R5 | The author is assigned, the role may post the kind, and the state accepts the kind. | `not_assigned`, `role_forbidden`, `wrong_state` |
| R6 | `verdict` comes from Tester or Reviewer; its `subject` is the current candidate and appears in `refs`. | `role_forbidden`, `stale_candidate`, `subject_missing` |
| R7 | `decision` comes only from the human channel. | `decision_forbidden` |
| R8 | Prose `body`, and each `issue` and `fix`, is at most 1,200 characters. | `body_too_long` |
| R9 | `to` lists assigned roles, `human`, or `*`. | `bad_recipient` |
| R10 | `reply_to` names an earlier message on the task; an `answer` replies to an `ask`. | `bad_reply_to` |
| R11 | Accepted agent writes charge `messages` and `bytes`. Charged reads (`read`, `get_artifact`) carry a live token and charge that turn's `reads_per_turn` and the task's `reads`, once per artifact id per turn. Every `turn_granted` charges `turns`. Exceeding `reads_per_turn` ends the turn (`budget`). Exceeding `messages`, `bytes`, `reads`, `turns` or `iterations` records `escalated`. Human, moderator and runner operations are never charged. | `budget_exhausted`, `turn_budget_exhausted` |
| R12 | Records are never edited or deleted. | `immutable` |
| R13 | Human events arrive only through the human channel. The Agent API has no operation that creates one. | `approval_forbidden` |
| R14 | Every Agent API mutation and charged read carries a live execution token: issued by `turn_granted` for this task and principal, before its deadline, not ended. Runner and human principals authenticate through their own channels and carry no token. | `bad_token`, `token_expired` |
| R15 | Every Agent API mutation carries an `op` scoped to (principal, task). Same `op`, same payload: original result. Same `op`, different payload: reject. Rejected ops are not recorded. | `op_conflict` |
| R16 | An artifact write matches the gating table and the envelope limit. Runner writes name a candidate on this task and the run the moderator started for it. A `test_report` with `status: passed` has every required profile at `exit_code: 0` and `failed: 0`, and its `profile_digest` equals the task's. | `kind_forbidden`, `wrong_state`, `too_large`, `candidate_required`, `unknown_run`, `inconsistent_report`, `profile_mismatch` |
| R17 | `human_approval` at `plan_gate` names the `spec` in the open gate request. At `merge_gate` it names the current candidate and its current run; that run is `passed` and a Reviewer `approve` naming the same candidate and run exists. `candidate.spec` equals the approved spec. | `stale_subject`, `not_reviewed`, `not_passed`, `spec_mismatch` |
| R18 | No write of any kind is accepted in `closed` or `cancelled`, and neither state can be resumed. | `terminal` |

## Reads and visibility

- `read(after, limit)` returns messages with `id > after` visible to the caller's role, at most 20, with `next` and `more`. At-least-once; clients dedupe on `id`. Charged (R11), token required (R14).
- `get_artifact(id, range)` returns the record and bytes within an optional range, with `more`. Charged once per artifact id per turn. Token required.
- `task_card()` is free and needs no token.

### Capability matrix

| Role | Messages readable | Artifact kinds readable | Kinds postable |
| --- | --- | --- | --- |
| Planner | all on the task | all | `ask`, `answer`, `propose`, `finding`, `request_decision`, `pass` |
| Coder | all on the task | all | same as Planner |
| Tester | all on the task | all | `ask`, `answer`, `finding`, `verdict`, `request_decision`, `pass` |
| Reviewer | messages from `human` addressed to `reviewer` | `spec`, `diff`, `candidate`, `test_report` | `finding`, `verdict`, `request_decision`, `pass` |
| Runner | none | `candidate`, `diff` | none |

## Lifecycle

Ten states. The transition table is normative; the prose after it is commentary.

### States

`planning`, `plan_gate`, `build`, `test`, `review`, `merge_gate`, `approved`, `escalated`, `closed`, `cancelled`. The last two are terminal.

### Transition table

Rows are evaluated top to bottom; the first matching row fires. "Grant X" means `turn_granted` to X. Every transition records `state_changed` and ends any open turn (`revoked`) unless the row says otherwise.

| # | In state | Trigger | Guard | Effect |
| --- | --- | --- | --- | --- |
| T1 | any non-terminal | human `cancelled` | | → `cancelled` |
| T2 | any non-terminal except `planning`, `plan_gate` | human `human_rejection(target: planning)` | | clear approved spec and all candidate approvals; → `planning`; grant Planner |
| T3 | any non-terminal | task budget exceeded (`messages`, `bytes`, `reads`, `turns`, `iterations`) | | `escalated(from)`; → `escalated` |
| T4 | any non-terminal | `request_decision` deadline passed | request has a deadline | `escalated(from)`; → `escalated` |
| T5 | `planning` | Planner `request_decision` with `gate: "plan"`, `to` includes `human`, `refs[0]` a `spec` | | open gate request; → `plan_gate` |
| T6 | `plan_gate` | `human_approval(spec)` | spec equals the open request's spec (R17) | approved spec set; → `build`; grant Coder |
| T7 | `plan_gate` | `human_rejection` | | → `planning`; grant Planner |
| T8 | `build` | `candidate` stored (R16) | | `candidate_submitted`; start run; → `test`; no turn until T9 |
| T9 | `test` | `test_report_stored` for current candidate | | grant Tester |
| T10 | `test` | Tester `approve` | R3, R6 | → `review`; grant Reviewer |
| T11 | `test`, `review` | Tester or Reviewer `revise` | | `iteration += 1`; → `build`; grant Coder |
| T12 | `review` | Reviewer `approve` | | → `merge_gate` |
| T13 | `review`, `merge_gate` | `test_report_stored` for current candidate (new run) | | stale verdicts on that candidate; → `test`; grant Tester |
| T14 | `merge_gate` | `human_approval(candidate, run)` | R17 | → `approved` |
| T15 | `merge_gate` | `human_rejection(target: build)` | | `iteration += 1`; → `build`; grant Coder |
| T16 | `approved` | `merge_receipt` | names the approved candidate | → `closed` |
| T17 | `approved` | `human_rejection(reason: base_drift, target: build)` | | clear candidate approvals, keep spec; `iteration += 1`; → `build`; grant Coder |
| T18 | `escalated` | human `resumed(target)` | target is the recorded `from` or one of `planning`, `build`, `test`, `review`; exhausted budgets have a `budget_extended` | → target; entry grant per T19 |
| T19 | entering `build`, `test`, `review`, `planning` by any row | | `test` requires a current run, else start one | grant Coder, Tester, Reviewer or Planner respectively |
| T20 | any state with an open `request_decision` | human `decision` with that `subject` | | cancel its deadline; grant the requester if the state grants that role |
| T21 | any state | agent `pass` with `yield_to` | target role has a turn in this state; no yield already open | end turn (`yield`); grant target; record return-to |
| T22 | any state | yielded turn ends | state and candidate unchanged since the yield, no human wait opened | grant the yielding role (fresh turn) |
| T23 | any state | yielded turn ends | otherwise | apply the state's default schedule |
| T24 | any state | turn deadline, `abort_turn`, or `reads_per_turn` exceeded | | `turn_ended(deadline, aborted, budget)`; grant per the state's default schedule |

Default schedule: `planning` Planner; `build` Coder; `test` Tester once a run exists; `review` Reviewer; `plan_gate`, `merge_gate`, `approved`, `escalated` grant a turn only to answer a human `ask` addressed to that role, and `pass` returns to waiting.

### Commentary

- **Only one turn is live.** A yield ends the yielder's turn; the return is a fresh `turn_granted` (T22). There are no attempt generations: a deadline ends the turn (T24), and the dead token (R14) is the fence.
- **Evidence freshness.** A new run on the current candidate stales verdicts and returns the task to `test` (T13), so an approval can never outlive the run it named (R17).
- **Human waits do not spend agent budget.** A `request_decision` ends the requester's turn (`request`); its deadline is its own, and a gate request has none. Settlement regrants the requester (T20).
- **Base drift** is detected by the human or merge tool at merge time and recorded as T17. A rebased candidate is a new candidate and goes through `test` and `review` again. The spec approval survives.
- **Recovery.** The moderator rebuilds the card from events on restart, records `turn_ended(revoked)` for the open turn, and regrants per the state's default schedule. A runner crash yields no report; after a host-set timeout the moderator starts a new run; a second failure stores `status: error`.
- **Exhausted work.** `resumed` into a state whose budget is exhausted requires a prior `budget_extended`; resume never resets counters.

## Interfaces

### Agent API (MCP)

One process per role per task. The adapter injects the token; tool schemas do not expose it.

| Tool | Signature | Rules |
| --- | --- | --- |
| `post` | `(op, kind, to, body, refs?, evidence?, reply_to?, yield_to?, gate?)` | order above, R1–R15, R18 |
| `put_artifact` | `(op, kind, bytes)` | order above, R4, R11, R14–R18 |
| `read` | `(after, limit?)` | R11, R14, capability |
| `get_artifact` | `(id, range?)` | R4, R11, R14, capability |
| `task_card` | `()` | capability |

No tool approves, assigns, grants a turn, changes state or creates a human event.

### Runner API

`runner_put(candidate, run, kind, bytes, op)` from the runner principal only (R16). On `run_started`, the supervisor checks out `base` in the task's repository, applies `diffs` in order inside the sandbox, executes the frozen profile set, stores `log` then `test_report`; the moderator records `test_report_stored`.

### Human channel

Host-provided to the human principal only: approve or reject a gate for a named subject, `decision`, `ask`, `answer`, `merge_receipt`, `resume`, `budget_extended`, `cancel`. Each is verified as `kind: human` before any event is written.

### Moderator

Owns the transition table, the schedule, accounting and recovery. Never posts messages, never forwards content.

## Implementation notes

Deferred by decision. The store is any single-writer, transactional, append-only log. The core is a pure library; storage, MCP, runner supervisor and human channel are adapters. The transition table is implemented once, as data, and the same table drives the model checker in the test strategy.

### Test strategy

- One passing and at least one adversarial failing case per rule, asserting the code.
- Transition model: every (state, trigger) pair either matches a row or is rejected; no two rows match the same event; every non-terminal state has an exit. Random event sequences never leave the table.
- Properties: append-only log; a rejected write changes only `rejections`, or ends the turn or escalates exactly as R11 says; the card equals the fold of events after any crash point; spent budget never decreases or exceeds its limit without `budget_extended`.
- Visibility: for each role, no read returns a record the matrix denies.
- Gates: no Agent API path creates a human event, stores a candidate outside `build`, or stores a report.
- Binding: approval on a non-current candidate or run is rejected; a new run returns the task to `test`; a candidate whose spec is not the approved spec is rejected.
- Tokens: a write or charged read with a dead, foreign or future token is rejected; a late write after the same role's next grant is rejected.
- Replay: a retry with the same `op` after turn end, state change, cancel or close returns the original result and charges nothing.
- Proving run (below) passes end to end.

## Measurement

On the same 16 tasks, record for BBP and the relay loop: total tokens, dollars, wall-clock, task success, human interventions, rejected writes per task, iterations, turns. The task set and success rubric are published with the first run; without them the adoption criterion cannot be assessed.

## Risks

| Risk | Mitigation |
| --- | --- |
| Transition table has gaps | Model checker over the table; every reviewer-traced sequence is a test. |
| Weak models loop on rejections | Specific codes, `rejections` on the card, `turns` budget bounds the loop. |
| Reviewer isolation hides settled context | Measure first. |
| Serial turns slow work | Accepted. `yield_to` covers ask/answer. |
| Sandbox escape | Host requirement, audited; the report records the sandbox profile. |
| External wrong-revision merge | `merge_receipt` names the merged revision; a mismatch is visible, not prevented. |

## Open questions for v0.4

- **Q1.** Budget defaults after the proving run. Current card values are Gemini's starting point.
- **Q2.** Should a Reviewer `revise` skip `test` when the Coder's next candidate changes only files the Reviewer flagged? Default: no.
- **Q3.** Should `request_decision` outside a gate allow `yield_to: human` semantics, so the human can answer inline without a decision record?
- **Q4.** Appendix B evaluator classes.

## Proving run before v0.4

Deterministic Planner, Reviewer and human stubs through the normal APIs; live or scripted Coder and Tester; one runner; one moderator. No fixture bypasses an admission rule. It must demonstrate: a lost `post` response replayed safely after the turn ended; a reader crash mid-slice; a human approval against a replaced candidate rejected by R17; an approval against a superseded run rejected by R17; a late write with a dead token rejected by R14; a yield whose target changes state (T23); a decision settling a request and regranting the requester (T20); a resume into an exhausted state refused until `budget_extended`.

---

## Appendix A: Delta (non-normative)

[Delta](https://delta.dev) could host the runner's sandboxed worktree (a fresh thread per run, never a subthread) and the human channel (review verdicts mapped to human events only after the adapter verifies the author is the human principal). Neither is an adapter contract until Delta exposes an automation API; a fresh thread does not by itself provide the sandbox properties the host requirements demand. A freely writable replicated log alone cannot enforce BBP's admission rules; one transactional authority is required.

## Appendix B: Cross-vendor decisions (deferred)

Nano's rusty_orch policy (2026-10-01) lets an agent record a decision when a second model agrees. If added later: an agent posts a `propose` whose body is an immutable proposed-decision object (exact `subject` and `outcome`); a designated evaluator of a different `vendor` from both proposer and decider, able to read the proposal, posts a `finding` with `reply_to` and a typed `endorse: true` body; the agent `decision` must equal the object, ref the endorsement, and never satisfy a gate. Open: evaluator classes per decision kind, and human revocation.
