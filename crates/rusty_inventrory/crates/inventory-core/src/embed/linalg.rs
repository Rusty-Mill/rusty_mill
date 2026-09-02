//! The small amount of dense linear algebra the semantic index needs.
//!
//! Deliberately dependency-free: pulling in a BLAS would mean a system
//! library, and the whole point of this crate is that it runs with nothing
//! installed and nothing downloaded.

/// Column-major-free dense matrix, row-major `rows × cols`.
#[derive(Clone)]
pub struct Dense {
    pub rows: usize,
    pub cols: usize,
    pub data: Vec<f32>,
}

impl Dense {
    pub fn zeros(rows: usize, cols: usize) -> Self {
        Dense {
            rows,
            cols,
            data: vec![0.0; rows * cols],
        }
    }

    #[inline]
    pub fn row(&self, r: usize) -> &[f32] {
        &self.data[r * self.cols..(r + 1) * self.cols]
    }

    #[inline]
    pub fn row_mut(&mut self, r: usize) -> &mut [f32] {
        let c = self.cols;
        &mut self.data[r * c..(r + 1) * c]
    }
}

/// Sparse matrix in coordinate form, interpreted as `terms × docs`.
pub struct Coo {
    pub rows: usize,
    pub cols: usize,
    pub entries: Vec<(u32, u32, f32)>,
}

impl Coo {
    /// `out = A · x` where x is `cols × k` dense.
    pub fn mul_dense(&self, x: &Dense) -> Dense {
        debug_assert_eq!(x.rows, self.cols);
        let mut out = Dense::zeros(self.rows, x.cols);
        for &(r, c, v) in &self.entries {
            let (r, c) = (r as usize, c as usize);
            let xr = x.row(c);
            let orow = &mut out.data[r * x.cols..(r + 1) * x.cols];
            for j in 0..x.cols {
                orow[j] += v * xr[j];
            }
        }
        out
    }

    /// `out = Aᵀ · y` where y is `rows × k` dense.
    pub fn transpose_mul_dense(&self, y: &Dense) -> Dense {
        debug_assert_eq!(y.rows, self.rows);
        let mut out = Dense::zeros(self.cols, y.cols);
        for &(r, c, v) in &self.entries {
            let (r, c) = (r as usize, c as usize);
            let yr = y.row(r);
            let orow = &mut out.data[c * y.cols..(c + 1) * y.cols];
            for j in 0..y.cols {
                orow[j] += v * yr[j];
            }
        }
        out
    }
}

/// Modified Gram-Schmidt orthonormalisation of the columns of `m`, in place.
/// Numerically good enough at the ranks we use and far less code than a
/// Householder QR.
pub fn orthonormalize_columns(m: &mut Dense) {
    for j in 0..m.cols {
        for prev in 0..j {
            let mut dot = 0.0f32;
            for r in 0..m.rows {
                dot += m.data[r * m.cols + j] * m.data[r * m.cols + prev];
            }
            for r in 0..m.rows {
                m.data[r * m.cols + j] -= dot * m.data[r * m.cols + prev];
            }
        }
        let mut norm = 0.0f32;
        for r in 0..m.rows {
            let v = m.data[r * m.cols + j];
            norm += v * v;
        }
        norm = norm.sqrt();
        if norm > 1e-8 {
            for r in 0..m.rows {
                m.data[r * m.cols + j] /= norm;
            }
        } else {
            // Degenerate column: zero it so it contributes nothing downstream.
            for r in 0..m.rows {
                m.data[r * m.cols + j] = 0.0;
            }
        }
    }
}

