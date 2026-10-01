# ADR-0005: Agents never settle decisions alone

- **Status:** Accepted
- **Date:** 2026-10-01

## Context
`EntryKind::Decision` is the board's "settled choice": agents read live decisions as constraints on later work. ADR-0001 requires every task's output to be reviewed by a different agent, but nothing stopped an agent from appending a `Decision` directly, or from superseding a human's decision, with no review behind it. One over-confident local model could then rewrite the goal's constraints for every agent that followed.

The first real adapter (`orch-ollama`, ADR-0004) let Design cards emit `decision`, which made the gap concrete.

## Decision
`Board::append` accepts `EntryKind::Decision` only when:

- the author is `Author::Human`, or
- the author is `Author::Agent` and `refs` contains at least one `Ref::Entry(id)` where `id` is a live (not superseded) `Review` entry with `Verdict::Approve` **whose author differs from the decision's author**. A human's approving review counts for any agent; an agent's own approving review never counts for itself. One qualifying review is enough, however many self-approvals sit beside it.

Anything else is `BoardError::DecisionNeedsApproval`. The rule applies to supersessions as well: an agent cannot replace a decision, human or agent, without a live approval behind the replacement.

The board enforces it, not the dispatcher or any adapter, so every adapter inherits it and a bug in one cannot bypass it. `orch-ollama` no longer offers `decision` to any role; agents propose decisions as findings, and the prompt's format spec says so.

What the rule does not check: that the approving review is *about* the work the decision rests on. `Review { of: TaskId }` and `Plan` are separate aggregates (ADR-0002); that cross-check belongs to the application layer when there is a concrete need for it.

## Consequences
- A live approving review by someone else becomes the only path for an agent to settle anything, so every agent decision has two parties behind it. The board checks the author directly rather than trusting `Plan`'s no-self-review routing, since the two are separate aggregates and a review entry can reach the board without passing through a plan.
- Superseding an approving review with `ChangesRequested` retroactively blocks new decisions that cite it; decisions already on the board stay, as append-only history should.
- Out of scope, by choice: a `Proposal` entry kind, routing changes, and auto-promoting approved findings to decisions. Each waits for a real need.
