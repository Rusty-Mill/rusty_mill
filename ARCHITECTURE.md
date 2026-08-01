# Architecture

## Overview
`rusty_stream` is a single-node, append-only durable log: segment files plus a
sparse offset index (Kafka's `.log`/`.index` model), extending `rusty_wire` for the
wire protocol rather than starting a new one. Phase 1 is deliberately scoped to one
node with no replication — see [docs/phase1-scope.md](./docs/phase1-scope.md) for
the full research brief and the differentiators this is meant to pursue instead of
just re-deriving Kafka (sovereignty-first deployment, DST-first testing, a
deliberate rather than reflexive consensus choice for Phase 2).

## Boundaries
<!-- Domain logic vs. I/O and framework details (ports-and-adapters).
     List the ports (interfaces) and the adapters that implement them.
     Nothing real to list yet — no code has landed. Fill in once the storage
     engine and wire-protocol layer exist. -->

| Port | Adapter(s) | Notes |
| ---- | ---------- | ----- |
|      |            |       |

## Structure
<!-- Greenfield default (see references/scan-and-defaults.md): modular monolith,
     composition over inheritance, ports-and-adapters keeping domain logic free of
     I/O and framework details. A component gets split into its own service only for
     a concrete forcing function — independent scaling, a team/language boundary, or
     hard fault isolation. Note here if/why this repo has already crossed that line. -->

## Data flow
<!-- Diagram or short walkthrough of a request/event through the system -->

## Key decisions
See [docs/adr/](./docs/adr/) for the record of individual decisions and their tradeoffs.

## Non-goals
Phase 1 explicitly excludes (see docs/phase1-scope.md §2):
- Multi-broker replication / clustering
- Kafka wire-protocol compatibility layer
- WASM-based stream transforms
- Consumer group rebalancing

These are Phase 2+ candidates, gated on an actual forcing function rather than
built speculatively.
