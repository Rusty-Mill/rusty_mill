//! Seeded randomness: [`SplitMix64`] and counter-based seed derivation.
//!
//! The workspace has no exported seeded PRNG (`rusty_rand` is an OS CSPRNG),
//! and reproducible lineage needs one, so it lives here. SplitMix64 (Steele,
//! Lea and Flood, 2014) is small, fast and passes BigCrush; it is not
//! cryptographic and is not used for anything secret.

const GOLDEN_GAMMA: u64 = 0x9E37_79B9_7F4A_7C15;

/// The SplitMix64 output function: a bijective 64-bit mixer.
#[must_use]
pub const fn mix(mut z: u64) -> u64 {
    z = (z ^ (z >> 30)).wrapping_mul(0xBF58_476D_1CE4_E5B9);
    z = (z ^ (z >> 27)).wrapping_mul(0x94D0_49BB_1331_11EB);
    z ^ (z >> 31)
}

/// A seed for one evaluation of one task, recorded in lineage.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub struct Seed(u64);

impl Seed {
    /// Wraps a raw seed value.
    #[must_use]
    pub const fn new(value: u64) -> Self {
        Self(value)
    }

    /// The raw seed value.
    #[must_use]
    pub const fn get(self) -> u64 {
        self.0
    }
}

/// Derives a seed from `root` and a path of counters (for example
/// `[candidate, round, index]`).
///
/// Derivation is a pure function, so a lineage entry's seeds can be
/// recomputed and checked rather than trusted.
#[must_use]
pub fn derive_seed(root: Seed, path: &[u64]) -> Seed {
    let mut acc = mix(root.get());
    for &part in path {
        acc = mix(acc ^ mix(part.wrapping_add(GOLDEN_GAMMA)));
    }
    Seed(acc)
}

/// The `count` seeds for evaluation round `round` of candidate `candidate`.
///
/// Different `(candidate, round)` pairs give independent seed streams, which
/// is how the accept gate gets fresh seeds; [`crate::confirm`] still checks
/// disjointness rather than relying on it.
#[must_use]
pub fn seed_set(root: Seed, candidate: u64, round: u32, count: usize) -> Vec<Seed> {
    (0..count as u64)
        .map(|index| derive_seed(root, &[candidate, u64::from(round), index]))
        .collect()
}

/// The SplitMix64 generator.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct SplitMix64 {
    state: u64,
}

impl SplitMix64 {
    /// A generator starting from `seed`.
    #[must_use]
    pub const fn new(seed: Seed) -> Self {
        Self { state: seed.get() }
    }

    /// The next 64 uniformly distributed bits.
    pub fn next_u64(&mut self) -> u64 {
        self.state = self.state.wrapping_add(GOLDEN_GAMMA);
        mix(self.state)
    }

    /// A uniform `f64` in `[0, 1)`, using the top 53 bits.
    pub fn next_f64(&mut self) -> f64 {
        const SCALE: f64 = 1.0 / (1u64 << 53) as f64;
        (self.next_u64() >> 11) as f64 * SCALE
    }

    /// A uniform integer in `[0, bound)`, without modulo bias.
    ///
    /// Returns `None` when `bound` is zero, since the range is empty.
    pub fn below(&mut self, bound: u64) -> Option<u64> {
        if bound == 0 {
            return None;
        }
        // Reject the low `2^64 mod bound` values so every residue is equally likely.
        let threshold = bound.wrapping_neg() % bound;
        loop {
            let value = self.next_u64();
            if value >= threshold {
                return Some(value % bound);
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn matches_reference_splitmix64_stream() {
        // Reference values for seed 0 from the published SplitMix64 algorithm.
        let mut rng = SplitMix64::new(Seed::new(0));
        assert_eq!(rng.next_u64(), 0xE220_A839_7B1D_CDAF);
        assert_eq!(rng.next_u64(), 0x6E78_9E6A_A1B9_65F4);
        assert_eq!(rng.next_u64(), 0x06C4_5D18_8009_454F);
    }

    #[test]
    fn same_seed_same_stream() {
        let mut a = SplitMix64::new(Seed::new(42));
        let mut b = SplitMix64::new(Seed::new(42));
        for _ in 0..100 {
            assert_eq!(a.next_u64(), b.next_u64());
        }
    }

    #[test]
    fn next_f64_stays_in_unit_interval() {
        let mut rng = SplitMix64::new(Seed::new(7));
        for _ in 0..10_000 {
            let x = rng.next_f64();
            assert!((0.0..1.0).contains(&x), "{x}");
        }
    }

    #[test]
    fn below_zero_is_none_and_one_is_zero() {
        let mut rng = SplitMix64::new(Seed::new(1));
        assert_eq!(rng.below(0), None);
        assert_eq!(rng.below(1), Some(0));
    }

    #[test]
    fn below_covers_range_roughly_uniformly() {
        let mut rng = SplitMix64::new(Seed::new(3));
        let mut counts = [0u32; 6];
        for _ in 0..60_000 {
            let i = rng.below(6).expect("bound is nonzero");
            counts[i as usize] += 1;
        }
        for count in counts {
            assert!((9_000..11_000).contains(&count), "{counts:?}");
        }
    }

    #[test]
    fn below_handles_largest_bound() {
        let mut rng = SplitMix64::new(Seed::new(9));
        let value = rng.below(u64::MAX).expect("bound is nonzero");
        assert!(value < u64::MAX);
    }

    #[test]
    fn derived_seeds_depend_on_every_input() {
        let root = Seed::new(11);
        let base = derive_seed(root, &[1, 2, 3]);
        assert_eq!(base, derive_seed(root, &[1, 2, 3]));
        assert_ne!(base, derive_seed(Seed::new(12), &[1, 2, 3]));
        assert_ne!(base, derive_seed(root, &[1, 2, 4]));
        assert_ne!(base, derive_seed(root, &[2, 1, 3]));
        assert_ne!(base, derive_seed(root, &[1, 2]));
    }

    #[test]
    fn seed_sets_for_different_rounds_are_disjoint() {
        let root = Seed::new(5);
        let first = seed_set(root, 4, 0, 32);
        let fresh = seed_set(root, 4, 1, 32);
        assert_eq!(first.len(), 32);
        assert!(first.iter().all(|seed| !fresh.contains(seed)));
        assert_eq!(first, seed_set(root, 4, 0, 32));
    }

    #[test]
    fn empty_seed_set() {
        assert!(seed_set(Seed::new(0), 0, 0, 0).is_empty());
    }
}
