# ADR-0028: H0-H3 ladder as a controlled-visibility ablation (and H0 reachability)

- Status: Proposed
- Date: 2026-05-27
- Tags: faithfulness, eval, maturity

## Context

The maturity model spans H0-H3, where H0 is "task description, repository files"
with no tool registry — the paper's ablation floor. But `RUSTYKEYS_HARNESS_LEVEL`
today accepts only `h1` / `h2` / `h3` (PRD 06), so H0 is unreachable at runtime,
and the paper's H0 ablation cannot be measured (consolidated plan §G).

The Round 3 faithfulness audit (`../review/round3-consolidated.md`, F8/F9/F10)
raises the stakes beyond H0's reachability: the paper's ladder is a
*controlled-visibility ablation*, not RK's *additive capability* gating. R1
requires that "each level exposes only the artifacts assigned to that level; lower
levels do not see higher-level artifacts" (p.7), and R5 requires that "every level
is adjudicated under the same final outcome taxonomy" (p.7, Table 5). RK gates
tools and checks additively but leaves H2 artifacts readable in the shared tree
(R1 unmet) and labels outcomes for H3 turns only (R5 unmet). So the named "is H0
selectable?" question is too narrow: freezing it alone would imply the ladder's
methodological gaps are closed when they are not. This ADR is therefore broadened
(Round 3 D4) to scope the whole controlled-visibility question.

## Decision

Scope this decision as: **does the H0-H3 ladder enforce R1 controlled visibility
and R5 all-levels adjudication?** — of which H0's runtime reachability is one
facet. The controlled-visibility build (R1 artifact-hiding + R5 evaluator-side
all-levels adjudication + per-episode isolated workspace) is specified by
ADR-0035; this ADR holds the surrounding product call. H0 runtime reachability
remains the open product decision: either make H0 a selectable harness level (a
runtime mode with no tool registry) or declare it explicitly evaluation-only.
Resolve which in `docs/dev/eval-plan.md`; if selectable, `RUSTYKEYS_HARNESS_LEVEL`
must accept `h0` and the configuration reference
(`docs/reference/configuration.md`) and the H0-H3 progression gates must reflect
it.

## Consequences

- The H0 ablation floor becomes reachable (or its eval-only status is documented),
  so H0-vs-H1+ comparisons are well-defined.
- The ladder's R1/R5 fidelity is now in scope here rather than implied closed by a
  narrow H0-reachability freeze; the build that meets R1/R5 lands in ADR-0035 and
  is gated before any Hn-vs-Hm lift is reported.
- Status is Proposed: H0 runtime reachability is the open product decision
  (consolidated plan "Open product decisions") — the owner sets whether H0 is a
  runtime mode or eval-only. The R1/R5 *build* (ADR-0035) is Accepted; what stays
  Proposed here is the surrounding product call, not the method.
- If runtime-selectable, the agent loop must run with no tool registry at H0,
  which the kernel and `Session` construction must support.
