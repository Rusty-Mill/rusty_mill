# ADR-0027: On-disk schema versioning

- Status: Accepted
- Date: 2026-05-27
- Tags: data-model, versioning, storage

## Context

The system persists many on-disk formats (JSONL evidence / interventions /
entropy / security logs, JSON episode packages, TOML config, and SQLite
databases). None currently carries a version, so evolving any format risks
silently misreading old records (consolidated plan §B).

## Decision

Add a `schema_version` (or short `v`) field to every persisted record, and set
`PRAGMA user_version` on every SQLite database. Readers check the version and
migrate or reject explicitly. The authoritative versioning policy and per-format
field placement live in `docs/architecture/data-model.md`.

## Consequences

- On-disk formats can evolve with explicit migration instead of silent
  misreads.
- Every record gains a small version field; every DB carries its `user_version`.
- Readers must handle version mismatches rather than assuming the current shape.
