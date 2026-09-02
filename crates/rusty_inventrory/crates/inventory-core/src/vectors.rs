//! Making semantic search scale.
//!
//! Two layers, and the first one matters more than the second.
//!
//! # 1. The vectors live in memory
//!
//! The original implementation read every embedding blob out of SQLite and
//! decoded it on **every query** — as-you-type, that is the same megabytes
//! re-read and re-parsed per keystroke. [`VectorSet`] loads them once into one
//! contiguous `Vec<f32>` and scores against that. The scan itself was never
//! the expensive part; the I/O and the decode were.
//!
//! # 2. An IVF index narrows the scan
//!
//! Above [`MIN_VECTORS_FOR_INDEX`] the set is clustered, and a query scans
//! only the nearest clusters. Three rules, all borrowed from the design in
//! `rusty_remind_me`'s `ann_index.rs`, because they are what makes an
//! approximate index safe to put in front of an exact one:
//!
//! * **It is never a source of truth.** Every failure mode — no index, a
//!   stale one, too few survivors after filtering — falls back to the exact
//!   scan. A search must never fail, or silently return less, because an
//!   optimisation was unavailable.
//! * **It narrows candidates; it does not score them.** The index picks a
//!   shortlist, then exact cosines are computed over that much smaller set.
//!   So scores are identical to the brute-force ones and RRF fusion upstream
//!   cannot tell whether the index ran.
//! * **Staleness is detected, not assumed away.** The index records the model
//!   and vector count it was built from. Either changing means it is ignored.
//!
//! Clustering is plain k-means in this crate rather than a bound C++ ANN
//! library: it reuses the arithmetic already here for the embedder, and keeps
//! the promise that this builds with nothing installed.

use crate::embed::{decode_vector, linalg::l2_normalize};
use crate::Result;
use rand::{Rng, SeedableRng};
use rusqlite::Connection;
use std::collections::HashSet;

/// Below this, the exact scan over in-memory vectors is already fast enough
/// that an index would only add a way to be wrong.
///
/// Measured with `examples/vector_bench.rs` at 128 dimensions — time for one
/// exact scan of the whole set:
///
/// | vectors | per query |
/// |---------|-----------|
/// | 5,000   | 0.6 ms    |
/// | 20,000  | 2.7 ms    |
/// | 50,000  | 7.0 ms    |
/// | 200,000 | 29 ms     |
/// | 1,000,000 | 141 ms  |
///
/// A heavy user with years of history across six tools lands in the tens of
/// thousands, where the exact scan is comfortably inside an as-you-type
/// budget. The index earns its recall risk only past that, which is where
/// this threshold sits.
pub const MIN_VECTORS_FOR_INDEX: usize = 50_000;

/// Candidates pulled per result wanted, before filtering and exact scoring.
///
/// Enough that a source filter removing most hits still leaves a full page;
/// small enough that exact scoring stays trivial.
pub const OVERFETCH: usize = 8;

const KMEANS_ITERATIONS: usize = 8;
/// Fraction of clusters probed at minimum, as a divisor.
///
/// With `k = sqrt(n)` clusters of roughly `sqrt(n)` members each, filling an
/// 80-candidate shortlist takes barely one cluster — and a shortlist drawn
/// from one cluster has poor recall, because a query near a boundary has its
/// true neighbours split across several. Probing an eighth of the clusters
/// scans an eighth of the data: still a large speed-up, with recall that
/// holds up.
const MIN_PROBE_DIVISOR: usize = 8;
const MIN_PROBES: usize = 8;
/// Fixed so that indexing the same corpus twice produces the same clusters.
const SEED: u64 = 0x5EED_1F5A;

/// Every stored vector, contiguous and resident.
pub struct VectorSet {
    ids: Vec<i64>,
    /// `ids.len() * dim`, row-major, each row L2-normalised.
    data: Vec<f32>,
    dim: usize,
    model: String,
}

