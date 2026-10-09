//! The order in which Bullet's quantized BVH reports a mesh's triangles
//! (`RB-PHYSICS-001-FR-117`, ADR-0037). `btBvhTriangleMeshShape` builds a
//! `btOptimizedBvh` over the triangles' quantized bounds and
//! `walkStacklessQuantizedTree` visits its leaves in array order, so every
//! query reports overlapping triangles in the leaf order the build's
//! in-place partitions leave. The ball's manifold keeps the last of
//! several triangles meeting it at one point (FR-116), so that order picks
//! the contact normal at a shared vertex.
//!
//! Ported operation for operation in `f32`, in Bullet's units, so the
//! quantization and partitions round as Bullet's do.

type V3 = [f32; 3];

/// `btQuantizedBvh`'s quantization frame: `m_bvhAabbMin` and
/// `m_bvhQuantization`.
struct Quantizer {
    min: V3,
    scale: V3,
}

fn map(a: V3, f: impl Fn(usize, f32) -> f32) -> V3 {
    [f(0, a[0]), f(1, a[1]), f(2, a[2])]
}

impl Quantizer {
    /// `setQuantizationValues` with the default 1.0 margin.
    fn new(min: V3, max: V3) -> Quantizer {
        const MARGIN: f32 = 1.0;
        let scale_for = |lo: V3, hi: V3| map(lo, |i, l| 65533.0 / (hi[i] - l));
        let mut lo = map(min, |_, v| v - MARGIN);
        let mut hi = map(max, |_, v| v + MARGIN);
        let mut quantizer = Quantizer {
            min: lo,
            scale: scale_for(lo, hi),
        };
        let back = quantizer.unquantize(quantizer.quantize(lo, false));
        lo = map(lo, |i, v| v.min(back[i] - MARGIN));
        quantizer = Quantizer {
            min: lo,
            scale: scale_for(lo, hi),
        };
        let back = quantizer.unquantize(quantizer.quantize(hi, true));
        hi = map(hi, |i, v| v.max(back[i] + MARGIN));
        Quantizer {
            min: lo,
            scale: scale_for(lo, hi),
        }
    }

    /// `quantize`: maxima round up to an odd step, minima down to even.
    fn quantize(&self, point: V3, is_max: bool) -> [u16; 3] {
        let v = map(point, |i, p| (p - self.min[i]) * self.scale[i]);
        v.map(|c| {
            if is_max {
                ((c + 1.0) as u16) | 1
            } else {
                (c as u16) & 0xfffe
            }
        })
    }

    /// `unQuantize`.
    fn unquantize(&self, q: [u16; 3]) -> V3 {
        map(self.min, |i, m| f32::from(q[i]) / self.scale[i] + m)
    }
}

/// One `btQuantizedBvhNode` leaf, its bounds as `getAabbMin`/`getAabbMax`
/// read them back.
#[derive(Clone, Copy)]
struct Leaf {
    min: V3,
    max: V3,
    triangle: usize,
}

impl Leaf {
    fn center(&self) -> V3 {
        map(self.max, |i, hi| 0.5 * (hi + self.min[i]))
    }
}

/// The indices of `triangles` (vertices in Bullet units, file order) in
/// the order Bullet's BVH reports them.
pub(crate) fn visit_order(triangles: &[[V3; 3]]) -> Vec<usize> {
    let Some(first) = triangles.first() else {
        return Vec::new();
    };
    let corners = || triangles.iter().flatten();
    let low = corners().fold(first[0], |acc, p| map(acc, |i, a| a.min(p[i])));
    let high = corners().fold(first[0], |acc, p| map(acc, |i, a| a.max(p[i])));
    let quantizer = Quantizer::new(low, high);
    let mut leaves: Vec<Leaf> = triangles
        .iter()
        .enumerate()
        .map(|(triangle, corners)| {
            let (min, max) = triangle_bounds(corners);
            Leaf {
                min: quantizer.unquantize(quantizer.quantize(min, false)),
                max: quantizer.unquantize(quantizer.quantize(max, true)),
                triangle,
            }
        })
        .collect();
    partition(&mut leaves);
    leaves.into_iter().map(|leaf| leaf.triangle).collect()
}

