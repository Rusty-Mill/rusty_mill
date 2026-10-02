//! Static triangle meshes (`RB-PHYSICS-001-FR-106`, ADR-0025): the arena's
//! curved ramps and corners as Rocket League's own collision triangles,
//! which RocketSim also collides against (`btBvhTriangleMeshShape`). The
//! flat facets sit up to ~2.5 uu inside the smooth curves the port used
//! before, and the owner's corner-wall ride (`test2.jsonl` 8.96-9.07 s)
//! touches those facets where the smooth curve leaves a gap.
//!
//! A `StaticMesh` holds triangles with normals facing the arena interior
//! and a uniform grid for broad-phase queries. Contact generation for a
//! car (one deepest corner per triangle a tick, kept in a persistent
//! manifold) lives in `collision::ContactManifold`; the ball and wheel
//! rays query the mesh directly here.

use crate::collision::{Contact, RayHit, CONTACT_PROCESSING_THRESHOLD};
use rb_domain::Vec3;
use std::collections::HashMap;

/// Broad-phase grid cell edge (uu).
const CELL_SIZE: f32 = 256.0;

/// How far past a triangle's edges (uu) a point still counts as over it,
/// plus a quarter of its depth behind the triangle: behind a concave seam
/// the two facets' prisms leave a gap that widens with depth (~11 degrees
/// between ramp facets), which Bullet's closest-feature search covers.
const EDGE_TOLERANCE: f32 = 0.5;
const EDGE_TOLERANCE_PER_DEPTH: f32 = 0.25;

/// One triangle, wound so that `(b - a) x (c - a)` points along `normal`,
/// the side facing the arena.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct Triangle {
    pub vertices: [Vec3; 3],
    pub normal: Vec3,
}

impl Triangle {
    /// The triangle `a, b, c` with its normal turned toward `inside`, or
    /// `None` when it has no area.
    pub fn facing(a: Vec3, b: Vec3, c: Vec3, inside: Vec3) -> Option<Triangle> {
        let normal = (b - a).cross(&(c - a)).normalize()?;
        let centroid = (a + b + c) * (1.0 / 3.0);
        Some(if normal.dot(&(inside - centroid)) >= 0.0 {
            Triangle {
                vertices: [a, b, c],
                normal,
            }
        } else {
            Triangle {
                vertices: [a, c, b],
                normal: -normal,
            }
        })
    }

    /// Height of `point` above the triangle's plane, positive on the
    /// arena side.
    pub fn signed_distance(&self, point: &Vec3) -> f32 {
        self.normal.dot(&(*point - self.vertices[0]))
    }

    /// `normal . p` for every `p` on the plane.
    pub fn offset(&self) -> f32 {
        self.normal.dot(&self.vertices[0])
    }

    /// Whether `point` projects onto the triangle, `tolerance` uu past its
    /// edges allowed.
    pub fn covers(&self, point: &Vec3, tolerance: f32) -> bool {
        (0..3).all(|i| {
            let start = self.vertices[i];
            let edge = self.vertices[(i + 1) % 3] - start;
            let length = edge.length();
            length > 0.0 && edge.cross(&(*point - start)).dot(&self.normal) / length >= -tolerance
        })
    }

    /// `covers` with the depth-dependent tolerance used for contacts.
    pub fn covers_at_depth(&self, point: &Vec3, gap: f32) -> bool {
        self.covers(
            point,
            EDGE_TOLERANCE + EDGE_TOLERANCE_PER_DEPTH * (-gap).max(0.0),
        )
    }