impl VectorSet {
    pub fn load(conn: &Connection, model: &str) -> Result<VectorSet> {
        let mut stmt =
            conn.prepare("SELECT conversation_id, vec FROM embeddings WHERE model = ?1")?;
        let rows = stmt.query_map([model], |r| {
            Ok((r.get::<_, i64>(0)?, r.get::<_, Vec<u8>>(1)?))
        })?;

        let mut ids = Vec::new();
        let mut data: Vec<f32> = Vec::new();
        let mut dim = 0usize;

        for (id, blob) in rows.flatten() {
            let v = decode_vector(&blob);
            if v.is_empty() {
                continue;
            }
            if dim == 0 {
                dim = v.len();
            }
            // A row at a different dimension is from a superseded model and
            // cannot be compared with the rest; skipping beats poisoning the
            // whole set.
            if v.len() != dim {
                continue;
            }
            ids.push(id);
            data.extend_from_slice(&v);
        }

        Ok(VectorSet {
            ids,
            data,
            dim,
            model: model.to_string(),
        })
    }

    pub fn len(&self) -> usize {
        self.ids.len()
    }

    pub fn is_empty(&self) -> bool {
        self.ids.is_empty()
    }

    pub fn dim(&self) -> usize {
        self.dim
    }

    pub fn model(&self) -> &str {
        &self.model
    }

    #[inline]
    fn row(&self, i: usize) -> &[f32] {
        &self.data[i * self.dim..(i + 1) * self.dim]
    }

    #[inline]
    fn score(&self, i: usize, query: &[f32]) -> f32 {
        // Both sides are stored normalised, so a dot product is the cosine.
        self.row(i).iter().zip(query).map(|(a, b)| a * b).sum()
    }

    /// Exact top-`k`, scanning everything. The reference implementation, and
    /// the fallback for every failure of the index.
    pub fn top_k_exact(
        &self,
        query: &[f32],
        k: usize,
        allow: Option<&HashSet<i64>>,
        floor: f32,
    ) -> Vec<(i64, f32)> {
        self.rank((0..self.len()).map(|i| i as u32), query, k, allow, floor)
    }

    /// Exact top-`k` over an index-supplied shortlist. Same scores as
    /// [`VectorSet::top_k_exact`] — only the set scanned differs.
    pub fn top_k_over(
        &self,
        candidates: &[u32],
        query: &[f32],
        k: usize,
        allow: Option<&HashSet<i64>>,
        floor: f32,
    ) -> Vec<(i64, f32)> {
        self.rank(candidates.iter().copied(), query, k, allow, floor)
    }

    fn rank(
        &self,
        rows: impl Iterator<Item = u32>,
        query: &[f32],
        k: usize,
        allow: Option<&HashSet<i64>>,
        floor: f32,
    ) -> Vec<(i64, f32)> {
        if query.len() != self.dim || self.is_empty() {
            return Vec::new();
        }
        let mut scored: Vec<(i64, f32)> = rows
            .filter_map(|i| {
                let i = i as usize;
                let id = *self.ids.get(i)?;
                if allow.is_some_and(|a| !a.contains(&id)) {
                    return None;
                }
                let s = self.score(i, query);
                (s > floor).then_some((id, s))
            })
            .collect();

        scored.sort_by(|a, b| b.1.partial_cmp(&a.1).unwrap_or(std::cmp::Ordering::Equal));
        scored.truncate(k);
        scored
    }
}

/// Inverted-file index: k-means centroids plus the rows assigned to each.
pub struct IvfIndex {
    /// `centroids × dim`, row-major, normalised.
    centroids: Vec<f32>,
    lists: Vec<Vec<u32>>,
    dim: usize,
    /// Staleness key: what the index was built from.
    built_from_vectors: usize,
    built_from_model: String,
}

impl IvfIndex {
    /// Cluster a set. `None` when the set is too small to be worth indexing.
    pub fn build(set: &VectorSet) -> Option<IvfIndex> {
        IvfIndex::build_with_minimum(set, MIN_VECTORS_FOR_INDEX)
    }

