# ADR-0021: Fixed `FailureType` taxonomy

- Status: Accepted
- Date: 2026-05-27
- Tags: compose, attribution, faithfulness

## Context

PRD 05's failure attribution uses free-string `failure_type` and `layer` fields
(for example `"validation_missing"` / `"validator"` in the episode package).
Free strings cannot be aggregated reliably across episodes, and they drift from
the AI Harness Engineering paper, which defines a fixed eight-type failure
taxonomy (consolidated plan §A.5, §G).

## Decision

Replace the free-string `failure_type` / `layer` with a fixed `FailureType` enum
matching the paper's eight types: `F_context`, `F_tool`, `F_feedback`,
`F_verify`, `F_recovery`, `F_entropy`, `F_model`, `F_unknown`. The enum and its
serde encoding (snake_case per ADR-0025) live in `docs/architecture/data-model.md`;
the attribution mapping lives in `docs/prd/05-compose.md`.

## Consequences

- Attribution becomes aggregatable and directly comparable to the paper's metric
  family.
- The existing `(category, layer)` attribution matrix must be frozen and mapped
  onto the eight `FailureType` variants.
- Adding a failure type is an explicit, exhaustively-checked enum change rather
  than a new ad-hoc string.
