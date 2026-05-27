# ADR-0035: Controlled-visibility ablation as the eval substrate

- Status: Accepted
- Date: 2026-05-27
- Tags: faithfulness, eval, maturity, integrity

## Context

The H0-H3 ladder is the paper's central methodological instrument: its claim is
that the *delta* between levels isolates the contribution of each harness layer
(separability, p.2). The Round 3 audit
(`../review/round3-consolidated.md`, F8/F9/F10/F26) found RK builds the ladder as
*additive capability* — each level gets more tools/checks — rather than the
paper's *controlled-visibility ablation*. Three things are unbuilt:

- **R1 controlled visibility** (p.7): "each level exposes only the artifacts
  assigned to that level; lower levels do not see higher-level artifacts." RK
  gates *authority* (which tools dispatch) but leaves H2 artifacts — memory,
  `AGENT_GUIDE`, `TASK_STATE`, `checks.toml` — readable in the shared tree, so the
  H1-vs-H2 contrast is confounded.
- **R5 outcome comparability** (p.7, Table 5): "every level is adjudicated under
  the same final outcome taxonomy." Table 5 labels **H0** as
  `autonomous_verified_success` via "evaluator-side deterministic checks pass;
  full regression succeeds" — with no agent report at all. RK classifies "every
  H3 turn" only and from the agent's *self-produced* report
  (`VerificationReportRequired`), so the headline H0-verified-vs-H1-unverified
  contrast is unreproducible.
- **R2 / Methods substrate isolation** (p.7/p.14): a per-episode isolated
  workspace at a fixed commit. RK shares one workspace/DB/`task.json`;
  `initial_state` is recorded but enforced by nothing — which is exactly where the
  H2 artifacts leak (compounding R1).

The paper supplies its own reconciliation of the apparent tension in checks
(p.10): "Deterministic checks serve two roles: at H3 they are agent-visible
harness artifacts that support the agent's own verification; at all levels they
are evaluator-side adjudication checks that classify the final outcome." Making
the evaluator run the checks at every level is therefore *faithful restoration*,
not a new feature.

## Decision

Build the H0-H3 ladder as a **true controlled-visibility ablation** in the eval
substrate, sequenced isolation → visibility → adjudication:

- **(a) R1 artifact-hiding at the feed/context-read seam.** Lower levels do not
  see the *existence* of higher-level artifacts (H2 memory, `AGENT_GUIDE`,
  `TASK_STATE`, `checks.toml`). This is enforced at the feed/context-read seam —
  **not** in `constrain`, which gates *authority* not *existence*. Withholding
  existence is what makes the inter-level contrast unconfounded.
- **(b) R5 evaluator-side deterministic checks at all levels.** A
  `CheckRegistry::run_all()` evaluator pass runs at **every** level (H0-H3) and
  assigns the `EpisodeOutcome` label — not only at H3, and not from the agent's
  self-report. At H3 the same checks remain agent-visible harness artifacts; the
  two roles coexist exactly as the paper specifies (p.10).
- **(c) Per-episode isolated workspace at a fixed commit.** Each eval episode runs
  in its own fresh workspace checked out at a fixed commit, with its own
  `.rustykeys/`, so `initial_state` is *enforced* and the shared-tree artifact
  leak that confounds R1 cannot occur.

This lands in the golden-episode replay (a task-grained context where
`checks.toml` is meaningful), not necessarily the live per-turn hot path. It is
**gated before reporting any Hn-vs-Hm lift** as evidence: an additive-capability
ladder may not be presented as an ablation result. Detail:
`docs/dev/eval-plan.md`, `docs/ARCHITECTURE.md` §3/§12, `BACKLOG.md`.

## Consequences

- RK becomes a true ablation instrument rather than a capability-gating ladder;
  the paper's separability claim (p.2) is reproducible because each inter-level
  contrast varies only the artifacts assigned to that level.
- R1 hiding lives at the feed/context-read seam, so the `constrain` vetting
  contract (ADR-0007/0016/0030) is untouched — authority gating and existence
  hiding are separate concerns that compose.
- `CheckRegistry::run_all()` (PRD 05) gains an all-levels evaluator role; outcome
  labels are assigned for H0-H2 as well as H3, so "agent produced evidence" is no
  longer conflated with "evaluator verified behaviour."
- The per-episode workspace makes the substrate reproducible and removes the
  cross-episode contamination path; it complements the eval-integrity guard
  (ADR-0033) — both defend against verified-success that is not earned.
- This is sequenced as an eval-plan roadmap workstream; it does not change the
  live per-turn adjudication (PRD 05's per-turn gating, F5, stays faithful) and is
  distinct from the episode-package assembly projector (ADR-0036).
- It closes the methodological half of the ladder question that ADR-0028
  (broadened) keeps open on the H0-reachability product call.