    /// As [`IvfIndex::build`], with an explicit size threshold. Exists so the
    /// tests can exercise the index without clustering fifty thousand vectors.
    pub fn build_with_minimum(set: &VectorSet, minimum: usize) -> Option<IvfIndex> {
        if set.len() < minimum || set.dim == 0 || set.len() < 4 {
            return None;
        }
        let k = (set.len() as f64).sqrt().round() as usize;
        let k = k.clamp(8, 4096).min(set.len() / 2);

        let mut rng = rand_chacha::ChaCha8Rng::seed_from_u64(SEED);
        // Seed centroids by an even stride through the set, jittered. Cheaper
        // than k-means++ and adequate: the vectors are already spread over a
        // unit sphere by construction.
        let stride = set.len() / k;
        let mut centroids = vec![0.0f32; k * set.dim];
        for c in 0..k {
            let pick = (c * stride + rng.gen_range(0..stride.max(1))).min(set.len() - 1);
            centroids[c * set.dim..(c + 1) * set.dim].copy_from_slice(set.row(pick));
        }

        let mut assignment = vec![0u32; set.len()];
        for _ in 0..KMEANS_ITERATIONS {
            // Assign.
            let mut moved = false;
            for (i, slot) in assignment.iter_mut().enumerate() {
                let row = set.row(i);
                let mut best = 0usize;
                let mut best_score = f32::MIN;
                for c in 0..k {
                    let s: f32 = centroids[c * set.dim..(c + 1) * set.dim]
                        .iter()
                        .zip(row)
                        .map(|(a, b)| a * b)
                        .sum();
                    if s > best_score {
                        best_score = s;
                        best = c;
                    }
                }
                if *slot != best as u32 {
                    *slot = best as u32;
                    moved = true;
                }
            }
            if !moved {
                break;
            }

            // Update: mean of members, renormalised so dot stays cosine.
            let mut sums = vec![0.0f32; k * set.dim];
            let mut counts = vec![0usize; k];
            for (i, &c) in assignment.iter().enumerate() {
                let c = c as usize;
                counts[c] += 1;
                let row = set.row(i);
                let dst = &mut sums[c * set.dim..(c + 1) * set.dim];
                for (d, v) in dst.iter_mut().zip(row) {
                    *d += v;
                }
            }
            for c in 0..k {
                if counts[c] == 0 {
                    continue; // keep the old centroid rather than collapsing
                }
                let dst = &mut sums[c * set.dim..(c + 1) * set.dim];
                l2_normalize(dst);
                centroids[c * set.dim..(c + 1) * set.dim].copy_from_slice(dst);
            }
        }

        let mut lists = vec![Vec::new(); k];
        for (i, &c) in assignment.iter().enumerate() {
            lists[c as usize].push(i as u32);
        }

        Some(IvfIndex {
            centroids,
            lists,
            dim: set.dim,
            built_from_vectors: set.len(),
            built_from_model: set.model.clone(),
        })
    }

    /// Does this index still describe the set it is being used with?
    ///
    /// A stale index returning deleted conversations is worse than no index,
    /// because the results look plausible.
    pub fn matches(&self, set: &VectorSet) -> bool {
        self.dim == set.dim
            && self.built_from_vectors == set.len()
            && self.built_from_model == set.model
    }

    pub fn clusters(&self) -> usize {
        self.lists.len()
    }

    /// Rows worth scoring exactly, nearest clusters first.
    pub fn candidates(&self, query: &[f32], want: usize) -> Vec<u32> {
        if query.len() != self.dim {
            return Vec::new();
        }
        let mut by_centroid: Vec<(usize, f32)> = (0..self.lists.len())
            .map(|c| {
                let s: f32 = self.centroids[c * self.dim..(c + 1) * self.dim]
                    .iter()
                    .zip(query)
                    .map(|(a, b)| a * b)
                    .sum();
                (c, s)
            })
            .collect();
        by_centroid.sort_by(|a, b| b.1.partial_cmp(&a.1).unwrap_or(std::cmp::Ordering::Equal));

        // Probe until there are enough candidates, but always probe a few
        // clusters: a query landing near a boundary has its true neighbours
        // split across several.
        let min_probes = MIN_PROBES
            .max(self.lists.len() / MIN_PROBE_DIVISOR)
            .min(self.lists.len());
        let mut out = Vec::with_capacity(want);
        for (probed, (c, _)) in by_centroid.into_iter().enumerate() {
            out.extend_from_slice(&self.lists[c]);
            if out.len() >= want && probed + 1 >= min_probes {
                break;
            }
        }
        out
    }

