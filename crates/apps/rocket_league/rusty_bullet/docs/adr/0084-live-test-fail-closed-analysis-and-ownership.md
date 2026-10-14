# ADR-0084: Fail closed on live-test analysis gaps and ambiguous process ownership

- Status: Accepted
- Date: 2026-10-13
- Scope: Corrective follow-up to Rusty-Mill/rusty_mill PR #634

## Context

The source review found lifecycle claims spanning undecodable bundles and name-based cleanup
that could terminate unrelated applications. Existing Rust CI did not exercise the PowerShell
scripts. Historical live observations are author-reported, not independent validation.

## Decision

Reject the entire demolition analysis on a decode/framing error, before printing its report.
Read one u16-sized frame at a time and fold it immediately. Retain only summary transitions,
not every physics bundle. This avoids inventing an identity after a potentially lost destroy.

Cleanup uses retained native process handles. A new process name or PID after an Epic URL
activation does not prove ownership. Require a verified direct child of our still-live launch
process before stopping the bootstrap game; otherwise abort and leave it for the operator.
Do not broaden ownership to a preexisting Epic launcher or its other descendants.

Use a headless helper file for deterministic mocked orchestration tests, and require its Windows
CI job in the aggregate gate. No desktop APIs, real watcher, game or bot runs in that test job.

## Consequences

Corrupt captures produce failure, not partial lifecycle conclusions. Summary memory still grows
with transitions/maps, while framing memory is bounded by the wire's u16 payload length.
Epic delegation through an existing launcher usually cannot satisfy the ownership proof, so
safe automatic startup may be unavailable. No live startup compatibility is claimed here.
