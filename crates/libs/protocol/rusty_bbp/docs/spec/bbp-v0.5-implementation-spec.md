# Blackboard Protocol (BBP) v0.5: Implementation Spec

Oct 13, 2026 · revision of v0.4.1. **Supersedes** [`bbp-v0.4-implementation-spec.md`](bbp-v0.4-implementation-spec.md) (v0.4.1), which stays as the record of the four review rounds. Role-extensibility release: roles are data supplied when a task opens; the protocol names none.

## Summary

**Status:** v0.5.0 is the specification to implement. It changes who the protocol names, not how a task moves: every v0.4.1 rule, transition and trace carries over, with role names replaced by capability checks. A four-role roster (Appendix C) reproduces v0.4.1 behavior exactly, and the 31 traces remain the conformance tests for it.

BBP lets several AI agents collaborate on one software task through a shared store without a human or script relaying messages. It defines the records the store holds, who may write which record and when, what each role may read, how a task moves between states, and which transitions need a human. It defines no transport, discovery, authentication or encryption, and no role.

**Why it exists.** [MCP](https://modelcontextprotocol.io/specification/2025-11-25/server/resources), [A2A](https://a2a-protocol.org/latest/specification/) and [AG-UI](https://docs.ag-ui.com/concepts/architecture) cover tools, agent-to-agent tasks and agent-to-user events. None defines shared-task policy: per-role visibility, evidence rules, budgets, revision-bound approvals and human gates. BBP is that policy layer, exposed first through MCP.

**The main idea.** Agents annotate a shared store and refer to things by id. Large content is stored once as an immutable blob. Every agent mutation is authorized by an execution token bound to the inference call that started the turn. Every approval is bound to an exact revision: a spec at the plan gate, a candidate and the run selected for it at the merge gate. Every agent permission is a capability on a role, and the roles come from a roster the host hands in.

**Scope boundary.** BBP is the board interaction rules: who writes and reads, turn-taking, evidence, state transitions, human gates, budgets. Factories, rosters, goals and GUI belong to the orchestrator (orch). BBP's only job for roles is to enforce the capabilities of the roster it receives.

**Context.** Agents from several vendors work on one repository. A human approves the plan and the merge. Retries are bounded, verdicts carry evidence from a sandboxed runner, and no agent or workload holds push or deploy credentials.

### Changes from v0.4.1

- **Generic `Role`.** A role is an id plus nine capability fields (Principals and roles). No role is named in the normative text. The four v0.4.1 roles move to Appendix C as a non-normative example of an orch roster.
- **Roster injection.** The roster is supplied at `task_opened`, validated, recorded in the log and immutable for the task (Roster).
- **One requirement table.** A single state-to-required-capability table derives each state's default turn holder and validates the roster, so the two cannot disagree. A roster with no finalizer fails with its own error, `NoFinalizer`.
- **Capability checks replace role names** in R5, R6, R17, the gating table, the by-state table, T4, T9 to T11, the resume guards, the entry actions, the consultation rule and the visibility rules. The transition table's topology is unchanged.
- **`final_decision`.** At least one `approve` from a `final_decision` role is required to pass the merge gate, and a `revise` from any `final_decision` role on the same candidate and run blocks it (R17). With one final role this is v0.4.1's single-Reviewer behavior.
- **Consultation rule (derived).** The per-state consultation table is replaced by one rule, derived from the table rather than stated in v0.4.1, over `consultable`, `read_messages` and the default holder; the four-role roster yields exactly the v0.4.1 table. A human `ask` may address any assigned role in any non-terminal state; v0.4.1's engine already did this and its table understated it.
- **Log schema 2.** `task_opened` carries `schema` and `roster`. v0.4.1 logs (schema 1, four enum roles) do not replay under v0.5.0 and are archived, not migrated.
- **Moved to orch.** The Reviewer fresh-session host requirement and open question Q3 are roster policy, not protocol (Host requirements, Open questions).

## Goals, non-goals and principles

### Goals

1. Agent-to-agent collaboration without a relay.
2. Reduce repeated context transfer. Measured, not assumed.
3. Evidence-backed claims: a verdict names the selected run that judged the candidate.
4. Fresh reviews by construction: a role's read capability decides what it sees.
5. Human authority at the gates, bound to an exact revision and run.
6. Bounded work: messages, bytes, reads, turns and iterations have limits.
7. Vendor neutrality: any agent that can call an MCP tool can participate.

### Non-goals for v0.4

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
- **Make illegal states unrepresentable.** Typed bodies, a closed transition model, one role schema, one requirement table, one gating table, one consultation rule. A roster that cannot complete a task is rejected before the task opens.
- **Coherence over counts.** A rule earns its place by an adversarial test and a cited failure scenario.
- **Traces become tests.** Every failure sequence from the four review rounds is a test before the proving run.

## Principals and roles

A **principal** is anything that writes to the store. Each principal record carries `id`, `kind` (`human`, `moderator`, `runner`, `agent`) and for agents a `role` (a role id from the task's roster) and a `vendor`.

| Principal | Writes | Through |
| --- | --- | --- |
| Human | human events, `decision`, `ask`, `answer`, `brief` | Human channel |
| Moderator | events, accounting | Internal |
| Runner | `test_report`, `log`, for the selected run only | Runner API, with the run secret |
| Agent (any rostered role) | messages and artifacts its capabilities allow | Agent API, with a live token |

`human`, `moderator` and `runner` are protocol principals with fixed authority the store checks. They are not roster roles and their ids are reserved.

### Role

A **role** is an identifier plus a capability set. BBP defines the type and enforces it; it never names, defines or ships a role.

| Field | Type | Meaning |
| --- | --- | --- |
| `id` | token, 1 to 32 of `[a-z0-9_-]`, starting with a letter | Role id. Appears in `to`, `yield_to`, `from` and events. `human`, `moderator`, `runner` are reserved |
| `turn_order` | integer 0 to 65535, unique in the roster | Position when several roles qualify for one state's default turn: the lowest wins |
| `post` | set of `ask`, `answer`, `propose`, `finding`, `verdict`, `request_decision`, `pass` | Message kinds the role may post. `decision` is never postable by an agent (R7) |
| `write` | set of `spec`, `diff`, `candidate` | Artifact kinds the role may store. The state each kind is writable in is a protocol rule (gating table) |
| `read_artifacts` | set of artifact kinds | Kinds `get_artifact` may return to the role |
| `read_messages` | `all` or `scoped` | `all`: every message on the task. `scoped`: messages from `human` addressed to the role's id or `*`, and any human `decision` or `answer` whose `subject` or `reply_to` is the role's own message |
| `verdict_at` | subset of `test`, `review` | Stages the role may judge. Setting it is the role's veto: a `revise` at its stage returns the task to `build` (T10) |
| `final_decision` | boolean | The role's `approve` at `review` is what the merge gate requires (R17) |
| `consultable` | boolean | The role may receive a yielded turn |

A role with an empty `post` and `write` is valid: it reads, and holds a turn only if a human `ask` addresses it. A `scoped` reader sees no other agent's messages, so it may answer only the human.

### Requirement table

One table says what each working state needs. A role **serves** a state when it holds every capability in the state's row. The state's **default holder** is the serving role with the lowest `turn_order`. The roster is valid only if every row has a serving role (Roster).

| State | Exit command | Required capabilities |
| --- | --- | --- |
| `planning` | `request_decision(gate: "plan")` | `write` ∋ `spec`; `post` ∋ `request_decision` |
| `build` | `candidate` stored | `write` ⊇ {`diff`, `candidate`} |
| `test` | `verdict` | `verdict_at` ∋ `test`; `post` ∋ `verdict` |
| `review` | `verdict` | `verdict_at` ∋ `review`; `post` ∋ `verdict`; `final_decision` |

**The review-turn holder must hold both `verdict_at` ∋ `review` and `final_decision`.** The review turn is the only place the merge gate's `approve` can be produced, so its holder has to be able to produce it. Roles that judge `review` without `final_decision` are valid but never hold the default review turn.

### Host requirements

The store cannot verify these; they are stated so the host can be audited.

- **Sessions.** One agent session per role per task. Whether a role's session is renewed per candidate is roster policy owned by the host; BBP cannot verify it.
- **Role fixed at launch.** One Agent API process per role per task. No field an agent can set changes role, task or principal.
- **Invocation-bound tokens.** On `turn_granted` the adapter captures the token and turn id into an immutable invocation context and only then starts inference. Every tool call from that invocation carries that token, never a newer one. On `turn_ended` the adapter cancels in-flight generation and drops queued tool calls before honouring any later grant. The model never sees the token.
- **Runner is a supervisor outside the sandbox.** The supervisor holds the runner credential and the per-run secret and writes reports. The workload has no BBP, push or deploy credentials, no network egress, a read-only mount of the base, bounded output, and time and resource limits. The host contract lists the dependencies available inside the sandbox and how a running workload is cancelled.
- **Envelope limits.** `max_artifact_bytes` and `max_body_bytes` are enforced before any store work.
- **`abort_turn`.** The host may end a turn early (cause `aborted`) when an agent process dies or an inference call times out. It names the turn (`Some`: that turn only, a no-op if not live) or none (`None`: every live turn).

## Data model

Five record types. Blobs, artifacts, messages and events are append-only. The task card is a fold over events and carries a monotonically increasing `rev`.

### Blob

Content addressed by the full SHA-256 of its bytes. No metadata, no access policy, no raw read.

### Artifact record

| Field | Type | Meaning |
| --- | --- | --- |
| `id` | `art:<ulid>` | Record id |
| `task` | task id | Owning task |
| `author` | principal id | Who stored it |
| `kind` | enum | `brief`, `spec`, `diff`, `candidate`, `test_report`, `log` |
| `blob` | sha256 | Content |
| `spec` | `art:` or null | For `candidate`: the approved spec, stamped by the store |
| `candidate` | `art:` or null | For runner artifacts: the candidate judged |
| `run` | run id or null | For runner artifacts: the selected run that produced them |

A reference may carry a fragment, `art:01H...#L40-L52`, at most 128 characters, opaque to the store.

#### Gating table for artifact writes (R16)

| Kind | Content | Writer | State | Credential |
| --- | --- | --- | --- | --- |
| `brief` | Task statement, Markdown | Human | at `task_opened` | human channel |
| `spec` | Specification, Markdown; must ref the brief | role with `write` ∋ `spec` | `planning` | token |
| `diff` | Unified diff against the candidate base | role with `write` ∋ `diff` | `build` | token |
| `candidate` | Manifest below; storing it is submission | role with `write` ∋ `candidate` | `build` | token |
| `test_report` | Typed result below; one per run | Runner | `test` | run secret |
| `log` | Raw workload output; one per run | Runner | `test` | run secret |

#### Candidate manifest

```json
{
  "base": "a1b2c3d4e5f6...40 hex",
  "diffs": ["art:01HDIFF1", "art:01HDIFF2"]
}
```

The repository is the task's, fixed at `task_opened`. The store stamps the approved spec onto the record; submission without an approved spec is rejected (R17). `base` is a full commit id in that repository. `diffs` apply in index order. The task has one **current** candidate, the most recently stored. Storing one records `candidate_submitted` and stales every verdict and approval on earlier candidates.

#### Run and test report

A **run** is one supervised execution of the frozen profile set against one candidate. The moderator selects it with `run_started`, which mints the run id and a secret for the supervisor. The card shows the selected run as `pending` until its final report is stored. A run has at most one final report. A report for any run other than the selected one is rejected.

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

`status` is `passed`, `failed` or `error`. It is `passed` only if every required profile has `exit_code: 0` and `failed: 0`; the store rejects a report that says otherwise. The report, its envelope and its log must name the same candidate and run.

### Message

```json
{
  "v": 5,
  "id": 41,
  "task": "T06",
  "from": "r2",
  "to": ["r1"],
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
| `ask` | Request information | optional | no | optional | if in `post` |
| `answer` | Reply to an `ask` | optional | optional | required, names an `ask` | if in `post` |
| `propose` | Suggest a change or approach | required | optional | optional | if in `post` |
| `finding` | Defect, fact or objection | required | required | optional | if in `post` |
| `verdict` | Judge the current candidate on its selected run | required | required | no | if in `post` and `verdict_at` contains the stage |
| `request_decision` | Ask the human; `gate: "plan"` opens the plan gate; `to` includes `human` | required | optional | optional | if in `post` |
| `decision` | Record a decision | required | optional | optional | never (human only) |
| `pass` | End the turn; may `yield_to` a role | no | no | no | if in `post` |

#### Message kinds by state (R5)

| State | `ask` | `answer` | `propose` | `finding` | `verdict` | `request_decision` | `pass` |
| --- | --- | --- | --- | --- | --- | --- | --- |
| `planning` | ✓ | ✓ | ✓ | ✓ | | ✓ (gate or plain) | ✓ |
| `build` | ✓ | ✓ | ✓ | ✓ | | ✓ | ✓ |
| `test` | ✓ | ✓ | | ✓ | ✓ (stage `test`) | ✓ | ✓ |
| `review` | | ✓ (to human) | | ✓ | ✓ (stage `review`) | ✓ | ✓ |
| `plan_gate`, `merge_gate`, `approved`, `escalated` | | ✓ (to human) | | | | | ✓ |
| `closed`, `cancelled` | | | | | | | |

A consultation turn (below) further limits the holder to `ask`, `answer`, `finding`, `pass`. Human `decision`, `ask` and `answer` are accepted in any non-terminal state.

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

`subject` is the current candidate and `run` its selected run, which must have a final report. `verdict` is `approve` or `revise`. `evidence` includes that report. `approve` requires `passed`; `revise` requires at least one `blocking` item (R3).

#### Decision body

```json
{"subject": "msg:37", "outcome": "accept", "note": "..."}
```

Human-only. `subject` is the `request_decision` or `propose` settled. An `answer` from the human may settle an informational request instead; `accept` or `reject` needs a `decision`.

### Event

| Event | Writer | Payload |
| --- | --- | --- |
| `task_opened` | moderator | `schema` (2, or 3 when `max_live_turns > 1`), roster, `max_live_turns` (schema 3 only), repo, profile set and digest, budget, brief id |
| `assigned` | moderator | role, principal |
| `turn_granted` | moderator | role, turn id, kind: `default` or `consultation`, deadline, return-to role or null |
| `turn_ended` | moderator | turn id, cause: `pass`, `yield`, `verdict`, `candidate`, `request`, `deadline`, `budget`, `aborted`, `revoked` |
| `candidate_submitted` | moderator | candidate id, stamped spec id |
| `run_started` | moderator | candidate id, run id (secret delivered to the supervisor out of band) |
| `test_report_stored` | moderator | candidate id, run id, status |
| `request_opened` | moderator | request message id, gate or null, deadline or null |
| `request_settled` | moderator | request message id, by message id |
| `state_changed` | moderator | from, to, cause id |
| `human_approval` | human | gate, subject: spec id at `plan_gate`; candidate id and run id at `merge_gate` |
| `human_rejection` | human | state, subject, reason, target: `planning` or `build`, expected `rev` |
| `rerun` | human | candidate id, expected `rev` |
| `merge_receipt` | human | candidate id, merged revision |
| `cancelled` | human | reason, expected `rev` |
| `escalated` | moderator | reason, from state |
| `resumed` | human | target state, expected `rev` |
| `budget_extended` | human | field, new limit |
| `charged` | moderator | principal, turn id, messages, bytes, reads |
| `rejected` | moderator | principal, code |
| `usage` | moderator | host-reported tokens, advisory |

### Task card

```json
{
  "id": "T06",
  "rev": 212,
  "state": "test",
  "turn": {"role": "r2", "kind": "default", "deadline": "2026-10-08T16:10:00Z"},
  "iteration": 2,
  "brief": "art:01HBRIEF",
  "spec": "art:01HSPEC",
  "candidate": "art:01HCAND",
  "run": {"id": "run_03", "status": "pending", "report": null},
  "pending_request": null,
  "budget": {"messages": 40, "bytes": 5000000, "reads": 150, "reads_per_turn": 30, "turns": 60, "iterations": 4},
  "spent": {"messages": 11, "bytes": 318000, "reads": 54, "reads_this_turn": 3, "turns": 17},
  "rejections": 3,
  "advisory": {"tokens": 31800}
}
```

`rev` increments on every event. Reading the card is free and needs no token.

## Processing model

Every external command goes through four phases in order. Phases 2 to 4 run inside one transaction with phase 1's accepted command.

1. **Command validation.** Evaluation order: authenticate principal and task; `op` lookup (R15), returning the original result for an exact replay and stopping; terminal check (R18); then R1 to R17. The first failure is the rejection, recorded as `rejected` with no other effect.
2. **Transition.** The accepted command is matched against the transition table. At most one row matches. A command that matches no row is **admitted without transition** (a message is appended, a report is stored). Its turn effects still apply: `pass`, `yield_to` and a non-gate `request_decision` end the turn as the Turns section says; any other admitted message leaves the turn running.
3. **Entry actions.** Entering a state runs that state's entry action exactly once.
4. **Generated events.** Budget evaluation (R11), request deadlines, run starts and turn grants are produced here and recorded after the row's effects. A generated event may itself trigger a transition (for example `escalated`), processed as a new command in the same transaction. Each budget field is evaluated once per transaction and emits `escalated` only when it crosses from within limit to exceeded; an already-exceeded field emits nothing further, and T3 does not match in `escalated`. The operation that crosses a limit is accepted and recorded; escalation follows it. This is deliberate: rejecting it would lose the write that proved the limit was reached.

## Rules

| ID | Rule | Rejects with |
| --- | --- | --- |
| R1 | `kind` is one of the eight kinds; `v` is a known integer. | `unknown_kind`, `unknown_version` |
| R2 | `propose`, `finding`, `verdict`, `request_decision`, `decision` carry at least one `refs` entry. | `refs_required` |
| R3 | `finding` and `verdict` carry `evidence`. A `verdict` names the current candidate's selected run, which has a final report, and cites that report; `approve` requires `passed`; `revise` requires one `blocking` item. | `evidence_required`, `evidence_unbound`, `run_pending`, `not_passed`, `blocking_required` |
| R4 | Every id in `refs`, `evidence`, `reply_to`, `subject`, `candidate.diffs`, `report.candidate`, `report.log`, `spec` refs resolves to a record on the same task of the expected kind. Fragments are opaque and at most 128 characters. | `ref_unresolved`, `wrong_kind`, `fragment_too_long` |
| R5 | The author is assigned, the role's `post` contains the kind, the state accepts the kind, and a consultation turn posts only `ask`, `answer`, `finding`, `pass`. A capability rejection names the role and the missing capability. | `not_assigned`, `role_forbidden`, `wrong_state`, `consultation_only` |
| R6 | `verdict` comes from a role whose `verdict_at` contains the stage of the current state (`test` or `review`); its `subject` is the current candidate, whose stamped spec equals the approved spec, and appears in `refs`. | `role_forbidden`, `stale_candidate`, `spec_mismatch`, `subject_missing` |
| R7 | `decision` comes only from the human channel. | `decision_forbidden` |
| R8 | Prose `body`, and each `issue` and `fix`, is at most 1,200 characters. | `body_too_long` |
| R9 | `to` lists assigned roles, `human`, or `*`; `request_decision` includes `human`. | `bad_recipient`, `human_required` |
| R10 | `reply_to` names an earlier message on the task; an `answer` replies to an `ask`, or, from the human only, to an open non-gate `request_decision`, which it settles. | `bad_reply_to` |
| R11 | Accepted agent writes charge `messages` and `bytes`. `read` charges one per call; an identical retry in the same turn is free. `get_artifact` charges once per artifact id per turn. Every `turn_granted` charges `turns`. Totals never decrease within a task or within a turn. Budget evaluation is a generated event after effects (the `turns` field is the exception: it is checked in the grant that charges it): `reads_per_turn` exceeded ends the turn (`budget`); any task total exceeded records `escalated`. Human, moderator and runner operations are never charged. | `turn_budget_exhausted`, `budget_exhausted` |
| R12 | Records are never edited or deleted. | `immutable` |
| R13 | Human events arrive only through the human channel. The Agent API has no operation that creates one. | `approval_forbidden` |
| R14 | Every Agent API mutation and charged read carries a live token: issued by `turn_granted` for this task and principal, whose turn id equals the live turn, before its deadline. Runner and human principals carry no token. | `bad_token`, `token_expired`, `stale_turn` |
| R15 | Every external mutation (Agent, Runner and Human API) carries an `op` scoped to (principal, task). Same `op`, same payload: original result, no charge. Same `op`, different payload: reject. Rejected ops are not recorded. `human_rejection`, `rerun`, `resumed` and `cancelled` carry the expected card `rev` and are rejected on mismatch. | `op_conflict`, `stale_rev` |
| R16 | An artifact write matches the gating table and the envelope limit. Runner writes present the run secret, name the selected run and its candidate, and are the first final report or log for that run. A `passed` report has every required profile at `exit_code: 0`, `failed: 0`, and `profile_digest` equal to the task's. | `kind_forbidden`, `wrong_state`, `too_large`, `bad_run_secret`, `stale_run`, `duplicate_report`, `inconsistent_report`, `profile_mismatch` |
| R17 | Candidate submission requires an approved spec, stamped on the record. `human_approval` at `plan_gate` names the spec in the open gate request. At `merge_gate` it names the current candidate, whose stamped spec equals the approved spec, and its selected run; that run's final report is `passed`, an `approve` from a `final_decision` role naming the same candidate and run exists, and no `revise` from a `final_decision` role names them. | `no_approved_spec`, `stale_subject`, `spec_mismatch`, `run_pending`, `not_passed`, `not_reviewed` |
| R18 | No write is accepted in `closed` or `cancelled`, and neither can be resumed. | `terminal` |

## Reads and visibility

- `read(after, limit)` returns messages with `id > after` visible to the caller's role, at most 20, with `next` and `more`. At-least-once; clients dedupe on `id`. Charged, token required.
- `get_artifact(id, range)` returns the record and bytes within an optional range, with `more`. Charged once per artifact id per turn. Token required.
- `task_card()` is free and needs no token.

### Capability checks

Reads are filtered by the caller's role, never by its name:

- **Messages.** `read` returns the messages `read_messages` allows (Role). A message outside the filter does not appear, is not counted in `more`, and cannot be fetched by id.
- **Artifacts.** `get_artifact` of a kind outside `read_artifacts` is rejected `role_forbidden`.
- **Runner.** Reads no messages; reads `candidate` and `diff` artifacts; posts nothing.

The roster decides who is isolated. Appendix C shows a roster in which one role is `scoped` and cannot read `log`.

## Lifecycle

### States

`planning`, `plan_gate`, `build`, `test`, `review`, `merge_gate`, `approved`, `escalated`, `closed`, `cancelled`. The last two are terminal.

### Turns

- **Default turn.** Each working state (`planning`, `build`, `test`, `review`) has one default holder from the requirement table. The `test` turn is granted only when the selected run has a final report. Gate, `approved` and `escalated` states have no default turn.
- **Consultation turn.** Granted by a `yield_to` or by a human `ask` addressed to a role. The holder may post `ask`, `answer`, `finding`, `pass`. It ends on `pass`, deadline or abort.
  - **Yield rule (derived).** v0.4.1 had a per-state consultation table. This rule is derived from it, not stated in v0.4.1: with the four-role roster of Appendix C it reproduces the table exactly. A default-turn holder may yield to a role when the state is a working state, the holder's `read_messages` is `all` (a `scoped` holder could not read the answer, so its yield could never be satisfied), and the target is assigned, has `consultable` set and is not that state's default holder. In any other case `yield_to` is rejected: `wrong_state` for a non-working state, a non-consultable or default-holder target, or a holder with `scoped` reading; `bad_recipient` for an unassigned target.
  - **Human ask.** A human `ask` may address any assigned role in any non-terminal state, whatever its `consultable`. This is the only way a `scoped` reader or a role with `consultable` unset receives a consultation turn.

- **Scheduling check.** One rule decides every grant, whether from an entry action, restart recovery, a consultation return, or request settlement: no default turn is granted while a pending request is open; a consultation turn may still be granted for a human `ask`. When a request is settled, the check runs again and grants the continuation recorded with the request.
- **At most `max_live_turns` turns are live, one per role.** `task_opened` records the cap (1 to the roster size; 0 or larger is `bad_live_turns`, and an old log without the field reads as 1). At 1, exactly one turn is live, which is the behavior of every earlier version. Above 1, the state's entry action fills the free slots with the roles that serve the state, in `turn_order`; a transition (including a candidate, a verdict, a gate request) revokes every live turn; a deadline ends only its own turn; `reads_per_turn` counts per turn. R14 is unchanged per role: a role without a live turn gets `stale_turn` while others are live and `token_expired` when none is. Above 1, `turn_granted` carries a `slice` of `messages` and `bytes`: `floor((total - spent - reserved) / (cap - live))`, where `reserved` is the unspent part of the live turns' slices. A write that would pass the turn's slice is refused `turn_budget_exhausted` and ends that turn; the unspent part is free for later grants as soon as the turn ends. **Finalizers are serialized.** A role holding `final_decision` is granted a turn only when no turn is live, and while it holds one nobody else is granted; with several finalizers in a roster, `review` therefore has one live turn, as at 1, and a verdict can never revoke a second finalizer's turn. This is not an all-finalizers barrier: as at 1, only the finalizer the schedule grants is heard, and a roster must not count on a second finalizer as an independent check. **`turns` is charged and checked where a turn is minted:** the grant itself escalates the first time `spent.turns` passes `budget.turns`, whichever path reached it (a refused write, a deadline, a command refused afterwards); a grant never follows an escalation. **Slices apply only above one live turn, so the overshoot invariant depends on the mode:** at `max_live_turns > 1`, `spent.messages` and `spent.bytes` never exceed the task totals through agent writes; at 1 there is no slice, the crossing write is accepted (R11) and escalation records it. Expiry is derived from the log, not a clock: a turn's `deadline` is recorded by `turn_granted`, the fold reads no time, and a turn leaves the state only through a logged `turn_ended`, which `handle` emits for every turn whose deadline is at or before the command's `now`. A yield ends the yielder's turn (`yield`) and records a return-to role. When a consultation turn ends: if state, candidate and selected run are unchanged and no request was opened meanwhile, the return-to role gets a fresh default turn; otherwise the state's entry action decides. A human `ask` grants exactly one consultation turn; afterwards the state returns to its default (a waiting state waits).
- **Ending a turn.** `pass`, a `verdict`, a candidate submission, a `request_decision` (`request`), deadline, `abort_turn`, per-turn read exhaustion, or moderator restart (`revoked`). The token is dead afterwards (R14).
- **Requests.** A non-gate `request_decision` opens a pending request with a deadline from task configuration and ends the requester's turn. A human `decision` or `answer` naming it settles it, cancels the deadline, and regrants the requester: a default turn if it is the state's default role, else a consultation turn. A gate request has no deadline.

### Transition table

Rows match on (state, command). At most one row matches; the first listed wins where a command could match two. Effects run, then the target's entry action, then generated events.

| # | In state | Command | Guard | Effect |
| --- | --- | --- | --- | --- |
| T1 | any non-terminal | human `cancelled(rev)` | | → `cancelled` |
| T2 | any non-terminal except `planning`, `plan_gate` | human `human_rejection(target: planning, rev)` | | clear approved spec; deselect the current candidate and its run (both stay in history, neither is current); clear all candidate approvals; → `planning` |
| T3 | any non-terminal | generated `escalated` (task budget or request deadline) | | → `escalated`, recording `from` |
| T4 | `planning` | `request_decision(gate: "plan")` from a role with `write` ∋ `spec`; `refs[0]` a `spec` that refs the brief | | open gate request; → `plan_gate` |
| T5 | `plan_gate` | `human_approval(spec)` | R17 | approved spec set; → `build` |
| T6 | `plan_gate` | `human_rejection(target: planning)` | | → `planning` |
| T7 | `build` | `candidate` stored | R16, R17 | `candidate_submitted`; stale earlier verdicts; → `test` |
| T8 | `test` | `test_report_stored` final, `status: error` | selected run | `escalated(from: test)`; → `escalated` |
| T9 | `test` | `approve` at stage `test` | R3, R6 | → `review` |
| T10 | `test`, `review` | `revise` at stage `test` or `review` | R3, R6 | `iteration += 1`; → `build` |
| T11 | `review` | `approve` at stage `review` | R3, R6 | → `merge_gate` |
| T12 | `test`, `review`, `merge_gate` | human `rerun(candidate, rev)` | candidate is current | stale verdicts on it; revoke the selected run (its secret is dead, any later report is `stale_run`); select a fresh run with a new id and secret; → `test` |
| T13 | `merge_gate` | `human_approval(candidate, run)` | R17 | → `approved` |
| T14 | `merge_gate` | `human_rejection(target: build, rev)` | | `iteration += 1`; → `build` |
| T15 | `approved` | `merge_receipt` | names the approved candidate | → `closed` |
| T16 | `approved` | `human_rejection(reason: base_drift, target: build, rev)` | | clear candidate approvals, keep spec; `iteration += 1`; → `build` |
| T17 | `escalated` | human `resumed(target, rev)` | target guard below | → target |

Everything else that validates is admitted without transition: messages (T9 to T11 aside), `pass` and `yield_to`, non-gate requests, human `ask`, `answer` and `decision`, runner artifacts with `passed` or `failed` status, `budget_extended`.

#### Resume guards (T17)

| Target | Guard | Also |
| --- | --- | --- |
| `planning` | always | applies T2's clears |
| `build` | approved spec exists | |
| `test` | current candidate exists and its stamped spec equals the approved spec | entry action starts a run if none is selected or the selected run is `error` |
| `review` | an `approve` at stage `test` exists on the current candidate and selected run | |
| recorded `from` | that state's guard above; `merge_gate` additionally needs the R17 `final_decision` approval on the current candidate and run; `approved` needs the prior `human_approval` on them | |

Any exhausted budget must have a `budget_extended` before resume. Resume never resets counters.

### Entry actions

| State | Entry action |
| --- | --- |
| `planning` | grant the default holder |
| `plan_gate`, `merge_gate`, `approved`, `escalated` | none; await the human |
| `build` | grant the default holder |
| `test` | if the current candidate has no selected run, or its selected run's final report is `error`: `run_started`. A run selected by T12 or by T7 is never restarted here. Grant nobody until `test_report_stored` final arrives; then grant the default holder, subject to the scheduling check |
| `review` | grant the default holder |
| `closed`, `cancelled` | none |

### Recovery

The moderator rebuilds the card from events on restart, records `turn_ended(revoked)` for the live turn, and re-runs the current state's entry action through the scheduling check, so an open request keeps the task waiting. A pending selected run stays selected; the supervisor may store its final report after restart. If the supervisor is lost, the human issues `rerun`, which selects a new run and makes the old one `stale_run` forever. Base drift is detected by the human or merge tool at merge time and recorded as T16; a rebased candidate is a new candidate.

### Known gaps (2a)

- **A verdict is not bound to the log revision the judge read.** It binds to the candidate, the selected run and that run's report (R6, R7). A `finding` or `answer` that a concurrent turn appends after the read does not void it (the judges that can be live together are the non-final ones, for example two testers in `test`): only a transition revokes the other live turns, and a finding is not one. At `max_live_turns = 1` this cannot happen, because the judge holds the only live turn. Above 1 it can, and `known_gap_a_verdict_is_not_bound_to_the_revision_the_judge_read` pins the behavior. Owner: the repository owner. Closes with the snapshot-hash binding of Phase 2b (`docs/bbp-concurrency-design.md`, d), or earlier by a `read_rev` on `verdict` that `check_verdict` compares to the last content append.

## Roster

The host supplies the roster when it opens a task. BBP validates it, records it, and enforces it; it does not store rosters beyond the task, assemble them or choose them.

### Validation

`task_opened` is refused, and no task exists, unless all of these hold. A refused open writes nothing to the log (there is no task to record against), so the host can correct the roster and open the same task id again. Each failure is reported with the field and role involved.

1. **Ids.** Every `id` is a valid token, none is reserved, none repeats.
2. **Turn order.** Every `turn_order` is distinct, so the default holder is never a tie.
3. **Servable states.** `planning`, `build` and `test` each have a serving role: a spec writer able to open the plan gate, a candidate writer, a `test` judge. `review` has a role with `verdict_at` ∋ `review` and `post` ∋ `verdict`. A missing one fails as `Unservable` and names the state. A roster missing any of these has a state nobody can leave.
4. **Finalizer.** At least one role that meets the `review` judge condition in check 3 also has `final_decision`. A roster that passes check 3 but has none fails as `NoFinalizer`, a distinct error from `Unservable`, because the repair differs: add `final_decision` to an existing judge, not a new role.

Roles beyond those the table needs, including roles with no write capability, are valid. An error carries the check, the field and the role ids involved; the host can show it unchanged.

### Immutability

The roster is part of `task_opened` and cannot change: no operation adds, removes or edits a role, or changes `turn_order`. To change a roster, open a new task. `Assign` binds an existing roster role to one principal and is rejected `unknown_role` for any other id. A rostered role with no principal is never granted a turn.

### Log schema

Every log begins with `task_opened`, whose `schema` field is 2 for a task that allows one live turn and 3 for a task with `max_live_turns > 1`. Schema 3 is written only for such tasks, so a one-turn task's log is byte-identical to schema 2 (the new `max_live_turns` and `slice` fields are omitted at 1) and a build that reads schema 2 alone refuses a concurrent log rather than replaying it as serialized. A schema 2 log folds with a cap of 1 whatever it carries. A reader that meets an unknown `schema` refuses to replay; it never guesses. Schema 1 is v0.4.1 (roles as a fixed enumeration, no roster, no `schema` field). v0.5.0 does not replay schema 1; those logs are archived, not migrated.

### Enforcement

Every command is checked against the roster's capabilities and never against a role name. A rejection keeps its v0.4.1 code (`role_forbidden`, `kind_forbidden`, `wrong_state`, `bad_recipient`, `not_reviewed`, ...) and its detail names the role and the missing capability, for example `role r4 lacks post:verdict`.

## Interfaces

### Agent API (MCP)

One process per role per task. The adapter injects the token per invocation; tool schemas do not expose it.

| Tool | Signature | Rules |
| --- | --- | --- |
| `post` | `(op, kind, to, body, refs?, evidence?, reply_to?, yield_to?, gate?)` | validation order, R1–R15, R18 |
| `put_artifact` | `(op, kind, bytes)` | validation order, R4, R11, R14–R18 |
| `read` | `(after, limit?)` | R11, R14, capability |
| `get_artifact` | `(id, range?)` | R4, R11, R14, capability |
| `task_card` | `()` | capability |

No tool approves, assigns, grants a turn, changes state or creates a human event.

### Runner API

`runner_put(candidate, run, run_secret, kind, bytes, op)` from the runner principal only (R15, R16). On `run_started` the supervisor receives the run id and secret, checks out `base` in the task's repository, applies `diffs` in order inside the sandbox, executes the frozen profile set, stores `log` then `test_report`; the moderator records `test_report_stored`.

### Human channel

Host-provided to the human principal only, every operation carrying an `op`: `brief` at task open, approve or reject a gate for a named subject, `decision`, `ask`, `answer`, `rerun`, `merge_receipt`, `resume`, `budget_extended`, `cancel`. Destructive operations carry the expected `rev`. Each is verified as `kind: human` before any event is written.

### Moderator

Owns the processing model, the transition table, the schedule, accounting and recovery. Never posts messages, never forwards content.

## Implementation plan

The core is a pure library with no I/O: records, the rule set in validation order, the transition table as data, entry actions, generated events, and the card fold. Storage, MCP, runner supervisor and human channel are adapters behind traits. The store is any single-writer, transactional, append-only log; an embedded database is the first adapter.

### Test strategy

- **Traces first.** Every failure scenario in `REVIEW-merged.md`, `review/v0.1-merged.md`, `review/v0.2-merged.md` and `review/v0.3-merged.md` is written as a test against the core before the proving run. Each must fail against a core with the corresponding fix removed.
- **Per rule:** one passing and at least one adversarial failing case, asserting the code.
- **Model:** random command sequences never leave the table; at most one row matches any (state, command); every non-terminal state has an exit; every entry action runs exactly once per entry.
- **Properties:** append-only log; a rejected write changes only `rejections`, or ends the turn or escalates exactly as R11 says; the card equals the fold of events after any crash point; task and per-turn totals never decrease without `budget_extended`.
- **Roster:** a valid two-role, a six-role and an observer-bearing roster open; each failed check (ids, `turn_order`, each unservable state, no finalizer) is rejected with its own error; a roster cannot change after `task_opened`.
- **Yield (derived rule):** a `scoped` holder cannot yield; a non-`consultable` target, the state's default holder and an unassigned role are each rejected, and a valid yield is granted; a human `ask` reaches a non-`consultable` role.
- **Capability enforcement:** a post, artifact write, verdict or read that the role's capabilities forbid is rejected with the existing code and a detail naming the role and capability.
- **Visibility:** for each role, no read returns a record the role's capabilities deny.
- **Gates:** no Agent API path creates a human event, stores a candidate outside `build`, or stores a report.
- **Binding:** approval on a non-current candidate or non-selected run is rejected; a report for a non-selected run is rejected; a second final report for a run is rejected; a `rerun` stales verdicts; a candidate without an approved spec is rejected.
- **Tokens:** a write or charged read with a dead, foreign or stale-turn token is rejected; a late write after the same role's next grant is rejected.
- **Replay:** a retry with the same `op` and payload after turn end, state change, revision change, cancel or close returns the original result with no new effects and no charge, for agent, human and runner operations alike. Separately, a **new** `op` carrying an old expected `rev` is rejected with `stale_rev`.

### Proving run

Deterministic Planner, Reviewer and human stubs through the normal APIs; live or scripted Coder and Tester; one runner; one moderator. No fixture bypasses an admission rule. It must demonstrate: a lost `post` response replayed safely after the turn ended; a reader crash mid-slice; a human approval against a replaced candidate rejected; an approval against a superseded run rejected; a zombie run's report rejected as `stale_run`; a late write with a dead token rejected; a yield whose target changes state; a decision settling a request and regranting the requester; a resume into an exhausted state refused until `budget_extended`; a replayed human rejection returning its original result with no effect; a fresh human rejection with an old `rev` rejected.

## Measurement

On the same 16 tasks, record for BBP and the relay loop: total tokens, dollars, wall-clock, task success, human interventions, rejected writes per task, iterations, turns. The task set and success rubric are published with the first run; without them the adoption criterion cannot be assessed.

## Risks

| Risk | Mitigation |
| --- | --- |
| Transition model has gaps | Model tests; every reviewer-traced sequence is a test. |
| Weak models loop on rejections | Specific codes, `rejections` on the card, `turns` budget bounds the loop. |
| `scoped` readers miss settled context | Measure first. |
| Serial turns slow work | Accepted. Consultation turns cover ask/answer. |
| Sandbox escape | Host requirement, audited; the report records the sandbox profile. |
| External wrong-revision merge | `merge_receipt` names the merged revision; a mismatch is visible, not prevented. |

## Open questions for after the proving run

- **Q1.** Budget defaults. Record why each limit was hit before changing it; never raise a limit to make a task pass.
- **Q2.** Appendix B evaluator classes, chosen by decision subject and expertise, not vendor alone.
- **Q3 (moved to orch).** Whether a judging role's per-candidate fresh session can relax is roster policy. It is no longer a protocol question.

---

## Appendix A: Delta (non-normative)

[Delta](https://delta.dev) could host the runner's sandboxed worktree (a fresh thread per run, never a subthread) and the human channel (review verdicts mapped to human events only after the adapter verifies the author is the human principal). Neither is an adapter contract until Delta exposes an automation API; a fresh thread does not by itself provide the sandbox properties the host requirements demand. A freely writable replicated log alone cannot enforce BBP's admission rules; one transactional authority is required.

## Appendix B: Cross-vendor decisions (deferred)

Nano's rusty_orch policy (2026-10-01) lets an agent record a decision when a second model agrees. If added later: an agent posts a `propose` whose body is an immutable proposed-decision object (exact `subject` and `outcome`); a designated evaluator, chosen by decision class and of a different `vendor` from both proposer and decider, able to read the proposal, posts a `finding` with `reply_to` and a typed `endorse: true` body; the agent `decision` must equal the object, ref the endorsement, and never satisfy a gate. Open: evaluator classes, and human revocation.

## Appendix C: Example rosters (non-normative)

These are orch configurations, shown only so the capability fields have concrete values. BBP does not ship, name or prefer any of them; the orchestrator owns role definitions and per-agent rules. Field names follow the Role table.

### C.1 The four-role roster of v0.4.1

Reproduces v0.4.1 behavior; the 31 trace tests run against it.

| `id` | `turn_order` | `post` | `write` | `read_artifacts` | `read_messages` | `verdict_at` | `final_decision` | `consultable` |
| --- | --- | --- | --- | --- | --- | --- | --- | --- |
| `planner` | 0 | ask, answer, propose, finding, request_decision, pass | spec | all | all | | no | yes |
| `coder` | 1 | ask, answer, propose, finding, request_decision, pass | diff, candidate | all | all | | no | yes |
| `tester` | 2 | ask, answer, finding, verdict, request_decision, pass | | all | all | test | no | yes |
| `reviewer` | 3 | answer, finding, verdict, request_decision, pass | | brief, spec, diff, candidate, test_report | scoped | review | yes | no |

Host policy that stays outside BBP in this example: the reviewer's session is new for each candidate it judges, and only the coder keeps built-in tools.

### C.2 Two roles

| `id` | `turn_order` | `post` | `write` | `read_artifacts` | `read_messages` | `verdict_at` | `final_decision` | `consultable` |
| --- | --- | --- | --- | --- | --- | --- | --- | --- |
| `author` | 0 | all but verdict | spec, diff, candidate | all | all | | no | yes |
| `judge` | 1 | ask, answer, finding, verdict, request_decision, pass | | all | all | test, review | yes | no |

### C.3 Observer

An extra role that only watches: `post` empty, `write` empty, `read_artifacts` all, `read_messages` all, `verdict_at` empty, `final_decision` no, `consultable` no. It holds a turn only when a human `ask` addresses it.
