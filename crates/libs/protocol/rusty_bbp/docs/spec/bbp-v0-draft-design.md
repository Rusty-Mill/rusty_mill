# Blackboard Protocol (BBP) v0: Draft Design for Review

Oct 8, 2026 · @Nano Hurtz

## Summary

**Status:** draft v0 for review. Nothing is built. The protocol has not been run against any agent.

BBP is a small protocol that lets several AI agents collaborate in one shared channel without a human or a script relaying their messages. It defines four things: the records agents store, the rules for who may post what, what each role may read, and which task transitions need a human. It defines no network transport, no discovery and no authentication.

**Why it exists.** The earlier loop design had a script call one agent at a time and relay each result, so agents never talked to each other. Existing protocols do not cover this gap: [A2A](https://a2a-protocol.org/latest/) is machine-to-machine task exchange, [AG-UI](https://docs.ag-ui.com/concepts/architecture) connects an agent to a user interface, and Zed's Agent Client Protocol connects editors to agents. None defines a shared room, per-role visibility, budgets or approval gates.

**The main idea.** Agents annotate a shared store and refer to things by id, instead of restating them in chat. Large content is stored once as an immutable artifact. Messages are short and point at artifacts. This avoids paying twice, in tokens, for the same information.

**Context.** The target environment has Claude, ChatGPT/Codex, Gemini, Ollama and Hermes agents working on software tasks in one repository. A human approves the plan and the merge, retries are bounded, verdicts must carry evidence, review contexts are fresh, and no agent holds push or deploy credentials. These requirements come from the earlier lifecycle design and carry over unchanged.

## Goals, non-goals and principles

BBP succeeds if agents from different vendors can collaborate through one store, with the safety properties enforced by the store and not by prompts.

### Goals

1. **Agent-to-agent collaboration without a relay.** Any agent can post to and read from a task's channel directly.
2. **No double payment for information.** Content is stored once and referenced by id.
3. **Evidence-backed claims.** Objections, findings and verdicts must cite references and evidence, or the store rejects them.
4. **Fresh reviews by construction.** A role's read capability decides what it can see, so a Reviewer can be denied the discussion.
5. **Human authority at the gates.** Plan approval and merge approval are events only a human principal can create.
6. **Bounded work.** Messages, tokens and iterations per task have budgets, and exhausting one escalates to a human.
7. **Vendor neutrality.** Any agent that can call a tool or a CLI can participate.

### Non-goals for v0

- Wire transport, discovery, authentication and encryption. Adapters and the host handle these.
- Free-form social chat. Every message has a kind, and the room is for work.
- Long-term agent memory. The store holds task history only.
- Replacing A2A, AG-UI, MCP or Zed's Agent Client Protocol. BBP sits beside them and maps to them at the edges.
- Autonomous deployment. No role can push, tag or deploy.

### Design principles

- **Records over conversation.** The log is structured data first and text second.
- **Pull over push.** Agents fetch what they need. The moderator does not paste transcripts into prompts.
- **Enforce in the store, not in the prompt.** A rule that depends on an agent's compliance is a suggestion.
- **Make illegal states unrepresentable.** Types and the state machine reject invalid records and transitions.
- **Small surface.** Nine message kinds, three record types, one state machine. A new rule needs a real failure that motivates it.
- **Domain without I/O.** The core logic is a pure library. Storage, MCP and UI are adapters.

## Architecture

Agents reach the store only through the MCP adapter, and the core checks every write against the same rules. The Moderator and the Human sit beside the stack and control task state.

&#91;embedded content: BBP architecture · 6 roles, 3 core layers, 2 control roles\]

The Reviewer and Triage roles see artifacts only, so their context stays fresh. Nothing in the agent-facing path can create an approval.

## Data model

Three record types cover the whole protocol: artifacts, messages and task cards. Artifacts and messages are append-only. Only the moderator changes a task card, and only through events.

### Artifact

Immutable content, addressed by a hash of its bytes. Posting the same bytes twice returns the same id.

| Field | Type | Meaning |
| --- | --- | --- |
| `id` | `art:<hash prefix>` | Content address |
| `kind` | enum | `spec`, `diff`, `test_report`, `log`, `note` |
| `task` | task id | Owning task |
| `author` | principal id | Who stored it |
| `bytes` | binary | The content |

A reference can point inside an artifact with a fragment, such as `art:9f2c#spec/AC2` or `art:51ab#diff/src/retry.py:42`.

### Message

An append-only event with a short body. Large content never goes in `body`; it goes in an artifact and is referenced.

```json
{
  "v": 0,
  "id": "m41",
  "task": "T06",
  "from": "tester",
  "to": ["coder"],
  "kind": "object",
  "reply_to": "m39",
  "body": "AC2 has no failing case for HTTP 429.",
  "refs": ["art:9f2c#spec/AC2", "art:51ab#diff/src/retry.py:42"],
  "evidence": ["art:c7d0#pytest"]
}
```

`to` holds role ids or `*` for everyone on the task. `v` is the protocol version. A store rejects any message with an unknown `v` or an unknown `kind`.

### Message kinds

| Kind | Purpose | `refs` | `evidence` | Who may post |
| --- | --- | --- | --- | --- |
| `ask` | Request information | optional | no | any assigned role |
| `answer` | Reply to an `ask` | optional | optional | any assigned role |
| `propose` | Suggest a change or approach | required | optional | any assigned role |
| `object` | Dispute a claim or proposal | required | required | any assigned role |
| `finding` | Report a defect or fact discovered | required | required | any assigned role |
| `verdict` | Judge a task output | required | required | Tester, Reviewer |
| `request_decision` | Ask the human to decide | required | optional | any assigned role |
| `decision` | Record a decision | required | optional | human, or after cross-model review |
| `pass` | Explicit no-op turn | no | no | any assigned role |

### Task card

The moderator's view of one task. Agents read it and cannot write it.

```json
{
  "id": "T06",
  "state": "build",
  "owner": "coder",
  "iteration": 2,
  "budget": {"messages": 40, "tokens": 120000, "iterations": 6},
  "spent": {"messages": 11, "tokens": 31800}
}
```

### Verdict body

The `body` of a `verdict` is a JSON string matching the earlier lifecycle design: `verdict` is `approve`, `revise` or `escalate`, with `blocking` and `non_blocking` lists. Each blocking item has `id`, `ref`, `issue` and `fix`. The store checks that every blocking item's `ref` resolves to an existing artifact.

## Rules and invariants

The store checks every write against these rules and rejects violations with a specific error code. A rejected write changes nothing. The error message names what was missing, so a weak model can fix it and retry.

| ID | Rule | Rejects with |
| --- | --- | --- |
| R1 | A message's `kind` is one of the nine known kinds, and `v` is known. | `unknown_kind`, `unknown_version` |
| R2 | `propose`, `object`, `finding`, `verdict`, `request_decision` and `decision` carry at least one `refs` entry. | `refs_required` |
| R3 | `object`, `finding` and `verdict` carry at least one `evidence` entry. | `evidence_required` |
| R4 | Every `refs` and `evidence` id resolves to an existing artifact, and its fragment exists where the artifact kind supports fragments. | `ref_unresolved` |
| R5 | The author is assigned to the task, and the task is in a state that accepts that kind. | `not_assigned`, `wrong_state` |
| R6 | `verdict` comes only from the Tester or Reviewer role. | `role_forbidden` |
| R7 | `decision` comes only from the human principal, or after a recorded cross-model review passed. | `decision_forbidden` |
| R8 | `body` is at most 1,200 characters. Longer content goes in an artifact. | `body_too_long` |
| R9 | `to` lists roles assigned to the task, or `*`. | `bad_recipient` |
| R10 | `reply_to`, when present, names an earlier message on the same task. | `bad_reply_to` |
| R11 | Each write is charged to the task's budget, and a write that would exceed a limit is rejected. | `budget_exhausted` |
| R12 | Artifacts and messages cannot be edited or deleted. | `immutable` |
| R13 | `human_approval` events come only from the human input channel, never through the agent API. | `approval_forbidden` |

### Notes on specific rules

- **R3 is the evidence rule.** Evidence is an artifact reference, such as a stored test report, not a claim in text. A verdict that says tests passed must point at the report that shows it.
- **R7 carries the decision policy.** Agents propose and may ask for a decision. They cannot record one.
- **R11 bounds retries.** Iterations, messages and tokens each have a limit per task. The moderator opens a `budget_exhausted` escalation to the human.
- **R13 keeps the human gates real.** The agent-facing API has no operation that creates an approval, so an agent cannot forge one.

## Read model and visibility

Agents pull what they need. A read returns a small, filtered slice of the log plus artifact ids. It never returns artifact contents unless the agent asks for one.

### Reads

- **Cursor.** The store keeps one cursor per agent and task. A read returns messages after the cursor that are addressed to the agent's role or to `*`, then advances the cursor.
- **Window cap.** A read returns at most 20 messages. If more are waiting, it returns a `more` flag so the agent decides whether to continue.
- **Artifacts on demand.** Messages carry artifact ids in `refs`. The agent fetches a specific artifact, or a fragment of it, only when needed.
- **Task card.** Any assigned agent can read the task card for state, owner and remaining budget.

### Per-role visibility

Each role has a capability record. The store applies it to every read, so visibility does not depend on the prompt.

| Role | Messages visible | Artifact kinds readable | Can post kinds |
| --- | --- | --- | --- |
| Planner | all on the task | all | `ask`, `answer`, `propose`, `object`, `request_decision`, `pass` |
| Coder | all on the task | all | `ask`, `answer`, `propose`, `object`, `finding`, `pass` |
| Tester | all on the task | all | `ask`, `answer`, `object`, `finding`, `verdict`, `pass` |
| Reviewer | none | `spec`, `diff`, `test_report` | `finding`, `verdict`, `request_decision`, `pass` |
| Documenter | verdicts and decisions only | `spec`, `diff`, `test_report` | `ask`, `answer`, `pass` |
| Triage | none | `log` | `finding`, `request_decision`, `pass` |

### Why the Reviewer reads no messages

A Reviewer that reads the Coder's reasoning tends to adopt it. Denying message visibility at the store gives a fresh review context by construction. The Reviewer sees the spec, the diff and the test report, and posts its own findings and verdict.

### Token cost

The design aims to reduce the cost of agents re-reading each other. Agents read only addressed messages, bodies are capped at 1,200 characters, and large content is referenced instead of repeated. The real savings are unmeasured. Measuring them is part of the build plan.

## Task lifecycle

A task moves from planning to closed through eight states and two human gates, and every loop back spends budget.

&#91;embedded content: task lifecycle · 8 states, 2 human gates, 4 rework loops\]

- **Only the moderator changes state.** Agent messages are inputs. For example, an `approve` verdict from the Tester and one from the Reviewer let the moderator advance the task to the merge gate.
- **Gates need a human.** The plan gate and merge gate each wait for a `human_approval` event, which only the human input channel can create (R13). A rejection returns the task to Planning or Build.
- **Escalation is always available.** Budget exhaustion (R11), an `escalate` verdict, or an unanswered `request_decision` moves any task to Escalated. A human decides where it resumes. How that choice is recorded is open question Q5.

## Integration edges

BBP's core is a pure library with no I/O. Everything that touches the outside world is an adapter. The first adapter is an MCP server, because it lets existing agents join without custom code.

| Edge | Role in the design | Status in v0 |
| --- | --- | --- |
| MCP server | Exposes the store to agents as tools | Planned first adapter |
| Hermes | Profiles per role call the MCP tools. Hermes [supports MCP servers](https://hermes-agent.nousresearch.com/docs/). | Planned |
| Claude Code, Codex, Gemini CLI | Join through the same MCP tools. Support for these was not verified for this draft. | To verify |
| AG-UI view | The moderator emits AG-UI events for a live human view and approval buttons. Approval would be a custom event, since the [AG-UI docs](https://docs.ag-ui.com/concepts/architecture) do not describe human approval explicitly. | Later |
| A2A bridge | Exposes an agent as an A2A server, or the moderator as an A2A client, if agents move to remote hosts. See the [A2A site](https://a2a-protocol.org/latest/). | Only if needed |
| Delta or Zed | Human review surface for the finished branch. Not integrated. | Manual use only |

### MCP tools

The MCP adapter exposes a small set of tools. Each call carries the caller's role, which the host sets and the agent cannot change.

| Tool | Purpose |
| --- | --- |
| `post` | Append a message, subject to the rules |
| `read` | Return the next filtered slice and advance the cursor |
| `get_artifact` | Return an artifact or one fragment of it |
| `put_artifact` | Store an artifact and return its id |
| `task_card` | Return the task card |

There is deliberately no tool to approve, assign, or change state. Those are moderator and human operations.

### Moderator

The moderator is a separate component. It opens tasks, assigns roles, chooses who acts next, enforces budgets, and moves tasks through the state machine. It does not summarize or forward content, and it does not post messages on behalf of agents.

### Hermes notes

In the Hermes design, each role is its own profile with its own credentials. Hermes group rooms hold 2 to 6 Bots and run at most 3 rounds per human message, so they are not used as the channel. Hermes profiles join BBP as ordinary MCP clients.

## Build plan and tests

Build a pure Rust domain crate first, test it hard, then add adapters one at a time. Nothing gets an adapter until the domain crate's tests pass.

### Crates

| Crate | Contents | I/O |
| --- | --- | --- |
| `bbp-core` | Record types, kinds, the thirteen rules, the task state machine, capability records | none |
| `bbp-store` | Append-only SQLite store implementing a storage trait from `bbp-core` | files, SQLite |
| `bbp-mcp` | MCP server over the store, with the five tools | stdio or HTTP |
| `bbp-moderator` | Opens tasks, picks the next speaker, enforces budgets, handles human gates | calls adapters |
| `bbp-agui` | Optional event emitter for a human view | HTTP |

### Order of work

1. **`bbp-core` with tests.** All types, rules R1 to R13 and the state machine, as pure functions.
2. **`bbp-store`.** SQLite persistence, content-addressed artifacts, cursors.
3. **`bbp-mcp`.** The five tools. Then a smoke test with one Hermes profile and one Claude Code session on the same task.
4. **`bbp-moderator`.** A minimal version: assign, pick the next speaker, enforce budgets, pause at gates.
5. **Measurement.** Run the 16-task evaluation from the earlier lifecycle design on BBP and compare it with the relay loop.
6. **Optional adapters.** AG-UI view, then an A2A bridge only if needed.

### Test strategy

- **Rule tests.** One passing and one failing case per rule, asserting the exact error code.
- **Property tests.** For any sequence of valid operations, the log is append-only, a rejected write leaves no trace, spent budget never decreases and never exceeds its limit, and no sequence reaches a state outside the state machine.
- **Visibility tests.** For each role, a read never returns a message or artifact its capability denies, even for artifacts referenced from a visible message.
- **Gate tests.** No call path from the agent-facing API creates a `human_approval` event.
- **Adapter contract tests.** The same suite runs against any store implementation.
- **Cost measurement.** Record tokens per task for BBP and for the relay loop on the same tasks. The claim that BBP is cheaper stays unproven until this runs.

### Code conventions

Type hints or Rust types everywhere. No `unwrap` or `expect` outside tests. Errors carry context. Dependencies stay minimal, and each new one needs a stated reason.

## Risks and open questions

The largest risk is that a custom protocol adds maintenance and delivers less than it promises. The questions below are unresolved and are where review is most useful.

### Risks

| Risk | Why it matters | Mitigation |
| --- | --- | --- |
| Rule creep | Each rule must be maintained and can block useful work | Cap v0 at the thirteen rules. Add one only after a real failure. |
| Weak models misuse the schema | Small local models may omit `refs` or `evidence` and loop on rejections | Specific error messages, a retry cap, and a rejection count in the task card |
| Pull-based reads slow progress | An agent may poll rarely or too often | The moderator wakes agents when a message addresses them |
| Savings are unproven | The cost claim rests on design intent | Measure against the relay loop before relying on it |
| Reviewer isolation hides useful context | A Reviewer with no messages may flag things the discussion already settled | Allow the Coder to attach a `note` artifact the Reviewer can read, and observe whether it undermines independence |
| Forged identity | A role claim set by the host is only as trustworthy as the host | Out of scope for v0. Run all agents in one trust domain. |
| Spec drift | Messages and SPEC.md could disagree | A `decision` must reference the spec fragment it changes, and only a human or a passed cross-model review may change the spec |

### Open questions

- **Q1. Is nine message kinds the right number?** Should `propose` and `object` merge, or should `finding` split into `defect` and `fact`?
- **Q2. Should the Reviewer see any messages?** The current design denies all. Is a curated channel, such as only `finding` messages addressed to it, better?
- **Q3. How does a cross-model review pass for R7?** Which models must agree, and who triggers it?
- **Q4. Is 1,200 characters the right body cap?** Too low forces needless artifacts, and too high invites chat.
- **Q5. Who moves the task state?** The draft makes it moderator-only. Should an approved verdict advance the task automatically?
- **Q6. How should the store prevent an agent from stuffing evidence with irrelevant artifacts?** R4 checks that references resolve, not that they support the claim.
- **Q7. Is content addressing worth its cost?** Hashing every artifact gives deduplication and tamper evidence, at some complexity.
- **Q8. What belongs in the identity model?** The draft assumes the host sets the role. Is that enough for a single-machine setup?
- **Q9. Should unknown `v` reject, or should the store accept and mark messages from a newer minor version?**
- **Q10. What is missing entirely?** Anything a real multi-agent software task needs that this draft ignores.

## Review request

Give this document and the prompt below to each reviewing agent, ideally from different providers and with different lenses. Agents cannot open a link to this document, so export it as Markdown and attach or paste the full text with the prompt.

### How to run the review

1. Export this document as Markdown.
2. Pick one lens per agent from the list below, and put it in the `LENS` line of the prompt.
3. Run each agent in a fresh session, with no other context.
4. Collect the replies, which share one format, and merge duplicate findings.
5. Bring the merged list back for a revision of this draft.

### Suggested lenses

| Lens | Focus |
| --- | --- |
| Simplicity | What can be cut? Argue against every rule, kind and crate. |
| Security and trust | Forgery, privilege escalation, prompt injection through messages and artifacts, gate bypass. |
| Distributed systems | Ordering, cursors, concurrency, idempotency, failure and restart behavior. |
| LLM practicality | Will real models use this schema reliably? Token cost, rejection loops, weak local models. |
| Rust and implementation | Type design, illegal states, crate boundaries, testability, effort. |
| Alternatives | Could A2A, MCP resources, a message queue or an existing framework replace parts of this? |

### Review prompt

```text
You are an independent design reviewer. Below is a draft design for the Blackboard Protocol (BBP), a small protocol for AI agents from different vendors to collaborate through a shared store. It is a first draft and nothing is built. Your job is to find real problems and say whether the design is worth building.

LENS: <paste one lens from the table, for example "Simplicity: what can be cut? Argue against every rule, kind and crate.">

Rules for your review:
1. Be specific and critical. Do not praise the design to be agreeable. If something is sound, say so in one line and move on.
2. Every finding must cite the section name and, where relevant, the rule id (R1 to R13) or question id (Q1 to Q10).
3. Separate blocking problems (the design fails its goals, or is unsafe) from non-blocking ones (improvements).
4. Do not invent facts about existing protocols. If you rely on a claim about A2A, AG-UI, MCP or any other system, say how confident you are and how to verify it.
5. For every problem, propose the smallest change that fixes it.
6. Stay within your lens, but flag anything serious outside it in one line.
7. Do not rewrite the document.

Answer the following, in this order:
A. Verdict: one of BUILD AS IS, BUILD WITH CHANGES, RETHINK, or DO NOT BUILD, with a two-sentence reason.
B. Strongest case against building a custom protocol, in at most five sentences, and whether it holds up.
C. Findings, each in this format:
   - id: F1, F2, ...
   - severity: blocking or non-blocking
   - section: section name and any rule or question id
   - issue: what is wrong
   - failure scenario: a concrete sequence of events that goes wrong
   - fix: the smallest change that resolves it
D. Answers to open questions Q1 to Q10: for each, a recommendation and a one-line reason. Write "no opinion" where you have none.
E. Missing pieces: anything a real multi-agent software task needs that the draft ignores.
F. What to cut: rules, kinds, crates or sections that do not earn their place.
G. Top three changes you would make first, in priority order.
H. Your confidence in this review, from low to high, and what you could not assess.

Keep the whole reply under 1,500 words.

The design document follows.

<paste the full Markdown of this document here>
```

### After the reviews

Collect the replies and look for agreement across agents first. A finding raised independently by several agents from different providers deserves priority. A finding raised by one agent needs a failure scenario you can reproduce before it changes the design. The answers to Q1 to Q10 settle the open questions for the next draft.
