//! HMAC (RFC 2104 / FIPS 198-1) over any [`Hash`].

use crate::{Hash, Sha256, Sha384, Sha512};
use rusty_crypto_key::{constant_time_eq, wipe};

/// Largest block length of a supported hash.
const MAX_BLOCK: usize = 128;

/// Streaming HMAC. The key block and hash state are wiped on drop.
#[derive(Clone)]
pub struct Hmac<H: Hash> {
    inner: H,
    opad: [u8; MAX_BLOCK],
}

/// HMAC-SHA-256.
pub type HmacSha256 = Hmac<Sha256>;
/// HMAC-SHA-384.
pub type HmacSha384 = Hmac<Sha384>;
/// HMAC-SHA-512.
pub type HmacSha512 = Hmac<Sha512>;

impl<H: Hash> Hmac<H> {
    /// An HMAC keyed with `key` (any length; long keys are hashed first).
    pub fn new(key: &[u8]) -> Self {
        let mut block = [0u8; MAX_BLOCK];
        if key.len() > H::BLOCK_LEN {
            let mut hashed = H::digest(key);
            let digest = hashed.as_ref();
            block[..digest.len()].copy_from_slice(digest);
            wipe(hashed.as_mut());
        } else {
            block[..key.len()].copy_from_slice(key);
        }
        let mut ipad = [0u8; MAX_BLOCK];
        let mut opad = [0u8; MAX_BLOCK];
        for i in 0..H::BLOCK_LEN {
            ipad[i] = block[i] ^ 0x36;
            opad[i] = block[i] ^ 0x5c;
        }
        let mut inner = H::new();
        inner.update(&ipad[..H::BLOCK_LEN]);
        wipe(&mut block);
        wipe(&mut ipad);
        Self { inner, opad }
    }

    /// Absorbs `data`.
    pub fn update(&mut self, data: &[u8]) {
        self.inner.update(data);
    }

    /// Finishes and returns the tag.
    pub fn finalize(self) -> H::Output {
        let mut inner_digest = self.inner.clone().finalize();
        let mut outer = H::new();
        outer.update(&self.opad[..H::BLOCK_LEN]);
        outer.update(inner_digest.as_ref());
        wipe(inner_digest.as_mut());
        outer.finalize()
    }

    /// One-shot tag of `data` under `key`.
    pub fn mac(key: &[u8], data: &[u8]) -> H::Output {
        let mut hmac = Self::new(key);
        hmac.update(data);
        hmac.finalize()
    }

    /// Checks `tag` against the tag of `data` under `key` in constant time.
    /// A tag of the wrong length is rejected (no truncated-tag acceptance).
    pub fn verify(key: &[u8], data: &[u8], tag: &[u8]) -> bool {
        let mut expected = Self::mac(key, data);
        let ok = constant_time_eq(expected.as_ref(), tag);
        wipe(expected.as_mut());
        ok
    }
}

impl<H: Hash> Drop for Hmac<H> {
    fn drop(&mut self) {
        wipe(&mut self.opad);
    }
}
