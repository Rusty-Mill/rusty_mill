# ADR-0013: Verification carries its limits

- Status: Accepted
- Date: 2026-05-27 (extracted from PRD 00)
- Tags: compose, verification, honesty

## Context

A "verified" result can be over-read as a stronger guarantee than the checks
actually provide. The verification output should always state what it did not
verify so the verdict is never taken to mean more than it is.

## Decision

Make `VerificationReport` always include a `limits` field describing what the
checks did not verify. When the `CriteriaJudge` check is active, it upgrades
`limits` from "deterministic only" to "LLM-judge on active task criteria
included".

## Consequences

- Every verdict ships with an explicit statement of scope.
- Consumers must read `limits` alongside `verified` rather than treating
  verification as absolute.
