# Blackboard Protocol (BBP) v0.1: Draft Design

Oct 8, 2026 · revision of v0 after three independent reviews (Claude, ChatGPT, Gemini; see `REVIEW-merged.md`)

## Summary

**Status:** draft v0.1, protocol only. Nothing is built. This document defines records, rules, visibility and lifecycle. Implementation choices, including whether an existing crate hosts it, are deferred until the protocol is solid.

BBP lets several AI agents collaborate on one software task through a shared store, without a human or script relaying messages. It defines five things: the records the store holds, who may write which record, what each role may read, how a task moves between states, and which transitions need a human. It defines no transport, discovery, authentication or encryption.

**Why it exists.** The earlier loop design had a script call one agent at a time and relay each result. Existing protocols cover adjacent ground: [MCP](https://modelcontextprotocol.io/specification/2025-11-25/server/resources) offers tools, resources and subscriptions; [A2A](https://a2a-protocol.org/latest/specification/) offers tasks, artifacts and a lifecycle between agents; [AG-UI](https://docs.ag-ui.com/concepts/architecture) connects an agent to a user interface. None defines shared-task policy: per-role visibility, evidence rules, budgets, candidate-bound approvals and human gates. BBP is that policy layer. It reuses MCP as its first interface and borrows A2A's task and artifact vocabulary where it fits.

**The main idea.** Agents annotate a shared store and refer to things by id instead of restating them. Large content is stored once as an immutable blob. Messages are short and point at artifacts. Every verdict and approval is bound to one exact candidate revision.

**Context.** Claude, ChatGPT/Codex, Gemini, Ollama and Hermes agents work on one repository. A human approves the plan and the merge, retries are bounded, verdicts carry evidence, review contexts are fresh, and no agent holds push or deploy credentials.

### Changes from v0

- Added the **runner** principal, the **candidate** manifest and the **event** record (reviews: all three; ChatGPT #1, #7).
- Reads are stateless with a client-supplied offset (all three).
- Artifacts split into content-addressed **blobs** and task-scoped **artifact records** (Claude F11, ChatGPT #4).
- Budgets count what the store observes: messages, bytes and reads. Tokens are host-reported and advisory (Claude F3, ChatGPT #5).
- Fragments are opaque to the store (Claude F2, Gemini 2).
- Typed bodies for `verdict` and `decision`; R8 applies to prose fields (Claude F8, ChatGPT Q4).
- Eight kinds: `object` folded into `finding` with `reply_to` (Claude, Gemini).
- One capability matrix. Documenter, Triage and the `note` artifact kind are cut (Claude, ChatGPT).
- Turn token, attempt generation and operation id for concurrency and idempotency (ChatGPT #3, #7; Gemini 3).
- Verdicts may judge a proposal as well as a candidate, so R7's cross-vendor decision rule is representable (Claude F9).
- "No double payment" became "reduce repeated context transfer", to be measured (ChatGPT).
- Added integration edges for Delta as runner host and human channel, with the reasoning for why DeltaDB is not the enforcing store (Nano, Oct 8).

## Goals, non-goals and principles

### Goals

1. **Agent-to-agent collaboration without a relay.**
2. **Reduce repeated context transfer.** Content is stored once and referenced by id. The saving is a hypothesis until measured.
3. **Evidence-backed claims.** Findings and verdicts cite artifacts, and a verdict on a candidate cites a test report produced by the runner for that candidate.
4. **Fresh reviews by construction.** A role's read capability decides what it sees.
5. **Human authority at the gates.** Plan and merge approval are human events bound to an exact candidate.
6. **Bounded work.** Messages, bytes, reads and iterations per task have limits. Exhausting one escalates.
7. **Vendor neutrality.** Any agent that can call an MCP tool can participate.

### Non-goals for v0.1

- Transport, discovery, authentication, encryption. The host handles these.
- Free-form chat. Every message has a kind.
- Long-term memory. The store holds task history only.
- Replacing MCP, A2A or AG-UI.
- Autonomous merge or deploy. No role can push, tag or deploy. The human performs the merge after the merge gate, by hand or through a tool such as Delta's Land flow that the human invokes.

### Design principles

- **Records over conversation.**
- **Pull over push.** Agents fetch what they need. The moderator grants turns and wakes agents; it never forwards content.
- **Enforce in the store, not in the prompt.** Every rule below is checkable from data the store holds. Anything not checkable is labeled advisory.
- **Make illegal states unrepresentable.** Typed bodies, a closed state machine, a single capability matrix.
- **Small surface.** Eight kinds, five records, one state machine. v0.1 has eighteen rules; each one added since v0 cites the review finding that motivated it.
- **Protocol before implementation.** This document fixes semantics. Crate layout and reuse of existing code are decided afterwards.

## Principals and roles

A **principal** is anything that writes to the store. The host issues principal identity; agents cannot change it. Each principal record carries `id`, `kind` (`human`, `moderator`, `runner`, `agent`), and for agents a `role` and a `vendor` (for example `anthropic`, `openai`, `google`, `ollama`).

| Principal | Writes | Notes |
| --- | --- | --- |
| Human | human events, `decision`, `ask`, `answer` | Through the human channel only, never the agent API |
| Moderator | events, task card | Opens tasks, assigns roles, grants turns, runs the transition function |
| Runner | `test_report` and `log` artifacts | Applies a candidate in an isolated working tree and runs the suite. Has no read capability on messages |
| Planner, Coder, Tester, Reviewer | messages and artifacts per the capability matrix | One agent session per role per task |

The Reviewer's freshness depends on the host starting it in a session that held no other role. The store cannot verify this; it is a stated host requirement.

## Data model

Five record types. Blobs, artifacts, messages and events are append-only. The task card is derived from events and is never written directly.

### Blob

Content addressed by the full SHA-256 of its bytes. Storing the same bytes twice returns the same id. A blob has no metadata and no access policy.

### Artifact record

A task-scoped reference to a blob. Access policy applies to the record, not the blob.

| Field | Type | Meaning |
| --- | --- | --- |
| `id` | `art:<ulid>` | Record id, per task |
| `task` | task id | Owning task |
| `author` | principal id | Who stored it |
| `kind` | enum | `spec`, `diff`, `candidate`, `test_report`, `log` |
| `blob` | sha256 | Content |
| `attempt` | integer | Attempt generation at write time |

A reference may carry a fragment, `art:01H...#L40-L52`. The store treats fragments as opaque text and verifies only the record id. Clients interpret fragments.

#### Artifact kinds

| Kind | Content | Writer |
| --- | --- | --- |
| `spec` | Task specification, Markdown | Planner |
| `diff` | Unified diff against the candidate base | Coder, Tester |
| `candidate` | Manifest, JSON below | Coder |
| `test_report` | Typed result, JSON below | Runner |
| `log` | Raw runner output | Runner |

#### Candidate manifest

An immutable statement of exactly what is being judged.

```json
{
  "spec": "art:01HSPEC",
  "base": "a1b2c3d4",
  "diffs": ["art:01HDIFF1", "art:01HDIFF2"],
  "iteration": 2
}
```

`base` is the repository commit the diffs apply to. A task has at most one **current** candidate: the most recently submitted one. Submitting a new candidate makes earlier verdicts and approvals on the task stale (R18).

#### Test report

```json
{
  "candidate": "art:01HCAND",
  "status": "failed",
  "command": "cargo test --workspace",
  "exit_code": 101,
  "passed": 38,
  "failed": 2,
  "log": "art:01HLOG"
}
```

`status` is `passed`, `failed` or `error`. The store checks that `candidate` names an artifact of kind `candidate` on the same task.

### Message

```json
{
  "v": 1,
  "id": 41,
  "task": "T06",
  "attempt": 2,
  "from": "tester",
  "to": ["coder"],
  "kind": "finding",
  "reply_to": 39,
  "supersedes": null,
  "body": "AC2 has no failing case for HTTP 429.",
  "refs": ["art:01HSPEC#L12-L18", "art:01HDIFF1#L40-L52"],
  "evidence": ["art:01HREP"]
}
```

- `id` is a per-task monotonic integer assigned by the store. It is the ordering and the read offset.
- `to` holds roles assigned to the task, `human`, or `*`.
- `refs` and `evidence` hold `art:` ids or `msg:<id>` ids on the same task.
- `supersedes` names an earlier live message by the same author of the same kind. Reads return live messages by default.
- `body` is prose for most kinds and a typed object for `verdict` and `decision`.

### Message kinds

| Kind | Purpose | `refs` | `evidence` | `reply_to` |
| --- | --- | --- | --- | --- |
| `ask` | Request information | optional | no | optional |
| `answer` | Reply to an `ask` | optional | optional | required, names an `ask` |
| `propose` | Suggest a change or approach | required | optional | optional |
| `finding` | Report a defect, fact or objection | required | required | optional; with `reply_to` it is an objection |
| `verdict` | Judge a candidate or a proposal | required | required | no |
| `request_decision` | Ask the human to decide | required | optional | optional |
| `decision` | Record a decision | required | optional | optional |
| `pass` | Explicit no-op turn | no | no | no |

### Verdict body

```json
{
  "subject": "art:01HCAND",
  "verdict": "revise",
  "blocking": [
    {"id": "B1", "ref": "art:01HDIFF1#L40-L52", "issue": "no 429 case", "fix": "add retry test"}
  ],
  "non_blocking": []
}
```

`subject` is a `candidate` artifact or a `propose` message. `verdict` is `approve`, `revise` or `escalate`. Each blocking item's `ref` must resolve. When `subject` is a candidate, `evidence` must include a `test_report` whose `candidate` field names that subject (R3).

### Decision body

```json
{
  "subject": "msg:37",
  "outcome": "accept",
  "changes_spec": "art:01HSPEC#L12-L18"
}
```

`subject` is the `propose` or `request_decision` settled. `outcome` is `accept` or `reject`. `changes_spec`, when present, names the spec fragment the decision alters; the Planner then writes a new `spec` artifact.

### Event

Control records. Only the moderator and the human channel write them.

| Event | Writer | Payload |
| --- | --- | --- |
| `task_opened` | moderator | spec id, budget |
| `assigned` | moderator | role, principal |
| `turn_granted` | moderator | role, deadline |
| `attempt_started` | moderator | attempt generation |
| `attempt_cancelled` | moderator | attempt generation, reason |
| `candidate_submitted` | moderator | candidate id (on Coder's `put_artifact` of kind `candidate`) |
| `state_changed` | moderator | from, to, cause (message or event id) |
| `human_approval` | human | gate, candidate id |
| `human_rejection` | human | gate, candidate id, return state |
| `escalated` | moderator | reason |
| `resumed` | human | target state |
| `usage` | moderator | tokens reported by the host, attempt, principal (advisory) |

### Task card

Derived by folding events. Agents read it; nobody writes it.

```json
{
  "id": "T06",
  "state": "build",
  "attempt": 2,
  "turn": "coder",
  "iteration": 2,
  "candidate": "art:01HCAND",
  "budget": {"messages": 40, "bytes": 2000000, "reads": 200, "iterations": 6},
  "spent": {"messages": 11, "bytes": 318000, "reads": 54},
  "advisory": {"tokens": 31800},
  "rejections": 3
}
```

## Rules

The store checks every write and rejects violations with one error code. A rejected write changes nothing except the `rejections` counter. Error messages name what was missing so a weak model can repair and retry.

| ID | Rule | Rejects with |
| --- | --- | --- |
| R1 | `kind` is one of the eight kinds; `v` is a known integer. | `unknown_kind`, `unknown_version` |
| R2 | `propose`, `finding`, `verdict`, `request_decision`, `decision` carry at least one `refs` entry. | `refs_required` |
| R3 | `finding` and `verdict` carry at least one `evidence` entry. A `verdict` whose subject is a candidate cites a `test_report` bound to that candidate. | `evidence_required`, `evidence_unbound` |
| R4 | Every `refs`, `evidence`, `reply_to`, `supersedes` and `subject` id resolves to a record on the same task. Fragments are not checked. | `ref_unresolved` |
| R5 | The author is assigned, the role may post the kind, and the task state accepts the kind (matrix below). | `not_assigned`, `role_forbidden`, `wrong_state` |
| R6 | `verdict` comes from Tester or Reviewer, and its `refs` include its `subject`. | `role_forbidden`, `subject_missing` |
| R7 | `decision` comes from the human, or from an agent whose `refs` include an `approve` verdict on the same `subject` by an agent of a different `vendor`. A decision never changes gate state. | `decision_forbidden` |
| R8 | Prose `body`, and each `issue` and `fix` field, is at most 1,200 characters. | `body_too_long` |
| R9 | `to` lists assigned roles, `human`, or `*`. | `bad_recipient` |
| R10 | `reply_to` names an earlier message on the task; an `answer` must reply to an `ask`. | `bad_reply_to` |
| R11 | Each write and read is charged to the task budget. A write that would exceed `messages` or `bytes`, or a read beyond `reads`, is rejected and the moderator records `escalated`. | `budget_exhausted` |
| R12 | Records are never edited or deleted. `supersedes` names a live message by the same author and kind. | `immutable`, `bad_supersede` |
| R13 | Human events arrive only through the human channel. The agent API has no operation that creates one. | `approval_forbidden` |
| R14 | A message other than `pass` comes from the role holding the turn. | `not_your_turn` |
| R15 | A write's `attempt` equals the task's current attempt generation. | `stale_attempt` |
| R16 | Each write carries an `op` id. A repeat with the same `op` and payload returns the original result; the same `op` with a different payload is rejected. | `op_conflict` |
| R17 | An artifact's `kind` is one the author's role may write. | `kind_forbidden` |
| R18 | A `verdict` on a candidate, and a `human_approval`, name the task's current candidate. | `stale_candidate` |

### Notes

- **R3 and R17 together make evidence mean something.** Only the runner writes `test_report`, the report names the candidate it ran, and the verdict must cite it. An agent can still cite an irrelevant report; the Reviewer judges relevance.
- **R7 keeps the existing decision policy.** Agents propose; a decision needs the human or a second vendor's approve verdict on the same proposal. Gates are untouched by decisions.
- **R11 budgets what the store sees.** Tokens are reported by the host in `usage` events and shown as advisory. A host that meters inference may reserve tokens before granting a turn; that is outside the protocol.
- **R14 serializes writers.** The moderator grants one turn at a time; the turn ends on `pass`, on a verdict, on a candidate submission, or at the deadline. Reads are never gated by turn.
- **R16 makes retries safe.** A lost response is replayed with the same `op`.
- **R18 binds judgement to a revision.** Approving A and merging B is unrepresentable.

## Reads and visibility

### Read operations

- `read(task, after, limit, scope)` returns live messages with `id > after`, filtered by capability, at most 20, with `next` and `more`. The store keeps no cursor. `scope` is `addressed` (to the caller's role or `*`) or `all`, where the matrix permits. Superseded messages are excluded unless `include_superseded`.
- `get_artifact(id, range)` returns the artifact record and the blob bytes within an optional byte range, with `more`. Reads above a host-set size require a range.
- `task_card(task)` returns the card.

Delivery is at-least-once. Clients dedupe on `id`.

### Capability matrix

The one authoritative table. The store applies it to every read and write.

| Role | Messages readable | `scope: all` | Artifact kinds readable | Artifact kinds writable | Kinds postable |
| --- | --- | --- | --- | --- | --- |
| Planner | all on the task | yes | all | `spec` | `ask`, `answer`, `propose`, `finding`, `request_decision`, `decision`, `pass` |
| Coder | all on the task | yes | all | `diff`, `candidate` | `ask`, `answer`, `propose`, `finding`, `request_decision`, `decision`, `pass` |
| Tester | all on the task | yes | all | `diff` | `ask`, `answer`, `finding`, `verdict`, `request_decision`, `pass` |
| Reviewer | none | no | `spec`, `diff`, `candidate`, `test_report` | none | `finding`, `verdict`, `request_decision`, `pass` |
| Runner | none | no | `candidate`, `diff` | `test_report`, `log` | none |

The Reviewer begins from the task card's current `candidate`, which lists everything it may inspect.

## Task lifecycle

Eight states, two human gates, one deterministic transition function owned by the moderator.

| State | Meaning | Leaves on |
| --- | --- | --- |
| `planning` | Planner writes the spec | Planner `propose` of the spec → `plan_gate` |
| `plan_gate` | Waits for the human | `human_approval` → `build`; `human_rejection` → `planning` |
| `build` | Coder works; Tester may add test diffs | `candidate_submitted` → `test` |
| `test` | Runner produces `test_report`; Tester posts verdict | Tester `approve` → `review`; `revise` → `build`; `escalate` → `escalated` |
| `review` | Reviewer posts verdict | Reviewer `approve` → `merge_gate`; `revise` → `build`; `escalate` → `escalated` |
| `merge_gate` | Waits for the human | `human_approval` → `closed`; `human_rejection` → `build` |
| `escalated` | Waits for the human | `resumed` → named state |
| `closed` | Terminal | none |

Any return to `build` increments `iteration` and spends the iteration budget. Budget exhaustion or an unanswered `request_decision` past its deadline moves any non-terminal state to `escalated`. The human performs the merge after `merge_gate` approval; recording it is `closed`.

### State × kind matrix

Which kinds R5 accepts in each state.

| State | `ask` | `answer` | `propose` | `finding` | `verdict` | `request_decision` | `decision` | `pass` |
| --- | --- | --- | --- | --- | --- | --- | --- | --- |
| `planning` | ✓ | ✓ | ✓ | ✓ | ✓ (proposal) | ✓ | ✓ | ✓ |
| `plan_gate` | ✓ | ✓ | | | | | | ✓ |
| `build` | ✓ | ✓ | ✓ | ✓ | ✓ (proposal) | ✓ | ✓ | ✓ |
| `test` | ✓ | ✓ | | ✓ | ✓ (candidate) | ✓ | | ✓ |
| `review` | | | | ✓ | ✓ (candidate) | ✓ | | ✓ |
| `merge_gate` | ✓ | ✓ | | | | | | ✓ |
| `escalated` | ✓ | ✓ | | | | | | ✓ |
| `closed` | | | | | | | | |

### Attempts

Each turn runs under the task's current attempt generation. When a turn passes its deadline, the moderator records `attempt_cancelled` and `attempt_started` with the next generation. Writes from the old generation fail R15. The runner's working tree is per attempt.

## Interfaces

### Agent API (MCP)

One MCP server process per role per task. Role and principal are fixed at launch; there is no field for an agent to set.

| Tool | Signature | Rules applied |
| --- | --- | --- |
| `post` | `(task, kind, to, body, refs?, evidence?, reply_to?, supersedes?, op)` | R1–R16 |
| `put_artifact` | `(task, kind, bytes, op)` | R11, R15, R16, R17 |
| `read` | `(task, after, limit?, scope?, include_superseded?)` | R11, capability |
| `get_artifact` | `(id, range?)` | R11, capability |
| `task_card` | `(task)` | capability |

There is no tool to approve, assign, grant a turn or change state.

### Human channel

A separate interface the host provides to the human only: approve or reject a gate for the current candidate, post `decision`, `ask` or `answer`, resume from `escalated`, cancel a task. Its events carry the human principal.

### Runner

Triggered by `candidate_submitted`. Checks out `base`, applies `diffs` in order in an isolated tree, runs the configured command, stores `log` then `test_report`. Failure to apply is `status: error`. The runner has no message access, so nothing it reads can instruct it.

### Moderator

Holds the transition function and the turn schedule. Grants turns in a fixed order per state (`build`: Coder, then Tester if it asks; `test`: Tester; `review`: Reviewer). Records `usage` from host-reported metering when available. Never posts messages and never forwards content.

## Integration edges

BBP fixes semantics; the table says where each piece could live. Status reflects what the public docs describe as of Oct 2026.

| Edge | Role in BBP | Fit | Status |
| --- | --- | --- | --- |
| MCP server | Agent API | Direct | First interface |
| [Delta](https://delta.dev) worktrees and subagents | Runner host: isolated worktree per attempt, failed work does not merge back | Good | Delta is in public beta; no public API, so the runner would drive it through its checkout and `delta cli` |
| Delta review threads | Human channel for the merge gate: review subthread, `Approve` or `Request Changes`, verdict bound to a snapshot taken at submission | Good, close to R18 | Anyone with thread access may submit a verdict, so BBP's human channel must still assert the human principal |
| Delta comments | Human `ask` and `finding` anchored to a diff span | Good | Comments are delivered to the agent; mapping them to BBP messages is adapter work |
| DeltaDB | Store | Poor as the enforcing store, see below | Replicated CRDT, closed, no API |
| AG-UI | Live view of the task card and events | Later | Not needed while Delta is the human surface |
| A2A | Remote agents | Only if agents leave the trust domain | Not needed |

### Why DeltaDB is not the store

DeltaDB records thread messages, comments and file edits as a stream of deltas and keeps every client's replica converging through conflict-free replication. That design exists to avoid a central authority on writes. BBP needs the opposite: R11, R14, R15, R16 and R18 are check-then-append decisions that one authority makes atomically, and a rejected write must leave no trace on any replica. A CRDT cannot reject a write after another replica has accepted it. So the rules need a single serializing writer in front of any replicated log, and once that writer exists the log is a transport, not the store.

Three further gaps, from Delta's own docs: there is no public API or SDK, only `delta cli thread delete` and `purge`; every subthread, including review subthreads, inherits the parent conversation as background, which is the contamination BBP's Reviewer isolation exists to prevent; and the agentic-safety page states that agents run with no permission system, no sandbox and unrestricted device access, which conflicts with "no agent holds push credentials" unless the Delta checkout holds no `origin` credentials.

What Delta does offer is the two components v0 left undefined: the working tree, through isolated worktrees that merge back only on success, and the human review surface, through snapshot-bound verdicts. The recommended shape is BBP's store and moderator as the authority, Delta hosting the runner's worktrees and the human channel, and the BBP moderator as the only principal that writes BBP records derived from Delta events. If DeltaDB gains an API and is open-sourced as Zed has said it plans, it becomes a candidate replicated log behind the single writer, which is an adapter decision and does not change this document.

## Implementation notes

Deferred by decision. When the protocol is stable, candidates for hosting it include a fresh core or the existing `orch-core` Board, whose append-only entries, referential integrity and supersession overlap with the message and artifact records here. That choice is made against this document, not before it. Whatever the host, the core logic stays a pure library with storage, MCP, runner and human channel as adapters, and the same rule and property tests run against any store. The store is any single-writer, transactional, append-only log; an embedded database is the obvious first adapter, and a replicated log behind the moderator is a later option.

### Test strategy for whoever implements

- One passing and one failing case per rule, asserting the error code.
- Properties: append-only log; a rejected write changes only `rejections`; spent budget never decreases or exceeds its limit; the task card equals the fold of its events; no reachable state is outside the machine.
- Visibility: for each role, no read returns a message or artifact the matrix denies, including artifacts referenced from visible messages.
- Gates: no agent API call path creates a human event.
- Binding: a verdict or approval on a non-current candidate is rejected; a new candidate stales prior approvals.
- Recovery: a lost `post` response replayed with the same `op` creates one record; a reader that crashes mid-slice loses nothing.

## Measurement

The cost claim is unproven. On the same 16 tasks, record for BBP and the relay loop: total tokens, dollars, wall-clock, task success, human interventions, rejected writes per task, and iterations. BBP is adopted only if it wins on cost without losing on success.

## Risks

| Risk | Mitigation |
| --- | --- |
| Rule creep (18 rules, up from 13) | Each addition cites a review failure scenario. Remove any rule the proving run never exercises. |
| Weak models loop on rejections | Specific error codes, `rejections` on the card, moderator escalates past a cap. Fragments are unchecked so the commonest v0 loop cannot occur. |
| Reviewer isolation hides settled context | Measure first. If needed, the candidate manifest gains a `notes` field written by the Coder and flagged as untrusted, rather than a new artifact kind. |
| Turn token slows parallel work | Accepted for v0.1. Optimistic revision keys are the fallback if measurement shows idle time. |
| Host identity is trusted | Single trust domain in v0.1. One process per role removes the forgeable field. |
| Prompt injection via artifacts | Reviewer and runner read only typed or diff content; the runner reads no prose. Documented as a residual risk. |

## Open questions for v0.2

- **Q1.** Should the Tester write `diff` artifacts, or should test additions flow through the Coder?
- **Q2.** Is a proposal verdict from Tester or Reviewer the right second signature for R7, or should any assigned agent of a different vendor qualify?
- **Q3.** Default budget numbers. The card's values are placeholders.
- **Q4.** Should the Reviewer see Tester `finding` messages on the current candidate?
- **Q5.** Retention and size limits for blobs.
- **Q6.** Should `request_decision` carry its own deadline, or inherit the turn deadline?

## Proving run before v0.2

One task, Coder and Tester only, one store implementation (any embedded single-writer store; the protocol does not name one), one runner. It must survive: a lost `post` response, a reader crash mid-slice, and an approval recorded against a candidate that is then replaced. Only after that do Reviewer, Planner and the human gates join.
