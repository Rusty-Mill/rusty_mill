# Third-Party Notices

## Bullet Physics (Bullet3) — algorithm port

`crates/rb_physics_bullet` is a from-scratch Rust **translation of specific
algorithms** from [Bullet Physics (bullet3)](https://github.com/bulletphysics/bullet3),
not a binding, a vendored copy, or a build of Bullet's C++ source. No
Bullet3 source file is copied, compiled, or linked into this repository —
the Rust code was written independently, informed by reading the referenced
functions to reproduce their math and control flow faithfully.

This notice exists to satisfy zlib license condition 1 (the origin of this
software must not be misrepresented) and to comply with condition 2
(altered source versions must be plainly marked as such) for the portions
of `rb_physics_bullet` derived from Bullet3.

### Bullet3's license (as it applies to the original work this ports)

```
Bullet Continuous Collision Detection and Physics Library
Copyright (c) 2003-2006 Erwin Coumans  https://bulletphysics.org

This software is provided 'as-is', without any express or implied warranty.
In no event will the authors be held liable for any damages arising from
the use of this software.
Permission is granted to anyone to use this software for any purpose,
including commercial applications, and to alter it and redistribute it
freely, subject to the following restrictions:

1. The origin of this software must not be misrepresented; you must not
   claim that you wrote the original software. If you use this software in
   a product, an acknowledgment in the product documentation would be
   appreciated but is not required.
2. Altered source versions must be plainly marked as such, and must not be
   misrepresented as being the original software.
3. This notice may not be removed or altered from any source distribution.
```

Full text: <https://github.com/bulletphysics/bullet3/blob/main/LICENSE.txt>.

### What was ported, from where

Each `rb_physics_bullet` module names its Bullet3 source file(s) in its
module doc comment. Summary, with the exact functions this port's
algorithms are derived from:

| `rb_physics_bullet` module | Bullet3 source | Function(s) ported |
|---|---|---|
| `integrate::apply_gravity`, `apply_damping`, `integrate_velocities` | `src/BulletDynamics/Dynamics/btRigidBody.cpp` | `applyGravity`, `applyDamping`, `integrateVelocities` |
| `integrate::integrate_transform` | `src/LinearMath/btTransformUtil.h` | `btTransformUtil::integrateTransform` |
| `solver::setup_rows`, `resolve_row` | `src/BulletDynamics/ConstraintSolver/btSequentialImpulseConstraintSolver.cpp` | `setupContactConstraint`, `setupFrictionConstraint`, `resolveSingleConstraintRowGeneric`, `resolveSingleConstraintRowLowerLimit`, `restitutionCurve` |
| `solver::plane_space` | `src/LinearMath/btVector3.h` | `btPlaneSpace1` |
| `body::RigidBody::update_inertia_tensor` | `src/BulletDynamics/Dynamics/btRigidBody.cpp` | `btRigidBody::updateInertiaTensor` |
| `body::Shape::local_inertia` | `src/BulletCollision/CollisionShapes/btSphereShape.h`, `btBoxShape.h` | `btSphereShape::calculateLocalInertia`, `btBoxShape::calculateLocalInertia` |
| `mat3::Mat3::from_quat` | `src/LinearMath/btMatrix3x3.h` | `btMatrix3x3::setRotation` |
| `mat3::Mat3::scaled_columns` | `src/LinearMath/btMatrix3x3.h` | `btMatrix3x3::scaled` |
| `state::Quat::conjugate` (in `rb_domain`, used by `collision::sphere_vs_box`) | `src/LinearMath/btQuaternion.h` | `btQuaternion::inverse` |
| `collision::sphere_vs_box` | `src/BulletCollision/CollisionDispatch/btBoxBoxCollisionAlgorithm.cpp` (closest-point-on-box structure) | closed-form equivalent of the closest-point query `btBoxSphereCollisionAlgorithm` runs via general support mapping — see the function's own doc comment for why a box's closest point to an external point reduces to a plain per-axis clamp |
| `solver::setup_two_body_rows`, `resolve_two_body_row`, `resolve_contacts_between` | `src/BulletDynamics/ConstraintSolver/btSequentialImpulseConstraintSolver.cpp` | the same `setupContactConstraint`/`resolveSingleConstraintRowGeneric` family as `solver::setup_rows`/`resolve_row`, generalized to carry both bodies' mass/inertia contributions (Bullet's solver always does; `resolve_contacts`' one-body-only version only worked because a static plane's side is always zero) |
| `collision::box_vs_box` (SAT axis test) | `src/BulletCollision/CollisionDispatch/btBoxBoxDetector.cpp` | `btBoxBoxDetector::dBoxBox`'s separating-axis loop (15 candidate axes: 3+3 face normals, 9 edge-pair cross products) — itself derived from ODE's `dBoxBox`, credited in Bullet3's own source comments |
| `collision::face_contact` | `src/BulletCollision/CollisionDispatch/btBoxBoxDetector.cpp` | a box-specific closed form of `dBoxBox`'s incident-face-vs-reference-face polygon clipping, direct rather than a line-for-line port of its (ODE-derived) general clipping code — see the function's own doc comment |
| `collision::edge_contact`'s closest-point step | *(not from Bullet3)* | standard closest-point-between-two-segments construction (e.g. Ericson, *Real-Time Collision Detection*, §5.1.9), used for `dBoxBox`'s edge-edge contact case |

Deliberate, documented deviations from the original algorithms (no split
impulse, no SIMD, no warm-starting/sleeping, `box_vs_box`'s clipping and
edge-edge steps implemented directly rather than as a line-for-line port of
`dBoxBox`'s own (ODE-derived) code, a different restitution/friction
combine mode) are noted in the relevant Rust module's doc comments and in
`RB-PHYSICS-001`/ADR-0004 — this port does not claim behavioral equivalence
with upstream Bullet3, only that its core integration and contact-solving
math is derived from it.

### Not from Bullet3

Rocket League's own modified/forked Bullet integration is **not** available
publicly and is not used, referenced, or reverse-engineered by this port —
see `docs/architecture/SYSTEM-ARCHITECTURE.md`'s "Legal and IP boundary".
Everything in this file concerns the public, zlib-licensed upstream Bullet3
project only.
