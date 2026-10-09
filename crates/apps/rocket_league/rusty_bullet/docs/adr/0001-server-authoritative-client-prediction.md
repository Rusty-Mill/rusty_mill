# ADR-0001: Server-authoritative simulation with client-side prediction

- Status: Accepted
- Date: 2026-08-28
- Deciders: baileyrd
- Related: RB-NET-001, CHARTER.md, SYSTEM-ARCHITECTURE.md
- Supersedes/Superseded by: none

## Context

The project needs a netcode architecture for Phase 3. Two things anchor
this decision: Psyonix has publicly disclosed their own architecture (Jared
Cone, GDC 2018, "It IS Rocket Science! The Physics and Networking of Rocket
League"), and one of this project's two purposes is to diagnose the exact
lag/rubber-banding behavior the owner experiences in the live game.

The GDC talk states plainly: client and server both run the same physics
simulation; clients predict locally from input so play feels instant;
network traffic is periodic authoritative-snapshot corrections, not
continuous full state; visible lag/rubber-banding is almost certainly
reconciliation-smoothing behavior, not raw network latency.

## Decision drivers

- Reproducing Psyonix's actual disclosed architecture is a stated goal
  (deconstruct the real system, don't guess at a plausible-sounding
  alternative).
- No legitimate ground-truth data source exists for validating netcode
  behavior against live official-server traffic (see the research
  backlog) — so the design has to be evidence-anchored on the *disclosed*
  architecture rather than reverse-engineered from network captures.
- Server-authoritative models are also the standard approach for
  competitive-integrity multiplayer games generally, independent of the
  Rocket League-specific evidence.

## Considered options

1. **Server-authoritative, client-predicted** (the disclosed Psyonix
   model). Client runs local prediction for responsiveness; server is
   ground truth; periodic correction snapshots reconcile the two.
2. **Client-authoritative / peer-to-peer**, each client trusts its own or a
   peer's simulation. Simpler to implement, no server process needed, but
   diverges from the actual system this project exists to understand, and
   is more exploitable.
3. **Lockstep** (deterministic simulation, all clients simulate identically
   from synchronized input, no server correction). Requires very tight
   determinism guarantees and doesn't match the disclosed architecture
   (which explicitly uses correction snapshots, implying clients aren't
   expected to stay in perfect lockstep).

## Decision

Adopt option 1: server-authoritative simulation with client-side
prediction and periodic correction snapshots, matching Psyonix's disclosed
design.

## Consequences

### Positive

- Directly serves the lag-diagnosis goal: this project's own
  implementation of the documented model can be inspected and instrumented
  in ways the closed live game cannot.
- Matches the preservation goal's requirement (online-style play with
  friends) without inventing an architecture that might behave
  differently from what players actually experience today.

### Negative / tradeoffs

- Requires a real server process/binary, correction-snapshot protocol
  design, and (per RB-SIM-001) deterministic simulation on both ends —
  meaningfully more work than a simpler peer-to-peer model.
- This project cannot validate its reconciliation behavior against live
  official-server traffic (no legitimate data source), so confidence in
  "this matches the real game" rests on the GDC talk's description plus
  this project's own client/server pair, not on direct comparison.

## Validation and revisit triggers

Revisit if: Phase 3 implementation reveals the GDC talk's description is
too coarse to implement directly (in which case, document the gap and the
engineering judgment call made to fill it, rather than silently deviating).
Not revisited due to implementation difficulty alone — the difficulty was
already anticipated as a driver of the multi-month Phase 3 estimate.
