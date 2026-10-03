# Soccar arena collision meshes

The curved parts of the standard (soccar) arena: one corner (x > 0, y < 0),
the side wall's floor ramp, its ceiling ramp, and one goal with the back
wall around it (`soccar_goal`, placed at `y = -5120` and mirrored, as
RLUtilities does; its triangles are used as wound), as little-endian `f32`
vertex triples (`*_vertices.bin`) and `i32` triangle index triples
(`*_ids.bin`). `rb_physics_bullet::arena` mirrors them into all four
corners and both sides, as RLUtilities' `Field::initialize_soccar` does.

- Source: RLUtilities (`samuelpmish/RLUtilities`, branch `develop`,
  `assets/soccar/`), unmodified.
- License: GPL-3.0, which is why this repository is GPL-3.0-only (see
  `/LICENSE`).
- The geometry is Rocket League's own collision mesh, extracted from the
  game by RLUtilities' authors. Rocket League is a trademark of Psyonix /
  Epic Games; this project is not affiliated with them.
