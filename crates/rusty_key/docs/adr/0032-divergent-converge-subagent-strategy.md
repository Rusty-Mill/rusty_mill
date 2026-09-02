# ADR-0032: Divergent→converge subagent strategy

- Status: Accepted
- Date: 2026-05-27
- Tags: feed, subagents, planning, cost

## Context

PRD 03/06 give us an `agent` tool and a `SessionFactory` (ADR-0017) for spawning
isolated child Sessions. Round 2 (consolidated §ADAPT.3) drew an ADHD-style
divergent-ideation pattern (MIT-licensed — pattern only, no code) onto this
existing infrastructure: for hard, open-ended problems, breadth of independent
exploration beats a single linear plan, and child-Session isolation already
gives "no cross-branch context" for free.

## Decision

Add an **opt-in divergent→converge subagent strategy** built on the *existing*
`agent` tool + `SessionFactory` — **no new infra**. Fan out **N isolated child
Sessions** under distinct **cognitive-frame identity preambles** (the divergent
pass), then run a **mechanical critic/converge pass**: score → cluster → deepen
the top-K. It is offered as a **plan-mode "explore" option**, not a default, and
is **cost-gated** (≈5–10× a normal turn). Detail: `docs/prd/03-feed.md`.

## Consequences

- The strategy is purely additive: it composes the `agent` tool and
  `SessionFactory` (ADR-0017) without new runtime components.
- Child-Session isolation supplies divergence cheaply; the converge pass is
  mechanical (score/cluster/deepen), keeping the cost predictable per branch.
- The ≈5–10× cost makes it opt-in and plan-mode-gated; branch count and cost cap
  remain product-tunable thresholds, not constants fixed here.
- Cognitive-frame preambles are the only per-branch variation, so the pattern
  stays within the existing identity-preamble mechanism.
