# ADR-0004: Resolve build-vs-integrate by porting Bullet3's algorithms directly

- Status: Accepted
- Date: 2026-08-28
- Deciders: baileyrd
- Related: RB-PHYSICS-001, ADR-0003, docs/research/RESEARCH-BACKLOG.md (RB-RESEARCH-O001)
- Supersedes/Superseded by: none (resolves RB-RESEARCH-O001, which
  ADR-0003 explicitly deferred rather than decided)

## Context

ADR-0003 settled *what* Phase 1 targets (Bullet-derived car/ball fidelity)
but explicitly deferred *how* to build it — from scratch, unguided, vs.
integrating an existing engine (e.g. Rapier) vs. some other approach —
pending the verification pipeline existing to score real candidates
(`RB-RESEARCH-O001`).

The owner has since directed a specific third path not fully captured by
that original framing: port Bullet3's own real algorithms into Rust
directly, rather than either (a) inventing car/ball physics from general
first principles, or (b) adopting a different, unrelated engine's actual
dynamics (Rapier's contact solver, integration scheme, etc. are its own
design, not Bullet's). Bullet3's own source (the public, zlib-licensed
upstream project — distinct from Psyonix's private, unavailable fork) is
directly inspectable and, per its license, freely portable.

This ADR records that direction as the decision, ahead of `PHASE-0-EXIT`
data existing to score it against alternatives — a deliberate departure
from ADR-0002's general verification-first ordering, justified below.

## Decision drivers

- Bullet3's actual source is available and zlib-licensed (permissive,
  explicitly permits alteration and redistribution) — see
  `THIRD_PARTY_NOTICES.md`. Porting it is legally uncomplicated, unlike
  RB-RESEARCH-O002's binary-RE question.
- A direct port is more likely to reproduce Rocket League's actual car/ball
  *feel* than either an unguided from-scratch design or a different
  engine's contact-solving math, because Rocket League's own physics is a
  modified Bullet integration (ADR-0003) — porting Bullet's real algorithms
  is the closest available approximation to that starting point absent
  access to Psyonix's actual fork.
- Deferring this decision further (per ADR-0003's original framing) would
  mean building a scoring harness for multiple candidate approaches before
  writing any real physics code — real, but slower to a working baseline
  than committing to the approach with the strongest evidence-based prior
  (faithfulness to the actual documented physics foundation) and revising
  later if the divergence metric says otherwise.

## Considered options

1. **Port Bullet3's real algorithms into Rust directly** (this decision).
   Read the actual zlib-licensed source, translate its rigid-body
   integration and sequential-impulse contact solver into idiomatic,
   tested Rust — not a binding, not a vendored build, a genuine rewrite
   that preserves the math and structure (see `THIRD_PARTY_NOTICES.md` for
   exactly which functions).
2. **Integrate an existing Rust physics engine (e.g. Rapier).** Faster to a
   working netcode testbed, but Rapier's actual contact solver and
   integration scheme are its own design, not Bullet's — likely to diverge
   from Rocket League's real car/ball feel in ways that are hard to
   attribute or fix, since the "physics" isn't ours to inspect and rewrite.
3. **Design physics from scratch, unguided by any existing engine.** Full
   control, but throws away the one concrete, inspectable analog to
   Rocket League's real foundation (public Bullet3) in favor of building
   from general principles alone — more likely to require many iterations
   against the divergence metric before converging on anything
   Rocket-League-like.

## Decision

Adopt option 1. `rb_physics_bullet` (new crate) is a from-scratch Rust port
of specific, cited Bullet3 algorithms — starting with rigid-body
integration (`btRigidBody`) and the sequential-impulse contact solver
(`btSequentialImpulseConstraintSolver`), scoped to a dynamic sphere (the
ball) against a static plane (the ground) for this increment. Box-shaped
car bodies, general 3x3 inertia tensors, split impulse, and warm-starting/
sleeping are explicitly out of scope for this increment (tracked in
`RB-PHYSICS-001`), not silently dropped.

## Consequences

### Positive

- Physics behavior is traceable to a real, well-understood reference
  implementation, not invented — every non-trivial formula in
  `rb_physics_bullet` cites the exact Bullet3 file/function it ports (see
  `THIRD_PARTY_NOTICES.md`).
- Unblocks Phase 1 immediately rather than waiting on a scoring harness for
  multiple candidate approaches.
- Legally uncomplicated: zlib explicitly permits this, unlike
  `RB-RESEARCH-O002`'s binary-RE question.

### Negative / tradeoffs

- This is still not the same code as Rocket League's actual (private,
  modified) Bullet fork — porting public Bullet3 narrows the gap to
  Rocket League's real physics but does not close it. The divergence
  metric (`RB-VERIFY-003`), once fed real replay/capture data, is what
  actually tells us how close v0 gets.
- Deviates from ADR-0002's stated ordering (verification pipeline fully
  informing Phase 1 decisions) — this decision was made on the strength of
  Bullet3's direct relevance and licensing, not on divergence-score
  evidence, because no candidate existed yet to score. Once real
  divergence data exists, if it shows this approach diverging badly, that
  data — not a re-litigation of "should we have started differently" —
  drives what changes next.
- v0's scope (sphere-vs-plane only) doesn't yet exercise the harder parts
  of a faithful port (general inertia tensors, multi-contact manifolds,
  box collision) — those are real, not-yet-attempted next increments.

## Validation and revisit triggers

Revisit if `RB-VERIFY-003` divergence scores against real replay/BakkesMod
data (once `RB-VERIFY-001`/`RB-VERIFY-002` are implemented) show this
approach diverging from Rocket League's actual ball/car behavior badly
enough that a different approach (e.g. integrating an existing engine
after all, or deviating further from Bullet3's specific algorithms) scores
better. Until then, continue extending `rb_physics_bullet`'s scope
(car boxes, general inertia, split impulse, warm-starting) rather than
reconsidering the port-vs-integrate choice itself.
