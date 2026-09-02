# ADR-0031: Validation-gated skill promotion

- Status: Accepted
- Date: 2026-05-27
- Tags: feed, memory, skills, self-improvement

## Context

A failure-born skill (minted by the Attribution → skill loop) is, today, treated
like any other skill — importance-floored and prune-exempt (ADR-0011) from the
moment it is written. Round 2 (consolidated §ADOPT.2, Microsoft SkillOpt) flags
the risk: an unvalidated skill can entrench itself and the self-improvement loop
can **silently un-learn** good behaviour. The owner wants self-improvement that
cannot regress without evidence.

## Decision

A failure-born skill is minted as a **candidate**: `validated=false`, **no
importance floor, not prune-exempt**. It is **promoted** (gaining the
ADR-0011 prune exemption and importance floor) only when it **validates**:

- **online** — a later matching turn reaches the VERIFIED outcome; or
- **offline** — it survives golden-episode regression replay.

A human `direct_edit` of a skill **un-validates** it (back to candidate),
forcing re-validation. Skill grooming thus becomes a non-regression-gated
optimizer: the loop cannot promote, or silently retain, an unproven skill.
Detail: `docs/prd/03-feed.md`, `docs/dev/eval-plan.md`,
`docs/architecture/data-model.md` (`validated` column).

## Consequences

- The data model gains a `validated: bool` column on the skill/memory row; ADR-0011
  prune-exemption now applies only to validated skills.
- Candidate skills are prunable, so a noisy or wrong mint decays naturally
  instead of entrenching.
- Validation couples the skill loop to the eval layer (golden-episode replay)
  and to the VERIFIED outcome (ADR-0013) — both already exist as seams.
- `direct_edit`-as-un-validate keeps human edits honest: an edited skill must
  re-earn its exemption rather than inherit it.
