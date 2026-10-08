# System Architecture — rusty_bullet

- Status: Accepted (Phase 0 detail); Draft (Phase 1-3 detail, expanded as
  each phase starts)
- Related: [CHARTER.md](./CHARTER.md), [ADR-0001](../adr/0001-server-authoritative-client-prediction.md),
  [ADR-0002](../adr/0002-verification-first-ordering.md),
  [ADR-0003](../adr/0003-bullet-fidelity-target.md)

## System context

```
                 ┌─────────────────────────┐
                 │   Ground-truth sources    │
                 │  (owner's own match data) │
                 ├───────────────┬───────────┤
                 │ .replay files │ BakkesMod  │
                 │ (boxcars)     │ captures   │
                 └───────┬───────┴─────┬──────┘
                         │             │
                         ▼             ▼
                 ┌─────────────────────────────┐
                 │      Verification pipeline     │   Phase 0
                 │  (rb_replay_ingest,             │
                 │   rb_capture_ingest,            │
                 │   rb_domain::divergence)        │
                 └───────────────┬─────────────────┘
                                 │ divergence score
                                 ▼
                 ┌─────────────────────────────┐
                 │        Physics core             │   Phase 1
                 │  (build-vs-integrate: open,     │
                 │   see ADR-0003)                 │
                 └───────────────┬─────────────────┘
                                 ▼
                 ┌─────────────────────────────┐
                 │   Deterministic simulation      │   Phase 2
                 └───────────────┬─────────────────┘
                                 ▼
                 ┌─────────────────────────────┐
                 │  Netcode: server-authoritative, │   Phase 3
                 │  client-predicted (ADR-0001)    │
                 └─────────────────────────────┘
```

Every later phase's implementation is evaluated by feeding it back through
the Phase 0 pipeline: run the candidate against recorded inputs, score its
output against the recorded outcome. The divergence score is the project's
one accuracy metric (see `RB-VERIFY-003`).

## Architectural principles

- **Rust**, primary and (initially) only language.
- **Composition over inheritance.**
- **Modular monolith by default.** A component is split into its own
  service/process only for a real forcing function (independent scaling, a
  team/language boundary, hard fault isolation) — not speculatively. A
  dedicated server binary is likely once Phase 3 netcode needs to run
  headless, but that is a second binary target in the same workspace, not a
  separately deployed service, until something concrete says otherwise.
- **Ports-and-adapters.** Domain logic (`rb_domain`: physics frame types,
  divergence scoring, and eventually the physics/simulation core itself)
  stays free of file I/O, network I/O, and third-party-format parsing.
  Adapters (`rb_replay_ingest`, `rb_capture_ingest`, and future
  netcode/transport adapters) implement domain-defined ports.
- **`Result` + `?`** over panics/`unwrap` outside tests. Enforced as a
  workspace clippy lint (`unwrap_used`/`expect_used`/`panic` = warn) —
  see [AGENTS.md](../../AGENTS.md).
- **Minimal dependencies**, one-line justification per addition. `rb_domain`
  has zero third-party dependencies as of Phase 0 bootstrap.
- **No speculative abstraction before two real call sites.** The
  `PhysicsStateSource` port exists because two adapters
  (`rb_replay_ingest`, `rb_capture_ingest`) need it today.

## Capabilities / domains

| Domain | Phase | Responsibility | Status |
| --- | --- | --- | --- |
| Verification | 0 | Ingest ground-truth physics data (replay + BakkesMod capture), score any candidate implementation's divergence from it. | In progress (bootstrap) |
| Physics core | 1 | Ball/car rigid-body dynamics, collision, suspension, boost, aerial control — reproducing Rocket League's Bullet-derived car feel. | Not started |
| Deterministic simulation | 2 | Make the physics core step identically given identical inputs, across runs and (eventually) across machines — a prerequisite for any rollback/prediction netcode. | Not started |
| Netcode | 3 | Server-authoritative simulation, client-side prediction, reconciliation/correction — per the GDC 2018 talk's documented design. | Not started |
| Polish | 4 | Whatever remains once the above is playable: UX, content, tooling. Deliberately open-ended and not spec'd in detail yet. | Not started |

## Trust boundaries

- **Verification pipeline (Phase 0):** runs entirely offline, over the
  owner's own recorded data. No network trust boundary yet.
- **Netcode (Phase 3, forward-looking):** server is the trust boundary —
  authoritative over game state, per Psyonix's own documented model (GDC
  2018 talk: server runs the same physics sim as clients and corrects them
  periodically). Clients are never trusted for final state, only for input
  and local prediction. This is a settled design decision — see
  [ADR-0001](../adr/0001-server-authoritative-client-prediction.md).

## Runtime topology (forward-looking, Phase 3)

Per the GDC talk: client and server both run the same physics simulation.
Clients predict locally from player input for responsiveness; the server
runs the authoritative simulation and periodically sends correction
snapshots, not continuous full state. Visible rubber-banding/lag in the
real game is understood to be reconciliation-smoothing behavior, not raw
network latency — this project's Phase 3 netcode is built to reproduce that
model so it can be inspected directly, rather than reverse-engineered
behaviorally from the outside.

## Legal and IP boundary

This project does not use, copy, or decompile any Psyonix code, Unreal
Engine 3 code, Bullet Physics fork code, or Rocket League game assets. All
implementation code is written from scratch in Rust, informed by:

- Psyonix's own public technical disclosure (the GDC 2018 talk and slides).
- Physics/gameplay behavior observed via legitimate means (the owner's own
  replay files and BakkesMod captures of their own local play).
- Publicly available information about Bullet Physics (a separate,
  independently zlib-licensed open-source project Rocket League happens to
  use — no code from Psyonix's fork of it is used or has been obtained).

Binary reverse engineering of the shipped Rocket League client (e.g. to
recover exact physics constants) is **not currently in scope** and is
tracked as an open, undecided research question — see
[docs/research/RESEARCH-BACKLOG.md](../research/RESEARCH-BACKLOG.md) — due
to unresolved legal/practical ambiguity. It requires explicit sign-off
before any work starts on it, not an inferred green light from this
document.

The project name and any related branding avoid Psyonix/Epic Games
trademarks; "Rocket League" is referenced only descriptively (compatibility
intent), never as a claim of affiliation or endorsement.

## Non-goals

See [CHARTER.md](./CHARTER.md#explicit-non-goals).
