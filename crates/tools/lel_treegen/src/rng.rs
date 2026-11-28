//! A hand-written, frozen PRNG: SplitMix64 seeding xoshiro256**.
//!
//! `rand`'s `StdRng` does not promise a stable algorithm across versions, and a
//! seed must name the same tree forever, so the generator carries its own.

/// SplitMix64 step: advances `state` and returns the next output.
fn splitmix64(state: &mut u64) -> u64 {
    *state = state.wrapping_add(0x9E37_79B9_7F4A_7C15);
    let mut z = *state;
    z = (z ^ (z >> 30)).wrapping_mul(0xBF58_476D_1CE4_E5B9);
    z = (z ^ (z >> 27)).wrapping_mul(0x94D0_49BB_1331_11EB);
    z ^ (z >> 31)
}

/// xoshiro256** generator. Each independent stream is keyed by
/// `(seed, stream, index)`, so a unit's draws never depend on how many units
/// came before it.
#[derive(Debug, Clone)]
pub struct Rng {
    s: [u64; 4],
}

impl Rng {
    /// The generator for one keyed stream.
    #[must_use]
    pub fn keyed(seed: u64, stream: u64, index: u64) -> Self {
        let mut key = seed;
        let mut mixed = splitmix64(&mut key);
        mixed ^= stream;
        let mixed2 = splitmix64(&mut mixed);
        let mut state = mixed2 ^ index;
        let s = [
            splitmix64(&mut state),
            splitmix64(&mut state),
            splitmix64(&mut state),
            splitmix64(&mut state),
        ];
        Self::from_state(s)
    }

    /// The generator with a raw state (all-zero is replaced, as it never leaves zero).
    #[must_use]
    pub fn from_state(s: [u64; 4]) -> Self {
        if s == [0; 4] {
            return Self { s: [1, 0, 0, 0] };
        }
        Self { s }
    }

    /// Next 64 random bits.
    pub fn next_u64(&mut self) -> u64 {
        let result = self.s[1].wrapping_mul(5).rotate_left(7).wrapping_mul(9);
        let t = self.s[1] << 17;
        self.s[2] ^= self.s[0];
        self.s[3] ^= self.s[1];
        self.s[1] ^= self.s[2];
        self.s[0] ^= self.s[3];
        self.s[2] ^= t;
        self.s[3] = self.s[3].rotate_left(45);
        result
    }

    /// Uniform in `[0, n)` without modulo bias (Lemire). `n == 0` returns 0.
    pub fn below(&mut self, n: u64) -> u64 {
        if n == 0 {
            return 0;
        }
        let threshold = n.wrapping_neg() % n;
        loop {
            let m = u128::from(self.next_u64()) * u128::from(n);
            // Truncation keeps the low 64 bits on purpose.
            #[allow(clippy::cast_possible_truncation)]
            let low = m as u64;
            if low >= threshold {
                return (m >> 64) as u64;
            }
        }
    }

    /// Uniform in `[lo, hi]` inclusive.
    pub fn range(&mut self, lo: u64, hi: u64) -> u64 {
        lo + self.below(hi - lo + 1)
    }

    /// Uniform in `[0, 1)` with 53 bits.
    pub fn unit(&mut self) -> f64 {
        // 53 random bits fit an f64 mantissa exactly.
        #[allow(clippy::cast_precision_loss)]
        let x = (self.next_u64() >> 11) as f64;
        x / (1u64 << 53) as f64
    }

    /// True with probability `p`.
    pub fn chance(&mut self, p: f64) -> bool {
        self.unit() < p
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn splitmix64_matches_reference() {
        let mut s = 0u64;
        assert_eq!(splitmix64(&mut s), 0xE220_A839_7B1D_CDAF);
        assert_eq!(splitmix64(&mut s), 0x6E78_9E6A_A1B9_65F4);
    }

    #[test]
    fn xoshiro256starstar_matches_reference() {
        let mut r = Rng::from_state([1, 2, 3, 4]);
        let got: Vec<u64> = (0..4).map(|_| r.next_u64()).collect();
        assert_eq!(got, [11520, 0, 1_509_978_240, 1_215_971_899_390_074_240]);
    }

    #[test]
    fn keyed_streams_are_stable_and_distinct() {
        let a = Rng::keyed(7, 1, 0).next_u64();
        assert_eq!(a, Rng::keyed(7, 1, 0).next_u64());
        assert_ne!(a, Rng::keyed(7, 1, 1).next_u64());
        assert_ne!(a, Rng::keyed(7, 2, 0).next_u64());
        assert_ne!(a, Rng::keyed(8, 1, 0).next_u64());
    }

    #[test]
    fn below_stays_in_range() {
        let mut r = Rng::keyed(1, 0, 0);
        assert!((0..10_000).all(|_| r.below(7) < 7));
        assert_eq!(r.below(0), 0);
        assert_eq!(r.range(5, 5), 5);
    }
}