    // --- persistence -------------------------------------------------------

    pub fn serialize(&self) -> Vec<u8> {
        let mut out = Vec::new();
        let put = |out: &mut Vec<u8>, v: u32| out.extend_from_slice(&v.to_le_bytes());
        put(&mut out, self.dim as u32);
        put(&mut out, self.lists.len() as u32);
        put(&mut out, self.built_from_vectors as u32);
        put(&mut out, self.built_from_model.len() as u32);
        out.extend_from_slice(self.built_from_model.as_bytes());
        for f in &self.centroids {
            out.extend_from_slice(&f.to_le_bytes());
        }
        for list in &self.lists {
            put(&mut out, list.len() as u32);
            for row in list {
                put(&mut out, *row);
            }
        }
        out
    }

    pub fn deserialize(bytes: &[u8]) -> Option<IvfIndex> {
        let mut at = 0usize;
        let take = |at: &mut usize| -> Option<u32> {
            let b = bytes.get(*at..*at + 4)?;
            *at += 4;
            Some(u32::from_le_bytes(b.try_into().ok()?))
        };
        let dim = take(&mut at)? as usize;
        let k = take(&mut at)? as usize;
        let built_from_vectors = take(&mut at)? as usize;
        let name_len = take(&mut at)? as usize;
        let built_from_model = std::str::from_utf8(bytes.get(at..at + name_len)?)
            .ok()?
            .to_string();
        at += name_len;

        let float_bytes = k * dim * 4;
        let centroids: Vec<f32> = bytes
            .get(at..at + float_bytes)?
            .chunks_exact(4)
            .map(|c| f32::from_le_bytes(c.try_into().unwrap()))
            .collect();
        at += float_bytes;

        let mut lists = Vec::with_capacity(k);
        for _ in 0..k {
            let len = take(&mut at)? as usize;
            let mut list = Vec::with_capacity(len);
            for _ in 0..len {
                list.push(take(&mut at)?);
            }
            lists.push(list);
        }

        Some(IvfIndex {
            centroids,
            lists,
            dim,
            built_from_vectors,
            built_from_model,
        })
    }
}

/// The in-memory search state, rebuilt whenever embeddings change.
pub struct VectorCache {
    pub set: VectorSet,
    pub index: Option<IvfIndex>,
}

