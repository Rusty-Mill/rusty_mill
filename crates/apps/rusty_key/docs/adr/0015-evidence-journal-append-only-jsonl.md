# ADR-0015: Evidence journal is append-only JSONL

- Status: Accepted
- Date: 2026-05-27 (extracted from PRD 00)
- Tags: compose, storage, audit

## Context

Task completion should be an auditable record rather than a bare claim. Every
turn's verification package and every consolidation changelog needs a durable,
tamper-evident home.

## Decision

Append every turn's verification package and every consolidation changelog to
`.rustykeys/evidence.jsonl`. Completion is an auditable record, not a claim.

## Consequences

- The evidence log grows unbounded; rotation / retention is a future seam.
- The append-only format makes the record auditable and replayable.

> Note: ADR-0027 adds `schema_version` / `v` to every record so the on-disk
> format can evolve.
