# ADR-0010: Model each goal net as a mass-spring mesh of rigid point bodies

- Status: Accepted (recorded retroactively 2026-09-30; decided 2026-08-31)
- Date: 2026-08-31
- Deciders: baileyrd
- Related: RB-PHYSICS-001-FR-029, FR-033, FR-038, FR-050; PRs #67, #75,
  #85, #107; ADR-0004, ADR-0006, ADR-0008
- Supersedes/Superseded by: none

## Context

FR-029 put a solid bounded box behind each goal window, so a ball or car
passing through settles instead of flying forever. It was scoped as "not
a springy/catching net mesh", a deliberate simplification.

The motivation for a real net was qualitative: the ball visibly tangling
in netting is a fidelity gap worth closing regardless of any specific
divergence signal. No doc cites a source for how Rocket League's net
deforms; its material properties have never been published.

## Decision drivers

- A net should catch and give, not bounce like a wall.
- Bullet's soft-body code is outside this port's scope (ADR-0004), so a
  net cannot be a straight port.
- Reusing existing collision and solver code keeps the new math to the
  springs alone.

## Considered options

1. **A mass-spring grid of tiny rigid point bodies** (chosen). Each point
   is a `RigidBody::sphere`, so ball and car contact use the existing
   `sphere_vs_sphere` and box-vs-sphere narrow phase and the existing
   dynamic solver path.
2. **A bespoke penalty-force net** (springs push directly on the ball or
   car): rejected in favor of option 1's zero new solver code.
3. **Keep only FR-029's rigid box**: no catching behavior. The box is kept
   anyway, as a fixed backstop behind the net.
4. **A port of Bullet's `btSoftBody`**: out of scope per ADR-0004.

## Decision

`net::NetMesh` is a flat rectangular `cols` x `rows` grid sized to the
goal mouth:
- perimeter points are anchored to the goal frame and interior points are
  free;
- points are joined by structural and shear springs (Hooke's law plus
  axial damping), the only new physics math;
- each `step` is split into `NET_SUBSTEPS` for stability, and the net
  changes only velocity;
- nets resolve after `resolve_manifolds` (see ADR-0008), on the same
  ball-and-cars snapshot;
- cars are caught since FR-038, and net-point contacts use the combined
  dynamic path since FR-050.

The spring math is original to this port, not ported from Bullet.

## Consequences

### Positive

- The ball and cars are caught and slowed by a deforming net, with FR-029's
  box as a backstop.
- Contact reuses tested narrow-phase and solver code.

### Negative / tradeoffs

- It is a single flat panel, not a 3D "sock", though it bends backward
  when hit.
- No bending stiffness, and one contact per overlapping point.
- Every net constant, including `NET_DEPTH`, is an uncalibrated
  placeholder with no published reference.
- It is solved outside the main combined solve, so a body touching both
  the net and another body is resolved in two phases.
- Mass-spring sub-stepping adds per-step cost.

## Validation and revisit triggers

- Revisit if captures show goal-mouth divergence attributable to how the
  net catches (post-goal frames usually matter little to gameplay
  physics).
- Revisit if the net's cost shows up in `PHASE-2-DETERMINISM` or netcode
  profiling.
- Revisit if solving the net inside `resolve_manifolds` becomes necessary
  for consistency (ADR-0008).
