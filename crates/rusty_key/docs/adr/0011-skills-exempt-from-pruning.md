# ADR-0011: Skills exempt from pruning

- Status: Accepted
- Date: 2026-05-27 (extracted from PRD 00)
- Tags: memory, skills, retention

## Context

Lessons learned from mistakes are valuable across sessions and should not be lost
to the same decay-based pruning that removes ordinary low-importance memories.

## Decision

Store lessons learned as `skill` memories that are exempt from decay-based
pruning. Importance decay still reduces their recall priority, but skills are
never deleted. Skill grooming (refine / merge / split) is the release valve that
keeps the skill set from growing without bound.

## Consequences

- Skill memories accumulate and must be managed by grooming rather than pruning.
- Recall priority of skills still decays, so stale skills sink without being
  discarded.
