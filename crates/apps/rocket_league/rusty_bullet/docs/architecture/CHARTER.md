# Project Charter — rusty_bullet

- Status: Accepted
- Owner: baileyrd
- Last reviewed: 2026-08-28

## Purpose

Reimplement Rocket League's client/server physics and networking
architecture in Rust, independently of Psyonix's code, for two concrete
reasons:

1. **Diagnose real network lag.** The owner plays Rocket League regularly
   and experiences persistent lag that Psyonix has not addressed. The
   working method here is to deconstruct the real system to find where the
   flaw actually lives, rather than speculate about it from the outside.
2. **Preservation / continuity.** If Psyonix ever shuts the official
   servers down, local play alone does not let the owner and friends play
   online-style matches together. A working, independently-built clone
   keeps that possible.

Both goals are served by the same build — there is one project, not two.

## Users

- Primary: the project owner, as a diagnostic tool and (eventually) a
  playable clone with friends.
- Secondary: anyone else who wants to understand Rocket League's netcode
  architecture, or who needs a continuity option if official servers stop.
  Not designed, at this stage, for a wide public audience or as a
  commercial product.

## Product shape

- A Rust workspace: a physics/simulation core, a netcode layer (client
  prediction + server reconciliation), and — as a prerequisite to all of
  that — a verification pipeline that scores any candidate implementation
  against recorded ground truth.
- Deployment target: initially a developer's own machines (client +
  self-hosted server for small-group play). No public server hosting,
  matchmaking service, or storefront is in scope.

## External systems and hard constraints

- **Replay files** (`.replay`), parsed via the Rust crate `boxcars`: high-
  fidelity ball/car position/rotation/velocity/boost state, but controller
  inputs are not reliably recoverable from them.
- **BakkesMod**, in local/offline play only: exposes a `ControllerInput`
  struct at high frequency, giving true raw inputs paired with physics
  state — but Easy Anti-Cheat now blocks BakkesMod during online matches,
  so this source never reflects live online conditions.
- No existing data source gives (real online match) + (raw inputs) +
  (resulting physics state) simultaneously. Every later phase's validation
  strategy has to account for this gap rather than assume it away — see
  [docs/research/RESEARCH-BACKLOG.md](../research/RESEARCH-BACKLOG.md).
- Bullet Physics (zlib-licensed, effectively unmaintained upstream since
  v3.2.4) is the physics engine Rocket League itself is built on, per
  Psyonix's own public disclosure. This project targets reproducing that
  behavior; see [ADR-0003](../adr/0003-bullet-fidelity-target.md) for what
  is and isn't decided about *how*.

## Explicit non-goals

- Not a copy, decompilation, or redistribution of Psyonix's client, server,
  Bullet fork, or game assets. See "Legal and IP boundary" in
  [SYSTEM-ARCHITECTURE.md](./SYSTEM-ARCHITECTURE.md).
- Not attempting feature parity with every Rocket League mode, cosmetic
  system, or platform at launch.
- Not building a public matchmaking or hosting service.
- Not attempting to empirically fix Psyonix's live servers or interact with
  them over the network (no packet injection, no live-match interception).
  Diagnosis here means architectural reconstruction and reasoning from
  Psyonix's own disclosed design, not instrumenting their production
  systems.

## Success measures

- Phase 0: the verification pipeline runs end-to-end and produces a
  divergence score on at least one real replay and one BakkesMod capture.
- Phase 1-3: divergence score against recorded ground truth trends down as
  physics/simulation/netcode work lands, with each phase's exit criteria
  tied to that metric where applicable (see
  [ROADMAP.md](../roadmap/ROADMAP.md)).
- Ultimate (open-ended): the owner can play an online-style match against
  friends on this implementation, and has a concrete, evidence-based
  account of where the original lag they experienced actually comes from
  (client prediction smoothing vs. raw network latency vs. something else).

## Major risks

- **Verification gap** (see Purpose/External systems above): no single
  source of real-online-input + physics ground truth. Mitigated by
  combining two partial sources and being explicit about what each does
  and doesn't validate, rather than treating either as complete.
- **Scope creep**: a project this ambitious can expand indefinitely.
  Mitigated by the phase gates in the roadmap and by deferring the
  build-vs-integrate physics decision until it can be scored rather than
  argued about.
- **Legal/IP ambiguity** around any binary reverse engineering of the
  shipped client (tracked as an open research item, not yet approved).
- **Part-time capacity**: staged estimate is ~6-9 months for Phases 1-3
  alone, part-time. Roadmap treats phases as sequential gates, not a fixed
  calendar commitment.

## Ownership, license, and classification

- Owner: baileyrd.
- License: dual MIT/Apache-2.0 (see [LICENSE-MIT](../../LICENSE-MIT),
  [LICENSE-APACHE](../../LICENSE-APACHE)) — permissive, matching the
  project's use of only independently-authored code.
- Data classification: no user data collection. Any recorded
  replays/captures used for verification are the owner's own match data.
