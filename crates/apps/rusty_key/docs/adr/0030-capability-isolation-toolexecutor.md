# ADR-0030: Capability isolation via a `ToolExecutor` seam + `RUSTYKEYS_ISOLATION`

- Status: Accepted
- Date: 2026-05-27
- Tags: constrain, isolation, security, roadmap

## Context

Today the constrain layer is bash-pattern checkers + a canonicalized workspace
boundary + redaction + egress rules — all *in-process* and bypassable if a
checker misses a case (the threat-model's §residual-risk admits this). Round 2
(consolidated §scope-expansion) surfaced Anthropic's central lesson: **supervise
what the agent *can* do, not only what it does.** This is the one genuine scope
expansion of the round, so it is sequenced rather than baked in. The owner
decision (LOCKED) is: adopt it as a **roadmap phase**, opt-in, default off.

## Decision

Introduce a **`ToolExecutor` isolation seam** selected by a
**`RUSTYKEYS_ISOLATION=none|sandboxed`** runtime profile. The seam sits **below
`feed`, above the OS**, and does **not** change the `constrain` vetting
contract — vetting still happens first; isolation governs *how* a vetted
side-effect runs.

- `none` (default) — today's in-process behaviour; the local-first,
  sub-millisecond hot path is untouched.
- `sandboxed` — tool side-effects (esp. `bash`) run inside an OS sandbox
  (Linux-first: landlock/namespaces, or a gVisor-class target) with
  **network-deny-by-default** and egress enforced at the sandbox boundary.

Per "be wary of custom components," the sandbox **wraps battle-tested
primitives** rather than hand-rolling isolation. This is sequenced as a roadmap
phase. Detail: `docs/ARCHITECTURE.md`, `docs/architecture/threat-model.md`,
`BACKLOG.md`.

## Consequences

- A new isolation seam is added below `feed`; the `constrain` vetting contract
  (ADR-0007, ADR-0016) is explicitly unchanged, so the two layers compose.
- The default `none` preserves backward behaviour and performance; `sandboxed`
  is opt-in and matches isolation strength to the user's capacity for oversight.
- `sandboxed` is Linux-first; other platforms inherit `none` until a target
  lands — recorded as a roadmap/backlog sequencing item, not a v1 blocker.
- Egress moves from in-process rules to the sandbox boundary under `sandboxed`,
  hardening the exfiltration residual-risk the threat-model flags.