    /// The triangle's point nearest `point` (Ericson, Real-Time Collision
    /// Detection 5.1.5).
    pub fn closest_point(&self, point: &Vec3) -> Vec3 {
        let [a, b, c] = self.vertices;
        let (ab, ac, ap) = (b - a, c - a, *point - a);
        let (d1, d2) = (ab.dot(&ap), ac.dot(&ap));
        if d1 <= 0.0 && d2 <= 0.0 {
            return a;
        }
        let bp = *point - b;
        let (d3, d4) = (ab.dot(&bp), ac.dot(&bp));
        if d3 >= 0.0 && d4 <= d3 {
            return b;
        }
        let vc = d1 * d4 - d3 * d2;
        if vc <= 0.0 && d1 >= 0.0 && d3 <= 0.0 {
            return a + ab * (d1 / (d1 - d3));
        }
        let cp = *point - c;
        let (d5, d6) = (ab.dot(&cp), ac.dot(&cp));
        if d6 >= 0.0 && d5 <= d6 {
            return c;
        }
        let vb = d5 * d2 - d1 * d6;
        if vb <= 0.0 && d2 >= 0.0 && d6 <= 0.0 {
            return a + ac * (d2 / (d2 - d6));
        }
        let va = d3 * d6 - d5 * d4;
        if va <= 0.0 && d4 - d3 >= 0.0 && d5 - d6 >= 0.0 {
            return b + (c - b) * ((d4 - d3) / ((d4 - d3) + (d5 - d6)));
        }
        let denominator = 1.0 / (va + vb + vc);
        a + ab * (vb * denominator) + ac * (vc * denominator)
    }

    /// Distance along a ray from the arena side to the triangle
    /// (Moller-Trumbore), front face only.
    fn ray_distance(&self, origin: &Vec3, direction: &Vec3, length: f32) -> Option<f32> {
        if direction.dot(&self.normal) >= 0.0 {
            return None;
        }
        let [a, b, c] = self.vertices;
        let (ab, ac) = (b - a, c - a);
        let p = direction.cross(&ac);
        let determinant = ab.dot(&p);
        if determinant.abs() < 1e-9 {
            return None;
        }
        let inverse = 1.0 / determinant;
        let t_vec = *origin - a;
        let u = t_vec.dot(&p) * inverse;
        if !(0.0..=1.0).contains(&u) {
            return None;
        }
        let q = t_vec.cross(&ab);
        let v = direction.dot(&q) * inverse;
        if v < 0.0 || u + v > 1.0 {
            return None;
        }
        let distance = ac.dot(&q) * inverse;
        (0.0..=length).contains(&distance).then_some(distance)
    }

    fn bounds(&self) -> (Vec3, Vec3) {
        let [a, b, c] = self.vertices;
        (
            Vec3::new(
                a.x.min(b.x).min(c.x),
                a.y.min(b.y).min(c.y),
                a.z.min(b.z).min(c.z),
            ),
            Vec3::new(
                a.x.max(b.x).max(c.x),
                a.y.max(b.y).max(c.y),
                a.z.max(b.z).max(c.z),
            ),
        )
    }
}

/// An immovable triangle mesh with a broad-phase grid.
#[derive(Debug, Clone, PartialEq)]
pub struct StaticMesh {
    triangles: Vec<Triangle>,
    cells: HashMap<(i32, i32, i32), Vec<u32>>,
    pub restitution: f32,
    pub friction: f32,
}

fn cell_of(value: f32) -> i32 {
    (value / CELL_SIZE).floor() as i32
}

impl StaticMesh {
    /// Indexes `triangles` with the same default material as the other
    /// static shapes.
    pub fn new(triangles: Vec<Triangle>) -> StaticMesh {
        let mut cells: HashMap<(i32, i32, i32), Vec<u32>> = HashMap::new();
        for (index, triangle) in triangles.iter().enumerate() {
            let (min, max) = triangle.bounds();
            for x in cell_of(min.x)..=cell_of(max.x) {
                for y in cell_of(min.y)..=cell_of(max.y) {
                    for z in cell_of(min.z)..=cell_of(max.z) {
                        cells.entry((x, y, z)).or_default().push(index as u32);
                    }
                }
            }
        }
        StaticMesh {
            triangles,
            cells,
            restitution: 0.5,
            friction: 0.5,
        }
    }

