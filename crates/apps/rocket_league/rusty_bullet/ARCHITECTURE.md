# Architecture

This is the short version. Full systems-engineering treatment — context,
trust boundaries, principles, and the case for each boundary below — lives in
[docs/architecture/SYSTEM-ARCHITECTURE.md](./docs/architecture/SYSTEM-ARCHITECTURE.md).

## Overview

`rusty_bullet` reimplements Rocket League's client/server physics and
netcode architecture in Rust, staged as: a verification pipeline (Phase 0,
built first, everything else scores against it) → physics core (Phase 1) →
deterministic simulation (Phase 2) → netcode with client prediction/rollback
(Phase 3) → polish (Phase 4). See
[docs/roadmap/ROADMAP.md](./docs/roadmap/ROADMAP.md).

## Boundaries

Ports-and-adapters. `rb_domain` holds pure domain logic (physics frame
types, the divergence-scoring algorithm, and the `PhysicsStateSource` port);
everything that touches a file, network socket, or external process is an
adapter crate that implements a domain port.

| Port | Adapter(s) | Notes |
| ---- | ---------- | ----- |
| `PhysicsStateSource` (`rb_domain::port`) | `rb_replay_ingest` (`boxcars`-backed `.replay` parsing) | No raw controller inputs — see `RB-VERIFY-001`. |
| `PhysicsStateSource` (`rb_domain::port`) | `rb_capture_ingest` (BakkesMod offline capture parsing) | Raw inputs, offline/local play only — see `RB-VERIFY-002`. |
| *(Phase 1, not yet designed)* physics-engine port | *(build-vs-integrate open, see ADR-0003)* | The port `rb_domain` will expose so `rb_verify_cli` can score any candidate engine. |

## Structure

Modular monolith: one Cargo workspace, crates split at real responsibility
boundaries (domain vs. each ingestion adapter vs. the composition-root
binary), not split for hypothetical future services. No component has a
forcing function (independent scaling, a team boundary, hard fault
isolation) that would justify extracting a separate deployable service yet.
Revisit only if Phase 3 netcode work surfaces one (e.g. a dedicated server
process is likely there, but that's a binary target in the same workspace
until proven otherwise).

## Data flow

Phase 0 (current): a replay file or BakkesMod capture is read by its
adapter → converted into a `Vec<PhysicsFrame>` → handed to
`rb_domain::divergence::score` alongside a second trajectory (either another
recording, or eventually a candidate physics engine's output) →
`DivergenceScore` is the pipeline's output. See
[docs/specifications/verification/](./docs/specifications/verification/)
for the full spec of each stage.

## Key decisions

See [docs/adr/](./docs/adr/) for the record of individual decisions and
their tradeoffs, and
[docs/research/RESEARCH-BACKLOG.md](./docs/research/RESEARCH-BACKLOG.md) for
decisions deliberately left open (build-vs-integrate physics, binary
reverse engineering, capture-tooling scope) pending more evidence.

## Non-goals

- Not a copy or decompilation of Psyonix's client, server, or Bullet fork —
  see [SYSTEM-ARCHITECTURE.md](./docs/architecture/SYSTEM-ARCHITECTURE.md)
  for the legal/IP boundary this project holds to.
- Not aiming for feature parity with every Rocket League game mode at
  launch — car/ball physics fidelity and 1v1-scale netcode come first.
- Not solving online-specific lag empirically against live official-server
  traffic — no legitimate ground-truth source for that exists right now
  (see the research backlog); validation there is against the GDC talk's
  documented architecture instead.