/// Cyclic Jacobi eigendecomposition of a small symmetric matrix.
/// Returns `(eigenvalues, eigenvectors)` with eigenvectors in columns,
/// sorted by descending eigenvalue.
pub fn symmetric_eigen(mut a: Dense) -> (Vec<f32>, Dense) {
    let n = a.rows;
    debug_assert_eq!(a.rows, a.cols);
    let mut v = Dense::zeros(n, n);
    for i in 0..n {
        v.data[i * n + i] = 1.0;
    }

    for _sweep in 0..60 {
        let mut off = 0.0f32;
        for i in 0..n {
            for j in (i + 1)..n {
                off += a.data[i * n + j] * a.data[i * n + j];
            }
        }
        if off.sqrt() < 1e-7 {
            break;
        }

        for p in 0..n {
            for q in (p + 1)..n {
                let apq = a.data[p * n + q];
                if apq.abs() < 1e-9 {
                    continue;
                }
                let app = a.data[p * n + p];
                let aqq = a.data[q * n + q];
                let theta = (aqq - app) / (2.0 * apq);
                let t = theta.signum() / (theta.abs() + (theta * theta + 1.0).sqrt());
                let c = 1.0 / (t * t + 1.0).sqrt();
                let s = t * c;

                for k in 0..n {
                    let akp = a.data[k * n + p];
                    let akq = a.data[k * n + q];
                    a.data[k * n + p] = c * akp - s * akq;
                    a.data[k * n + q] = s * akp + c * akq;
                }
                for k in 0..n {
                    let apk = a.data[p * n + k];
                    let aqk = a.data[q * n + k];
                    a.data[p * n + k] = c * apk - s * aqk;
                    a.data[q * n + k] = s * apk + c * aqk;
                }
                for k in 0..n {
                    let vkp = v.data[k * n + p];
                    let vkq = v.data[k * n + q];
                    v.data[k * n + p] = c * vkp - s * vkq;
                    v.data[k * n + q] = s * vkp + c * vkq;
                }
            }
        }
    }

    let mut order: Vec<usize> = (0..n).collect();
    order.sort_by(|&x, &y| {
        a.data[y * n + y]
            .partial_cmp(&a.data[x * n + x])
            .unwrap_or(std::cmp::Ordering::Equal)
    });

    let eigenvalues: Vec<f32> = order.iter().map(|&i| a.data[i * n + i]).collect();
    let mut vectors = Dense::zeros(n, n);
    for (new_col, &old_col) in order.iter().enumerate() {
        for r in 0..n {
            vectors.data[r * n + new_col] = v.data[r * n + old_col];
        }
    }
    (eigenvalues, vectors)
}

pub fn l2_normalize(v: &mut [f32]) {
    let norm: f32 = v.iter().map(|x| x * x).sum::<f32>().sqrt();
    if norm > 1e-8 {
        for x in v.iter_mut() {
            *x /= norm;
        }
    }
}

pub fn cosine(a: &[f32], b: &[f32]) -> f32 {
    // Both sides are stored already normalised, so this is a plain dot.
    a.iter().zip(b).map(|(x, y)| x * y).sum()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn eigen_recovers_a_known_spectrum() {
        // diag(3,2,1) rotated is still {3,2,1}.
        let mut m = Dense::zeros(3, 3);
        m.data = vec![2.0, 1.0, 0.0, 1.0, 2.0, 0.0, 0.0, 0.0, 1.0];
        let (vals, _) = symmetric_eigen(m);
        // Eigenvalues of [[2,1],[1,2]] are 3 and 1, plus the standalone 1.
        assert!((vals[0] - 3.0).abs() < 1e-4, "{vals:?}");
        assert!((vals[1] - 1.0).abs() < 1e-4, "{vals:?}");
        assert!((vals[2] - 1.0).abs() < 1e-4, "{vals:?}");
    }

    #[test]
    fn orthonormalize_produces_orthonormal_columns() {
        let mut m = Dense::zeros(3, 2);
        m.data = vec![1.0, 1.0, 1.0, 0.0, 0.0, 1.0];
        orthonormalize_columns(&mut m);
        let col = |j: usize| (0..3).map(|r| m.data[r * 2 + j]).collect::<Vec<_>>();
        let (a, b) = (col(0), col(1));
        assert!((cosine(&a, &a) - 1.0).abs() < 1e-5);
        assert!(cosine(&a, &b).abs() < 1e-5);
    }

    #[test]
    fn sparse_dense_products_agree_with_hand_computation() {
        // A = [[1, 0], [0, 2]] (2 terms x 2 docs)
        let a = Coo {
            rows: 2,
            cols: 2,
            entries: vec![(0, 0, 1.0), (1, 1, 2.0)],
        };
        let mut x = Dense::zeros(2, 1);
        x.data = vec![3.0, 4.0];
        let y = a.mul_dense(&x);
        assert_eq!(y.data, vec![3.0, 8.0]);
        let z = a.transpose_mul_dense(&y);
        assert_eq!(z.data, vec![3.0, 16.0]);
    }
}