    /// Builds a mesh from little-endian `f32` vertex triples and `i32`
    /// index triples (RLUtilities' asset layout), each vertex scaled
    /// component-wise by `mirror` (`+-1` per axis) and every triangle
    /// facing `inside`. Triangles naming a missing vertex, or with no
    /// area, are skipped.
    pub fn from_buffers(vertices: &[u8], ids: &[u8], mirror: Vec3, inside: Vec3) -> StaticMesh {
        let word = |chunk: &[u8]| [chunk[0], chunk[1], chunk[2], chunk[3]];
        let points: Vec<Vec3> = vertices
            .chunks_exact(12)
            .map(|v| {
                let x = f32::from_le_bytes(word(&v[0..4]));
                let y = f32::from_le_bytes(word(&v[4..8]));
                let z = f32::from_le_bytes(word(&v[8..12]));
                Vec3::new(x * mirror.x, y * mirror.y, z * mirror.z)
            })
            .collect();
        let vertex = |chunk: &[u8]| {
            usize::try_from(i32::from_le_bytes(word(chunk)))
                .ok()
                .and_then(|i| points.get(i).copied())
        };
        let triangles = ids
            .chunks_exact(12)
            .filter_map(|t| {
                let (a, b, c) = (vertex(&t[0..4])?, vertex(&t[4..8])?, vertex(&t[8..12])?);
                Triangle::facing(a, b, c, inside)
            })
            .collect();
        StaticMesh::new(triangles)
    }

    /// Every triangle.
    pub fn triangles(&self) -> &[Triangle] {
        &self.triangles
    }

    /// Triangles whose grid cells overlap the box `min..max`.
    pub fn near(&self, min: Vec3, max: Vec3) -> impl Iterator<Item = &Triangle> {
        let mut indices: Vec<u32> = Vec::new();
        for x in cell_of(min.x)..=cell_of(max.x) {
            for y in cell_of(min.y)..=cell_of(max.y) {
                for z in cell_of(min.z)..=cell_of(max.z) {
                    if let Some(cell) = self.cells.get(&(x, y, z)) {
                        indices.extend(cell);
                    }
                }
            }
        }
        indices.sort_unstable();
        indices.dedup();
        indices
            .into_iter()
            .filter_map(|index| self.triangles.get(index as usize))
    }

    /// The nearest front-facing triangle along the ray, if within `length`.
    pub fn raycast(&self, origin: Vec3, direction: Vec3, length: f32) -> Option<RayHit> {
        let end = origin + direction * length;
        let min = Vec3::new(
            origin.x.min(end.x),
            origin.y.min(end.y),
            origin.z.min(end.z),
        );
        let max = Vec3::new(
            origin.x.max(end.x),
            origin.y.max(end.y),
            origin.z.max(end.z),
        );
        self.near(min, max)
            .filter_map(|triangle| {
                triangle
                    .ray_distance(&origin, &direction, length)
                    .map(|distance| RayHit {
                        distance,
                        normal: triangle.normal,
                    })
            })
            .min_by(|a, b| a.distance.total_cmp(&b.distance))
    }

    /// A sphere's contacts, one per triangle it reaches, as Bullet's
    /// `btSphereTriangleCollisionAlgorithm` makes them, deepest first and
    /// at most 4 (one persistent manifold's worth).
    pub fn sphere_contacts(&self, center: Vec3, radius: f32) -> Vec<Contact> {
        let reach = Vec3::new(radius, radius, radius);
        let mut contacts: Vec<Contact> = self
            .near(center - reach, center + reach)
            .filter_map(|triangle| {
                let height = triangle.signed_distance(&center);
                if height < -radius {
                    return None;
                }
                let closest = triangle.closest_point(&center);
                let offset = center - closest;
                let distance = offset.length();
                let (normal, depth) = if height > 0.0 && distance > 1e-6 {
                    (offset * (1.0 / distance), radius - distance)
                } else if triangle.covers_at_depth(&center, height) {
                    (triangle.normal, radius - height)
                } else {
                    return None;
                };
                (depth >= -CONTACT_PROCESSING_THRESHOLD).then_some(Contact {
                    normal,
                    point: closest,
                    penetration_depth: depth,
                })
            })
            .collect();
        contacts.sort_by(|a, b| b.penetration_depth.total_cmp(&a.penetration_depth));
        contacts.truncate(4);
        contacts
    }
}

