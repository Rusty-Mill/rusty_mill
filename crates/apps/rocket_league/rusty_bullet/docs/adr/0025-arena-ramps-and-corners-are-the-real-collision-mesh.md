# ADR-0025: The arena's ramps and corners are the real collision mesh; the repository is GPL-3.0

- Status: Accepted
- Date: 2026-10-02
- Deciders: baileyrd
- Related: RB-PHYSICS-001-FR-106, FR-102, FR-105; ADR-0006, ADR-0022,
  ADR-0024
- Supersedes/Superseded by: supersedes ADR-0006 and ADR-0022 for the side
  ramps and corners (analytic primitives); amends ADR-0024 (meshes,
  speculative points); relicenses the repository (was MIT OR Apache-2.0)

## Context

After ADR-0024 the worst one-step error was the corner-wall ride in
`test2.jsonl` (8.96-9.07 s). The recorded car's normal and friction match
the port's (friction / normal impulse = 0.30), but it keeps taking small
wall impulses for 100 ms while the candidate, started from each recorded
state, finds no contact. RocketSim collides with Rocket League's own
triangle mesh. Its flat facets sit up to ~2.5 uu inside the smooth curves
ADR-0022 fitted through the mesh's vertices: against the triangles the
recorded corners penetrate 0.4-1.6 uu during those ticks, against the
smooth curve they are 1-3 uu clear. At 9.067 s the recorded car takes an
impulse from 0.75 uu clear of the triangles: Bullet gives its whole
manifold, points up to the breaking threshold away, to the solver.

The only mesh source at hand is RLUtilities' `assets/soccar/` (GPL-3.0).
The owner accepted relicensing this repository to GPL-3.0 to embed it.

## Decision

- `mesh::StaticMesh`: triangles facing the arena, a uniform 256 uu grid,
  front-face ray casts, and sphere contacts (one per triangle, deepest 4).
- `arena::standard_meshes`: the corner (mirrored into all four quadrants),
  the side floor ramp and ceiling ramp (mirrored to both sides),
  `include_bytes!` from `crates/rb_physics_bullet/assets/soccar/`, as
  RLUtilities' `Field::initialize_soccar` assembles them.
- They replace the corner wall planes, the side and corner seams, the
  vertical corner edges and FR-102's swept fillets. The back walls' seams
  and the goal mouth stay analytic: the goal mesh is not used.
- `collision::ContactManifold` (was `PlaneManifold`) stores each point's
  own plane, so one manifold serves a plane (support corner a tick) or a
  mesh (each nearby triangle's deepest covered corner a tick), and returns
  every stored point; a point still clear of its surface is a speculative
  contact (`solver::setup_rows` already handles a positive gap).
- Wheel rays take the nearer of the analytic hit and the mesh hit.
- The workspace license is GPL-3.0-only; `LICENSE` holds the GPL text.

## Consequences

- One-step error: `test2.jsonl` mean 1.78 uu/s (was 2.25), corner/wall
  segment 6.0 (was 13.4); 8.967 s 23 (was 178), 8.992 s 35 (was 73),
  9.067 s 36 (was 134), 8.958 s 173 (was 223). `front.jsonl` mean 0.385
  (was 0.407; speculative points). `side.jsonl` unchanged. `rb-verify`
  runs ~25% faster.
- The repository can no longer be used under MIT/Apache terms; earlier
  commits keep them. The mesh is Rocket League's geometry as extracted by
  RLUtilities' authors (`assets/soccar/README.md`).
- Not modeled: the goal mesh (back walls, goal box); the ball's mesh
  contacts are stateless; a full-manifold replacement uses oldest-first,
  not Bullet's area heuristic.
