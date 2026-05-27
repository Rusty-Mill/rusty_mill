# ADR-0025: Serde wire convention — `rename_all = "snake_case"` for on-disk/wire enums

- Status: Accepted
- Date: 2026-05-27
- Tags: serde, data-model, standards

## Context

Across the PRDs the serde encodings are inconsistent: PascalCase, snake_case, and
lowercase are mixed across `ToolStatus`, `EpisodeOutcome`, `InterventionKind`,
`EntropyCategory`, `CompactionTier`, and `FailureType`. Inconsistent encodings
make on-disk records and wire payloads ambiguous and fragile to evolve
(consolidated plan §A.5).

## Decision

Apply `#[serde(rename_all = "snake_case")]` to all on-disk and wire enums. The
authoritative list of affected enums and their encodings lives in
`docs/architecture/data-model.md`, which owns serde conventions.

## Consequences

- All JSONL / JSON / TOML records and IPC/HTTP payloads use a single, predictable
  enum encoding (for example `autonomous_verified_success`, `tool_error`).
- Existing schema snippets in the PRDs that show PascalCase variants must be
  updated to match.
- The convention is checked at the data-model SSOT rather than re-stated per PRD.
