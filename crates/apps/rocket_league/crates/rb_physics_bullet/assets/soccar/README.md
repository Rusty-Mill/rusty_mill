# Soccar arena collision meshes

The standard (soccar) arena's curved parts, as the 16 RocketSim collision
mesh files (`mesh_0.cmf` .. `mesh_15.cmf`): 4 corners, 4 goal halves with
the back wall around them (split at `x = 0`), 4 side-wall floor ramps and
4 ceiling ramps. Each file is little-endian: `i32` triangle count, `i32`
vertex count, the triangles as `i32` vertex index triples, the vertices as
`f32` triples in Bullet units (1 BT = 50 uu). Every triangle is wound to
face the arena. `rb_physics_bullet::arena::standard_meshes` loads them in
file order and `mesh::StaticMesh::from_cmf` reports their triangles in
Bullet's BVH order (`RB-PHYSICS-001-FR-117`, ADR-0037).

- Source: the `rlgym_rocket_league` 2.0.1 source distribution on PyPI
  (`rlgym/rocket_league/sim/collision_meshes/soccar/`), unmodified. They
  are the files RocketSim (`ZealanL/RocketSim`, MIT) loads; its
  `RocketSim.cpp` lists their hashes, which these match.
- License: Apache-2.0 (rlgym's). See `/THIRD_PARTY_NOTICES.md`.
- The geometry is Rocket League's own collision mesh, dumped from the game
  with RocketSim's `RLArenaCollisionDumper`. Rocket League is a trademark
  of Psyonix / Epic Games; this project is not affiliated with them.
