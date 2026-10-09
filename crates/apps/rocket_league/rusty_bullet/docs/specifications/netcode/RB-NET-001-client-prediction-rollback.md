# RB-NET-001 — Client Prediction and Rollback Netcode

- Version: 0.1.0
- Status: Draft (placeholder — full design deferred to Phase 3 start)
- Owners: baileyrd
- Depends on: RB-SIM-001
- Supersedes: none

## Purpose and scope

Implement server-authoritative simulation with client-side prediction and
reconciliation, reproducing the architecture Psyonix has publicly disclosed
(GDC 2018, Jared Cone): both client and server run the same physics
simulation; the client predicts locally from player input for
responsiveness; the server periodically sends authoritative correction
snapshots rather than continuous full state. This is the layer this
project believes is the actual source of the owner's observed lag/
rubber-banding (reconciliation-smoothing behavior, not raw latency) — see
[docs/research/RESEARCH-BACKLOG.md](../../research/RESEARCH-BACKLOG.md).

## Non-goals

Not attempting to validate this against live official-server traffic — no
legitimate ground-truth source for that exists (see research backlog).
Validation here is against the GDC talk's documented architecture and this
project's own client/server pair, not against Psyonix's production
infrastructure.

## Context and terminology

- **Server-authoritative**: the server's simulation state is the ground
  truth; clients are corrected toward it, never the reverse.
- **Client prediction**: the client runs its own local simulation from
  player input immediately, before server confirmation, for responsiveness.
- **Reconciliation**: when a server correction snapshot disagrees with the
  client's predicted state, the client resimulates from the corrected state
  forward through its buffered unacknowledged inputs.

## Requirements

- `RB-NET-001-FR-001` (open, settled direction per ADR-0001): Server runs
  the authoritative simulation; clients predict locally and reconcile
  against periodic server snapshots.
- `RB-NET-001-FR-002` (open): Define snapshot frequency/format and the
  reconciliation/rollback algorithm — depends on RB-SIM-001's determinism
  guarantees being in place first.

## Architecture and interfaces

TBD — depends on Phase 2's deterministic simulation.

## Data/state and invariants

TBD.

## Errors, failure, recovery, and observability

TBD — this phase will need real network-condition instrumentation
(latency, packet loss simulation) to be useful for the project's original
lag-diagnosis goal.

## Security, privacy, and compatibility

Server-authoritative model exists partly for cheat resistance as a side
effect, though anti-cheat is not a primary goal of this project.

## Acceptance criteria

TBD — defined at Phase 3 start.

## Verification plan

Own client/server pair under controlled/simulated network conditions,
compared against the GDC talk's documented behavior descriptions (no live
official-server ground truth is available — see non-goals).

## Traceability

See [docs/traceability/TRACEABILITY.md](../../traceability/TRACEABILITY.md).

## Open questions

- Exact reconciliation/rollback algorithm — the GDC talk describes the
  architecture at a conceptual level, not implementation detail.
- How to simulate realistic network conditions (latency, jitter, loss) for
  testing without live official-server access.

## Change history

- 0.1.0 (2026-08-28): Placeholder created at bootstrap; full spec deferred
  to Phase 3 start.
