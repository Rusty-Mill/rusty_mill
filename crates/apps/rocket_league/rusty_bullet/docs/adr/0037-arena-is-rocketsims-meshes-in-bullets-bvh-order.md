# ADR-0037: The arena is RocketSim's 16 meshes, reported in Bullet's BVH order; the repository is MIT OR Apache-2.0 again

- Status: Accepted
- Date: 2026-10-03
- Deciders: baileyrd
- Related: RB-PHYSICS-001-FR-117, FR-106, FR-109, FR-113, FR-115, FR-116; ADR-0025, ADR-0033, ADR-0035, ADR-0036
- Supersedes/Superseded by: supersedes ADR-0025's licensing decision

## Context

After FR-116, the worst ball frame was `hitjump.jsonl` 127.633 s at
988 uu/s: the ball meets the crossbar bevel at a vertex shared by 8
triangles, which fold into one manifold point, the last triangle reported
(FR-116). In our mesh index order a bevel triangle came last and kept its
downward normal. Bullet reports triangles in the leaf order of its
`btQuantizedBvh`, which depends on the mesh's triangle order and bounds.

RLUtilities' meshes (ADR-0025, ADR-0033) turned out not to be RocketSim's:
- RocketSim loads 16 files (4 corners, 4 goal halves split at `x = 0`,
  8 ramps); RLUtilities has 4, mirrored into 10 meshes.
- The geometry matches triangle for triangle, but the order differs, so
  no BVH over RLUtilities' meshes reproduces RocketSim's visit order.
- RocketSim's files ship in the `rlgym_rocket_league` 2.0.1 source
  distribution on PyPI under Apache-2.0; RocketSim's `MeshHashSet`
  hashes identify them (all 16 match). They are in Bullet units, wound
  to face the arena.

The RLUtilities assets were GPL-3.0, the only reason the repository was
relicensed GPL-3.0-only in ADR-0025. Every commit since is the owner's.

## Decision

- Vendor the 16 `.cmf` files under `assets/soccar/` and drop the four
  RLUtilities `.bin` files and their loaders. `arena::standard_meshes`
  loads them in RocketSim's order through `StaticMesh::from_cmf`.
- `bvh::visit_order` ports `btQuantizedBvh`'s build in `f32`, in Bullet
  units: the quantization frame, per-leaf quantized bounds, the
  max-variance split axis, the mean partition with its one-third balance
  fallback. `sphere_contacts` visits near triangles by that rank.
- Relicense the workspace `MIT OR Apache-2.0`, restoring the pre-ADR-0025
  `LICENSE-MIT`/`LICENSE-APACHE`; `THIRD_PARTY_NOTICES.md` records the
  meshes' provenance and the GPL span.

Alternatives considered:
- Reorder RLUtilities' triangles to match. There is no mapping to derive
  the order from; it would be copying RocketSim's order anyway.
- Keep both asset sets. Nothing would use the RLUtilities set, and it
  would keep the GPL obligation.
- Prefer the clamped normal at shared vertices (ADR-0036's rejected
  alternative). Still not Bullet.

## Consequences

- `hitjump` one-step ball mean, resets excluded: 0.513 (was 0.589); the
  new meshes in file order alone give 0.569, so the BVH order is a real
  part of the gain.
- `hitjump` worst ball frame: 208 uu/s at 97.908 s (was 988 at
  127.633 s); the 20.767 s frame (633) is gone too.
- `hitjump` k = 30 ball: 20.84 (was 22.47). `test2`, `front` and `side`
  are unchanged.
- The ported order is only right while our triangle order equals
  RocketSim's, which the vendored files guarantee; `from_cmf` must keep
  degenerate triangles in the order until after ranking, as it does.
- Bullet's subtree headers and the quantized AABB overlap test are not
  ported: they change which leaves a query visits, not the order of the
  ones it reports, and our grid broad-phase over-reports harmlessly.
- The goal halves meet at `x = 0`; FR-109's edge classification works per
  mesh, so a seam between halves is `Open` there. RocketSim has the same
  seam.
