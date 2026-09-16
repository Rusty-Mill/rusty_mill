# ADR-0009: Tiered consolidation — idle / sleep / explicit

- Status: Accepted
- Date: 2026-05-27 (extracted from PRD 00)
- Tags: memory, consolidation, async

## Context

Distilling short-term memory into long-term memory should happen at more than one
tempo so it can run cheaply during normal operation and more thoroughly at
session boundaries or on demand.

## Decision

Consolidate short-term -> long-term at three tempos: micro (idle), sleep
(session end), and explicit (user command).

## Consequences

- Consolidation quality depends on an async aisdk call.
- Each consolidation has a token cost.
