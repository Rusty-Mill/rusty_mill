# RB-PHYSICS-001 — Physics Core Port

- Version: 0.1.0
- Status: Draft (placeholder — full design deferred to Phase 1 start)
- Owners: baileyrd
- Depends on: RB-VERIFY-003
- Supersedes: none

## Purpose and scope

Define the domain-level port a candidate physics engine must implement so
`rb_verify_cli` can score it via `RB-VERIFY-003`, independent of whether
that engine is hand-rolled or an integrated third-party crate (open
question — see [ADR-0003](../../adr/0003-bullet-fidelity-target.md) and
[docs/research/RESEARCH-BACKLOG.md](../../research/RESEARCH-BACKLOG.md)).

This spec is intentionally light at bootstrap time: the port's real shape
should be informed by running the Phase 0 pipeline against at least one
candidate, not designed speculatively before that exists.

## Non-goals

- Not choosing build-vs-integrate here — that decision is explicitly
  deferred (see ADR-0003).
- Not specifying car/ball dynamics, collision response, or any specific
  physics behavior yet — that's the bulk of Phase 1's real work, to be
  spec'd once this port and the build-vs-integrate decision exist.

## Context and terminology

- **Physics core**: whatever produces a simulated `PhysicsFrame` sequence
  given initial state and a sequence of inputs — the thing `RB-VERIFY-003`
  scores.

## Requirements

- `RB-PHYSICS-001-FR-001` (open): A port that, given an initial
  `PhysicsFrame` and a sequence of per-tick inputs, produces a
  `Vec<PhysicsFrame>` the divergence scorer can consume.
- `RB-PHYSICS-001-NFR-001` (open): The port must not force a specific
  physics engine's data model into `rb_domain` — adapters translate to/from
  their own internal representations at the boundary.

## Architecture and interfaces

To be defined in `rb_domain` (or a new `rb_physics_port` module/crate if
the surface grows large enough to warrant one — not decided yet, per "no
speculative abstraction before two real call sites": there is currently
zero implementations of this port, so even the port itself is provisional
until Phase 1 starts).

## Data/state and invariants

TBD.

## Errors, failure, recovery, and observability

TBD.

## Security, privacy, and compatibility

TBD.

## Acceptance criteria

TBD — defined when Phase 1 starts, informed by whichever candidate(s) get
built/integrated first.

## Verification plan

Scored via `RB-VERIFY-003` against `RB-VERIFY-001`/`RB-VERIFY-002` ground
truth once a candidate exists.

## Traceability

See [docs/traceability/TRACEABILITY.md](../../traceability/TRACEABILITY.md).

## Open questions

- Build-vs-integrate physics engine choice (ADR-0003 / research backlog) —
  this spec's real content depends on that decision, or on scoring both
  options via this same port.
- Whether reverse-engineered physics constants (if that research question
  is ever resolved affirmatively) would feed this port's tuning, or a
  separate calibration step.

## Change history

- 0.1.0 (2026-08-28): Placeholder created at bootstrap; full spec deferred
  to Phase 1 start.
