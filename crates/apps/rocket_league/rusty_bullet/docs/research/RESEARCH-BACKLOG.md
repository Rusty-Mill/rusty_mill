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

### RB-RESEARCH-O001 — Build vs. integrate physics engine (RESOLVED)

- **Question**: Roll a from-scratch physics core (more control, matters
  more for the preservation goal and for tuning against the divergence
  metric) vs. integrate an existing engine (e.g. Rapier — faster path to a
  working netcode testbed, less faithful to Rocket League's actual
  Bullet-based car feel)?
- **Status**: Resolved by [ADR-0004](../adr/0004-bullet3-source-port-for-physics-core.md):
  neither "unguided from scratch" nor "integrate an unrelated engine" — a
  direct, cited Rust port of Bullet3's own (public, zlib-licensed)
  algorithms, implemented as `crates/rb_physics_bullet`. Decided ahead of
  `PHASE-0-EXIT` divergence data existing, on the strength of Bullet3's
  direct relevance and permissive license — see ADR-0004's "Validation and
  revisit triggers" for what would reopen this.
- **Owner**: baileyrd.

### RB-RESEARCH-O002 — Binary reverse engineering as a supplementary source

- **Question**: Should this project reverse-engineer the shipped Rocket
  League client binary to recover client-side physics constants, as a
  supplementary verification source beyond replay/BakkesMod data?
- **Status**: Open — legal review below completed; practical work not
  started and currently blocked (see "Practical blocker"). No prior art
  found doing this specifically (for Rocket League's physics constants).
- **Legal review findings** (2026-08-28, web search against Epic Games'
  and Psyonix's current public terms — not legal advice, and not a
  substitute for the owner's own counsel before acting on it):
  - Psyonix's own EULA page (psyonix.com/eula) redirects to Epic Games'
    Terms of Service/EULA family. Epic's EULAs (Epic Games Store EULA,
    Fortnite EULA, and the general pattern across their agreements)
    contain an explicit contractual clause prohibiting reverse
    engineering, decompiling, or disassembling the software, and deriving
    source code from it.
  - Rocket League's Code of Conduct separately prohibits "exposing
    unreleased features or content found within Rocket League's code,"
    which would plausibly also cover publishing anything recovered via
    binary RE, independent of the EULA's RE clause itself.
  - This is a **contractual** prohibition (breach of the EULA the owner
    agreed to by installing/playing the game), which is a different legal
    question from the U.S. DMCA's §1201(f) interoperability exception
    (which permits some circumvention specifically for achieving
    interoperability of independently created software) — this backlog
    entry does not attempt to resolve how those two interact for this
    specific case; that is exactly the kind of judgment call that needs
    the owner's own legal counsel, not a research-backlog conclusion.
  - Net effect of the legal review: proceeding would likely breach the
    EULA the owner has agreed to, independent of whether it's otherwise
    lawful. That's a real cost (account/access risk at minimum) this
    backlog did not previously make concrete.
- **Practical blocker** (2026-08-28): this project's current working
  environment (a sandboxed cloud dev container) has no Rocket League
  installation and no access to the client binary at all. Any actual RE
  work would have to happen on the owner's own machine, not in this
  session — this document can research the legal question but cannot
  itself perform or stage binary analysis here.
- **Constraints to resolve before proceeding**: given the above, whether
  any output derived from it (constants, documentation) would be safe to
  keep in this repository at all, given the project's stated non-goal of
  using or redistributing any Psyonix code (see SYSTEM-ARCHITECTURE.md
  "Legal and IP boundary"). Extracting a numeric constant is arguably a
  different question from extracting code, but that distinction has not
  been legally evaluated here and shouldn't be assumed favorable.
- **Revisit trigger**: Requires explicit owner sign-off after the owner's
  own review of the above (ideally with actual legal counsel, given the
  EULA breach risk this review surfaced) — not an inferred green light
  from this document or from project momentum. Not blocking Phase 0 or
  Phase 1 start; both have proceeded without it.
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

- 2026-08-28: RB-RESEARCH-O001 resolved (ADR-0004: direct Bullet3 source
  port). RB-RESEARCH-O002's legal review completed (Epic/Psyonix EULA and
  Code of Conduct both prohibit reverse engineering contractually; DMCA
  §1201(f) interoperability exception is a separate, unresolved question
  needing the owner's own counsel) and its practical blocker documented
  (no Rocket League client binary accessible in this environment) — still
  open pending owner sign-off, not advanced further.
- 2026-08-28: Initial backlog created at bootstrap, transcribing prior
  research into settled entries and the three open questions from the
  project handoff.