#[cfg(test)]
#[allow(clippy::expect_used)]
mod tests {
    use super::*;

    fn floor_quad() -> StaticMesh {
        let inside = Vec3::new(0.0, 0.0, 100.0);
        let (a, b, c, d) = (
            Vec3::new(-100.0, -100.0, 0.0),
            Vec3::new(100.0, -100.0, 0.0),
            Vec3::new(100.0, 100.0, 0.0),
            Vec3::new(-100.0, 100.0, 0.0),
        );
        StaticMesh::new(vec![
            Triangle::facing(a, c, b, inside).expect("has area"),
            Triangle::facing(a, d, c, inside).expect("has area"),
        ])
    }

    #[test]
    fn triangles_face_the_inside_point_whatever_their_winding() {
        let mesh = floor_quad();
        for triangle in mesh.triangles() {
            assert!((triangle.normal - Vec3::new(0.0, 0.0, 1.0)).length() < 1e-6);
            let [a, b, c] = triangle.vertices;
            let wound = (b - a).cross(&(c - a)).normalize().expect("has area");
            assert!((wound - triangle.normal).length() < 1e-6);
        }
    }

    #[test]
    fn a_ray_hits_the_front_face_only() {
        let mesh = floor_quad();
        let down = Vec3::new(0.0, 0.0, -1.0);
        let hit = mesh
            .raycast(Vec3::new(10.0, 20.0, 30.0), down, 50.0)
            .expect("the floor is 30 uu below");
        assert!((hit.distance - 30.0).abs() < 1e-4);
        assert!(mesh
            .raycast(Vec3::new(10.0, 20.0, 30.0), down, 20.0)
            .is_none());
        assert!(mesh
            .raycast(Vec3::new(10.0, 20.0, -30.0), -down, 50.0)
            .is_none());
        assert!(mesh
            .raycast(Vec3::new(500.0, 0.0, 30.0), down, 50.0)
            .is_none());
    }

    #[test]
    fn a_sphere_touching_the_mesh_gets_one_contact_per_triangle_it_reaches() {
        let mesh = floor_quad();
        let contacts = mesh.sphere_contacts(Vec3::new(30.0, -40.0, 8.0), 10.0);
        assert_eq!(contacts.len(), 1);
        assert!((contacts[0].penetration_depth - 2.0).abs() < 1e-4);
        assert!((contacts[0].normal - Vec3::new(0.0, 0.0, 1.0)).length() < 1e-4);
        // On the shared diagonal both triangles report it.
        assert_eq!(
            mesh.sphere_contacts(Vec3::new(0.0, 0.0, 8.0), 10.0).len(),
            2
        );
        assert!(mesh
            .sphere_contacts(Vec3::new(30.0, -40.0, 12.0), 10.0)
            .is_empty());
    }

    #[test]
    fn closest_point_clamps_to_edges_and_vertices() {
        let triangle = Triangle::facing(
            Vec3::ZERO,
            Vec3::new(10.0, 0.0, 0.0),
            Vec3::new(0.0, 10.0, 0.0),
            Vec3::new(0.0, 0.0, 5.0),
        )
        .expect("has area");
        assert_eq!(
            triangle.closest_point(&Vec3::new(-5.0, -5.0, 3.0)),
            Vec3::ZERO
        );
        let on_edge = triangle.closest_point(&Vec3::new(5.0, -3.0, 0.0));
        assert!((on_edge - Vec3::new(5.0, 0.0, 0.0)).length() < 1e-5);
        let inside = triangle.closest_point(&Vec3::new(2.0, 2.0, 7.0));
        assert!((inside - Vec3::new(2.0, 2.0, 0.0)).length() < 1e-5);
    }
}
