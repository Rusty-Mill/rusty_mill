//! Differential tests against `ring` (a dev-dependency oracle, never shipped).
//! A divergence is a finding until proven otherwise.

use ring::{digest, hkdf, hmac};
use rusty_sha2::{extract, Hash, Hmac, Sha256, Sha384, Sha512};

/// xorshift64*: deterministic, seedable, good enough for test inputs.
struct Rng(u64);
impl Rng {
    fn next(&mut self) -> u64 {
        self.0 ^= self.0 >> 12;
        self.0 ^= self.0 << 25;
        self.0 ^= self.0 >> 27;
        self.0.wrapping_mul(0x2545_f491_4f6c_dd1d)
    }
    fn bytes(&mut self, len: usize) -> Vec<u8> {
        (0..len).map(|_| (self.next() >> 32) as u8).collect()
    }
    /// Random bytes of random length `0..max`.
    fn vec(&mut self, max: usize) -> Vec<u8> {
        let len = self.below(max);
        self.bytes(len)
    }
    fn below(&mut self, n: usize) -> usize {
        (self.next() % n as u64) as usize
    }
}

fn check_hash<H: Hash>(alg: &'static digest::Algorithm, rng: &mut Rng) {
    for len in 0..=300 {
        let data = rng.bytes(len);
        assert_eq!(
            H::digest(&data).as_ref(),
            digest::digest(alg, &data).as_ref(),
            "len {len}"
        );
        // Incremental, split at a random point.
        let cut = rng.below(len + 1);
        let mut hasher = H::new();
        hasher.update(&data[..cut]);
        hasher.update(&data[cut..]);
        assert_eq!(
            hasher.finalize().as_ref(),
            digest::digest(alg, &data).as_ref()
        );
    }
    for len in [1000, 4096, 65_537] {
        let data = rng.bytes(len);
        assert_eq!(
            H::digest(&data).as_ref(),
            digest::digest(alg, &data).as_ref()
        );
    }
}

#[test]
fn digests_match_ring() {
    let mut rng = Rng(0x1234_5678_9abc_def1);
    check_hash::<Sha256>(&digest::SHA256, &mut rng);
    check_hash::<Sha384>(&digest::SHA384, &mut rng);
    check_hash::<Sha512>(&digest::SHA512, &mut rng);
}

fn check_hmac<H: Hash>(alg: hmac::Algorithm, rng: &mut Rng) {
    // Key lengths straddle the block size, where HMAC hashes the key first.
    for key_len in (0..=200).chain([255, 256, 1000]) {
        let key = rng.bytes(key_len);
        let msg = rng.vec(300);
        let theirs = hmac::sign(&hmac::Key::new(alg, &key), &msg);
        assert_eq!(
            Hmac::<H>::mac(&key, &msg).as_ref(),
            theirs.as_ref(),
            "key {key_len}"
        );
    }
}

#[test]
fn hmac_matches_ring() {
    let mut rng = Rng(0x0bad_cafe_f00d_1234);
    check_hmac::<Sha256>(hmac::HMAC_SHA256, &mut rng);
    check_hmac::<Sha384>(hmac::HMAC_SHA384, &mut rng);
    check_hmac::<Sha512>(hmac::HMAC_SHA512, &mut rng);
}

struct Len(usize);
impl hkdf::KeyType for Len {
    fn len(&self) -> usize {
        self.0
    }
}

fn check_hkdf<H: Hash>(alg: hkdf::Algorithm, rng: &mut Rng) {
    for _ in 0..200 {
        let salt = rng.vec(150);
        let ikm = rng.vec(100);
        let info = rng.vec(100);
        let size = rng.below(16 * H::OUTPUT_LEN + 1);
        let mut theirs = vec![0u8; size];
        let prk = hkdf::Salt::new(alg, &salt).extract(&ikm);
        let info_parts = [info.as_slice()];
        prk.expand(&info_parts, Len(size))
            .unwrap()
            .fill(&mut theirs)
            .unwrap();
        let mut ours = vec![0u8; size];
        extract::<H>(Some(&salt), &ikm)
            .expand(&info, &mut ours)
            .unwrap();
        assert_eq!(ours, theirs, "size {size}");
    }
}

#[test]
fn hkdf_matches_ring() {
    let mut rng = Rng(0xfeed_beef_0000_0001);
    check_hkdf::<Sha256>(hkdf::HKDF_SHA256, &mut rng);
    check_hkdf::<Sha384>(hkdf::HKDF_SHA384, &mut rng);
    check_hkdf::<Sha512>(hkdf::HKDF_SHA512, &mut rng);
}
