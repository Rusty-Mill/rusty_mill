# Research Backlog

Tracks what's already established (ground truth — don't re-derive) and what
remains genuinely open. Status vocabulary matches the rest of the repo:
`Settled`, `Open`, `Blocked`.

## Settled (ground truth — cite, don't re-research)

### RB-RESEARCH-S001 — Architecture, per Psyonix's own disclosure

- **Source**: Jared Cone (Lead Gameplay Engineer, Psyonix), GDC 2018, "It IS
  Rocket Science! The Physics and Networking of Rocket League."
  [Video](https://www.youtube.com/watch?v=ueEmiDM94IE),
  [slides](https://media.gdcvault.com/gdc2018/presentations/Cone_Jared_It_Is_Rocket.pdf).
- **Finding**: Client and server both run the same physics simulation
  (Bullet). Server is authoritative; each client also predicts locally so
  input feels instant. Network traffic is periodic authoritative-snapshot
  corrections, not continuous full state. Visible lag/rubber-banding is
  almost certainly reconciliation-smoothing behavior, not raw network
  latency.
- **Used by**: ADR-0001, RB-NET-001. This is the single best public
  technical source and anchors all physics/netcode research.

### RB-RESEARCH-S002 — Physics engine identity

- **Finding**: Rocket League uses a modified Bullet Physics integration
  inside Unreal Engine 3, running fully client-side (confirmed by offline/
  local play requiring no network connection). Bullet is zlib-licensed
  (permissive, no copyleft) but effectively unmaintained upstream since
  v3.2.4 (April 2022) — still stable/production-trusted (Red Dead
  Redemption 1/2), but not actively evolving upstream.
- **Used by**: ADR-0003, CHARTER.md, SYSTEM-ARCHITECTURE.md.

### RB-RESEARCH-S003 — No public Psyonix Bullet fork exists

- **Finding**: Checked Bullet's fork network and issue tracker for
  Psyonix-attributable activity; nothing surfaced. Their integration is
  almost certainly a private internal fork predating the 2015 release.
- **Implication**: No source code comparison is possible; fidelity
  validation must be behavioral (divergence scoring), never
  code-comparison. Feeds directly into ADR-0003's fidelity-vs-engine-choice
  split.

### RB-RESEARCH-S004 — Replay data gap

- **Finding**: Replay files (parseable via the Rust crate `boxcars`) give
  high-fidelity positions/rotations/velocities/boost state, but do not
  reliably capture raw controller inputs (throttle/steer/jump/boost/
  air-roll) — those are lossy/inferred at best.
- **Confidence basis**: Confirmed against prior first-hand experience
  building a replay viewer/analyzer over ~1,000 replay files.
- **Used by**: RB-VERIFY-001 (non-goals), ADR-0002.

### RB-RESEARCH-S005 — BakkesMod is offline-only ground truth now

- **Finding**: Rocket League added Easy Anti-Cheat this year, which now
  blocks BakkesMod during online matches — it's only usable in local/
  offline play against bots. BakkesMod exposes a `ControllerInput` struct
  at high frequency, but only reachable offline.
- **Implication**: Real per-frame controller input can only be captured
  offline, never from live online matches.
- **Used by**: RB-VERIFY-002 (non-goals), ADR-0002, CHARTER.md.

### RB-RESEARCH-S006 — Net verification gap

- **Finding**: No single existing data source gives (real online match) +
  (raw inputs) + (resulting physics state) all at once. Combining
  S004 + S005, the verification pipeline has to be designed around this
  gap explicitly, not assume it away.
- **Used by**: ADR-0002, RB-VERIFY-003 (non-goals), SYSTEM-ARCHITECTURE.md.

## Open (tracked, not yet decided)

### RB-RESEARCH-O001 — Build vs. integrate physics engine

- **Question**: Roll a from-scratch physics core (more control, matters
  more for the preservation goal and for tuning against the divergence
  metric) vs. integrate an existing engine (e.g. Rapier — faster path to a
  working netcode testbed, less faithful to Rocket League's actual
  Bullet-based car feel)?
- **Status**: Open. Explicitly not decided by ADR-0003 (which settles the
  fidelity *target*, not the implementation approach).
- **Revisit trigger**: Once the Phase 0 verification pipeline exists and
  can score both a from-scratch prototype and an integrated-engine
  prototype against the same recorded ground truth, decide based on actual
  divergence numbers rather than priors. Do not decide before that data
  exists.
- **Owner**: baileyrd.

### RB-RESEARCH-O002 — Binary reverse engineering as a supplementary source

- **Question**: Should this project reverse-engineer the shipped Rocket
  League client binary to recover client-side physics constants, as a
  supplementary verification source beyond replay/BakkesMod data?
- **Status**: Open, with real legal/practical ambiguity. No prior art found
  doing this specifically (for Rocket League's physics constants).
- **Constraints to resolve before proceeding**: EULA/ToS terms around
  reverse engineering; whether any output derived from it (constants,
  documentation) would be safe to keep in this repository, given the
  project's stated non-goal of using or redistributing any Psyonix code
  (see SYSTEM-ARCHITECTURE.md "Legal and IP boundary"). Extracting a
  numeric constant is a materially different question from extracting
  code, and that distinction hasn't been legally evaluated here.
- **Revisit trigger**: Requires explicit owner sign-off after that
  legal/practical review — not an inferred green light from this document
  or from project momentum. Not blocking Phase 0 or Phase 1 start.
- **Owner**: baileyrd.

### RB-RESEARCH-O003 — Scope of BakkesMod offline-capture tooling

- **Question**: Does the BakkesMod-side capture tool (RB-VERIFY-002) need
  to be a proper, reusable capture harness (versioned format, configurable
  sampling, robust to BakkesMod API changes) or is a one-off script
  sufficient for Phase 0's needs?
- **Status**: Open.
- **Revisit trigger**: Decide once RB-VERIFY-002 is actually being
  implemented and the real frequency of re-capturing becomes clear (a
  single one-time capture favors a script; a workflow of repeated
  captures across many local sessions favors a harness). Default to the
  smaller option (script) absent evidence a harness is needed, per the
  "no speculative abstraction" convention.
- **Owner**: baileyrd.

## Change history

- 2026-08-28: Initial backlog created at bootstrap, transcribing prior
  research into settled entries and the three open questions from the
  project handoff.