impl VectorCache {
    /// Top-`k` conversation ids by cosine, using the index when it is usable
    /// and the exact scan whenever it is not.
    pub fn search(
        &self,
        query: &[f32],
        k: usize,
        allow: Option<&HashSet<i64>>,
        floor: f32,
    ) -> Vec<(i64, f32)> {
        let Some(index) = self.index.as_ref().filter(|i| i.matches(&self.set)) else {
            return self.set.top_k_exact(query, k, allow, floor);
        };

        let candidates = index.candidates(query, k * OVERFETCH);
        let hits = self.set.top_k_over(&candidates, query, k, allow, floor);

        // Over-fetching is what makes filtering safe — but if the filter still
        // took the shortlist below a full page, the index has not been given
        // enough to work with. Falling back costs one scan; not falling back
        // returns fewer results than the exact path would, which nobody would
        // notice was wrong.
        if hits.len() < k {
            return self.set.top_k_exact(query, k, allow, floor);
        }
        hits
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn unit(dim: usize, seed: u64) -> Vec<f32> {
        let mut rng = rand_chacha::ChaCha8Rng::seed_from_u64(seed);
        let mut v: Vec<f32> = (0..dim).map(|_| rng.gen_range(-1.0f32..1.0)).collect();
        l2_normalize(&mut v);
        v
    }

    /// Vectors scattered around a handful of centres — what a trained
    /// embedding model actually produces, and what an IVF index exists to
    /// exploit. Uniform random vectors (see `synthetic_set`) are the
    /// pathological case: with no cluster structure there is nothing for any
    /// approximate index to find, which is why the threshold above keeps the
    /// index away from sets the exact scan handles comfortably.
    fn clustered_set(n: usize, dim: usize, clusters: usize) -> VectorSet {
        let centres: Vec<Vec<f32>> = (0..clusters).map(|c| unit(dim, 7_000 + c as u64)).collect();
        let mut rng = rand_chacha::ChaCha8Rng::seed_from_u64(99);
        let mut ids = Vec::new();
        let mut data = Vec::new();
        for i in 0..n {
            let centre = &centres[i % clusters];
            let mut v: Vec<f32> = centre
                .iter()
                .map(|c| c + rng.gen_range(-0.35f32..0.35))
                .collect();
            l2_normalize(&mut v);
            ids.push(i as i64 + 1);
            data.extend_from_slice(&v);
        }
        VectorSet {
            ids,
            data,
            dim,
            model: "test-model".into(),
        }
    }

    fn synthetic_set(n: usize, dim: usize) -> VectorSet {
        let mut ids = Vec::new();
        let mut data = Vec::new();
        for i in 0..n {
            ids.push(i as i64 + 1);
            data.extend_from_slice(&unit(dim, i as u64));
        }
        VectorSet {
            ids,
            data,
            dim,
            model: "test-model".into(),
        }
    }

    #[test]
    fn exact_search_ranks_by_cosine() {
        let set = synthetic_set(200, 32);
        let query = set.row(7).to_vec();
        let hits = set.top_k_exact(&query, 5, None, -1.0);
        assert_eq!(hits[0].0, 8, "a vector should be its own nearest neighbour");
        assert!((hits[0].1 - 1.0).abs() < 1e-4);
        // Monotonically decreasing.
        for pair in hits.windows(2) {
            assert!(pair[0].1 >= pair[1].1);
        }
    }

    #[test]
    fn the_source_filter_is_honoured() {
        let set = synthetic_set(100, 16);
        let allow: HashSet<i64> = [10i64, 20, 30].into_iter().collect();
        let hits = set.top_k_exact(&unit(16, 999), 10, Some(&allow), -1.0);
        assert!(hits.len() <= 3);
        assert!(hits.iter().all(|(id, _)| allow.contains(id)));
    }

    #[test]
    fn a_set_too_small_to_index_declines() {
        // Well under the production threshold, so the exact scan stands.
        let set = synthetic_set(100, 16);
        assert!(IvfIndex::build(&set).is_none());
    }

    /// The index must agree with the exact scan on the overwhelming majority
    /// of queries — and wherever it does agree, the scores must be identical,
    /// because the index only ever narrows what gets scored.
    ///
    /// Queries are perturbations of real members, which is what a query
    /// embedding actually is: text from the same space as the corpus, landing
    /// near the documents it relates to. A uniform-random query is not a
    /// realistic case — it sits far from every cluster, so which cluster is
    /// "nearest" is close to arbitrary.
    #[test]
    fn the_index_recalls_what_the_exact_scan_finds() {
        let set = clustered_set(4_000, 32, 24);
        let index = IvfIndex::build_with_minimum(&set, 1_000).expect("large enough to index");
        assert!(index.clusters() >= 8);
        let cache = VectorCache {
            set,
            index: Some(index),
        };

        let mut rng = rand_chacha::ChaCha8Rng::seed_from_u64(555);
        let mut top1 = 0usize;
        let mut overlap = 0usize;
        let trials = 60;

        for t in 0..trials {
            let anchor = (t * 61) % cache.set.len();
            let mut q: Vec<f32> = cache
                .set
                .row(anchor)
                .iter()
                .map(|v| v + rng.gen_range(-0.25f32..0.25))
                .collect();
            l2_normalize(&mut q);

            let exact = cache.set.top_k_exact(&q, 10, None, -1.0);
            let viaidx = cache.search(&q, 10, None, -1.0);
            assert_eq!(viaidx.len(), exact.len());

            // Anything the index did return was scored exactly.
            for (id, score) in &viaidx {
                if let Some((_, truth)) = exact.iter().find(|(i, _)| i == id) {
                    assert!((truth - score).abs() < 1e-5, "index rescored a hit");
                }
            }

            if viaidx[0].0 == exact[0].0 {
                top1 += 1;
            }
            let exact_ids: HashSet<i64> = exact.iter().map(|(i, _)| *i).collect();
            overlap += viaidx.iter().filter(|(i, _)| exact_ids.contains(i)).count();
        }

        let top1_pct = top1 * 100 / trials;
        let recall_pct = overlap * 100 / (trials * 10);
        assert!(
            top1_pct >= 90,
            "top hit agreed on only {top1}/{trials} queries ({top1_pct}%)"
        );
        assert!(recall_pct >= 85, "top-10 recall was only {recall_pct}%");
    }

    /// The specific failure the staleness key exists to prevent.
    #[test]
    fn a_stale_index_is_ignored_rather_than_trusted() {
        let set = clustered_set(2_000, 24, 12);
        let index = IvfIndex::build_with_minimum(&set, 1_000).unwrap();
        assert!(index.matches(&set));

        // Conversations were added or removed since the build.
        let grown = clustered_set(2_100, 24, 12);
        assert!(!index.matches(&grown), "count change not detected");

        // The model was retrained, so the vector space is different.
        let mut retrained = clustered_set(2_000, 24, 12);
        retrained.model = "lsa-v2".into();
        assert!(!index.matches(&retrained), "model change not detected");

        // And a stale index still returns exact results, via the fallback.
        let cache = VectorCache {
            set: grown,
            index: Some(index),
        };
        let q = unit(24, 4242);
        assert_eq!(
            cache.search(&q, 5, None, -1.0),
            cache.set.top_k_exact(&q, 5, None, -1.0)
        );
    }

    /// A filter that guts the shortlist must fall back, not return a short page.
    #[test]
    fn a_filter_that_empties_the_shortlist_falls_back_to_the_exact_scan() {
        let set = clustered_set(2_000, 24, 12);
        let index = IvfIndex::build_with_minimum(&set, 1_000).unwrap();
        let cache = VectorCache {
            set,
            index: Some(index),
        };

        // Ten ids scattered across the whole set — almost certainly not all in
        // the clusters nearest any one query.
        let allow: HashSet<i64> = (0..10).map(|i| i * 173 + 1).collect();
        let q = unit(24, 77);
        let viaidx = cache.search(&q, 10, Some(&allow), -1.0);
        let exact = cache.set.top_k_exact(&q, 10, Some(&allow), -1.0);
        assert_eq!(
            viaidx, exact,
            "filtered search disagreed with the exact scan"
        );
    }

    #[test]
    fn the_index_survives_a_round_trip() {
        let set = clustered_set(1_500, 16, 10);
        let index = IvfIndex::build_with_minimum(&set, 1_000).unwrap();
        let restored = IvfIndex::deserialize(&index.serialize()).expect("round trips");

        assert_eq!(restored.clusters(), index.clusters());
        assert!(restored.matches(&set));
        let q = unit(16, 31337);
        assert_eq!(restored.candidates(&q, 80), index.candidates(&q, 80));
    }

    #[test]
    fn a_mismatched_query_dimension_returns_nothing_rather_than_garbage() {
        let set = synthetic_set(50, 16);
        assert!(set.top_k_exact(&unit(32, 1), 5, None, -1.0).is_empty());
    }
}