/// A triangle's bounds, each side at least `MIN_AABB_DIMENSION` wide.
fn triangle_bounds(corners: &[V3; 3]) -> (V3, V3) {
    const MIN_AABB_DIMENSION: f32 = 0.002;
    const MIN_AABB_HALF_DIMENSION: f32 = 0.001;
    let [a, b, c] = corners;
    let mut min = map(*a, |i, v| v.min(b[i]).min(c[i]));
    let mut max = map(*a, |i, v| v.max(b[i]).max(c[i]));
    for i in 0..3 {
        if max[i] - min[i] < MIN_AABB_DIMENSION {
            max[i] += MIN_AABB_HALF_DIMENSION;
            min[i] -= MIN_AABB_HALF_DIMENSION;
        }
    }
    (min, max)
}

/// `buildTree`'s reordering of `leaves`, left subtree first.
fn partition(leaves: &mut [Leaf]) {
    if leaves.len() < 2 {
        return;
    }
    let axis = splitting_axis(leaves);
    let split = sort_and_split(leaves, axis);
    let (left, right) = leaves.split_at_mut(split);
    partition(left);
    partition(right);
}

fn mean_center(leaves: &[Leaf]) -> V3 {
    let sum = leaves.iter().fold([0.0; 3], |acc: V3, leaf| {
        let center = leaf.center();
        map(acc, |i, a| a + center[i])
    });
    let inverse = 1.0 / leaves.len() as f32;
    map(sum, |_, s| s * inverse)
}

/// `calcSplittingAxis`: the axis of greatest center variance.
fn splitting_axis(leaves: &[Leaf]) -> usize {
    let means = mean_center(leaves);
    let sum = leaves.iter().fold([0.0; 3], |acc: V3, leaf| {
        let center = leaf.center();
        map(acc, |i, a| {
            let d = center[i] - means[i];
            a + d * d
        })
    });
    let inverse = 1.0 / (leaves.len() as f32 - 1.0);
    let v = map(sum, |_, s| s * inverse);
    // `btVector3::maxAxis`.
    if v[0] < v[1] {
        if v[1] < v[2] {
            2
        } else {
            1
        }
    } else if v[0] < v[2] {
        2
    } else {
        0
    }
}

/// `sortAndCalcSplittingIndex`: swaps leaves centered past the mean to the
/// front, falling back to the middle when that leaves a third or less on
/// one side.
fn sort_and_split(leaves: &mut [Leaf], axis: usize) -> usize {
    let split_value = mean_center(leaves)[axis];
    let mut split = 0;
    for i in 0..leaves.len() {
        if leaves[i].center()[axis] > split_value {
            leaves.swap(i, split);
            split += 1;
        }
    }
    let count = leaves.len();
    let balanced = count / 3;
    if split <= balanced || split >= count - 1 - balanced {
        count >> 1
    } else {
        split
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn flat(x: f32, y: f32) -> [V3; 3] {
        [[x, y, 0.0], [x + 1.0, y, 0.0], [x, y + 1.0, 0.0]]
    }

    #[test]
    fn an_empty_mesh_has_no_order() {
        assert!(visit_order(&[]).is_empty());
    }

    #[test]
    fn triangles_past_the_mean_on_the_widest_axis_come_first() {
        // Spread along x: the two right of the mean swap to the front in
        // scan order, then each half splits again.
        let order = visit_order(&[
            flat(0.0, 0.0),
            flat(10.0, 0.0),
            flat(20.0, 0.0),
            flat(30.0, 0.0),
        ]);
        assert_eq!(order, vec![3, 2, 1, 0]);
    }

    #[test]
    fn a_lopsided_split_falls_back_to_the_middle() {
        // One far triangle alone past the mean is a third or less, so the
        // split is the middle instead: [far, 1] | [2, 0] after the swap.
        let order = visit_order(&[
            flat(0.0, 0.0),
            flat(1.0, 0.0),
            flat(2.0, 0.0),
            flat(100.0, 0.0),
        ]);
        assert_eq!(order.len(), 4);
        assert_eq!(order[0], 3);
    }

    #[test]
    fn quantized_bounds_contain_the_triangle() {
        let quantizer = Quantizer::new([0.0; 3], [50.0, 10.0, 1.0]);
        let point = [12.345, 6.789, 0.5];
        let lo = quantizer.unquantize(quantizer.quantize(point, false));
        let hi = quantizer.unquantize(quantizer.quantize(point, true));
        for i in 0..3 {
            assert!(lo[i] <= point[i] && point[i] <= hi[i]);
        }
    }
}
