//! AES-GCM (NIST SP 800-38D) with a 96-bit IV and a 16-byte tag, on the bitsliced AES core.

use core::fmt;

use rusty_crypto_key::{constant_time_eq, wipe};

use crate::aead::Error;
use crate::aes::{Aes, LANES};
use crate::ghash::ghash;

/// Nonce (IV) length in bytes. Other IV lengths are not supported.
pub const GCM_NONCE_LEN: usize = 12;
/// Authentication tag length in bytes. Truncated tags are not supported.
pub const GCM_TAG_LEN: usize = 16;

/// Longest plaintext, SP 800-38D: 2^39 - 256 bits = 2^36 - 32 bytes, i.e. counters
/// 2 ..= 2^32 - 1 for data (counter 1 masks the tag).
const MAX_LEN: u64 = (1 << 36) - 32;
/// Longest additional authenticated data, SP 800-38D: 2^64 - 1 bits = 2^61 - 1 bytes. GHASH
/// writes the AAD bit length into a 64-bit field, so a longer AAD would corrupt it.
const MAX_AAD: u64 = (1 << 61) - 1;

/// Whether the AAD and data lengths are within the specification's limits. Arithmetic only,
/// so the boundaries can be tested without allocating anything near them.
fn within_limits(aad_len: u64, data_len: u64) -> bool {
    aad_len <= MAX_AAD && data_len <= MAX_LEN
}

struct Gcm {
    aes: Aes,
    /// GHASH key, E_K(0^128) read big-endian.
    h: u128,
}

impl Gcm {
    fn new(aes: Aes) -> Self {
        let mut zero = [[0u8; 16]; LANES];
        aes.encrypt_blocks(&mut zero);
        let h = u128::from_be_bytes(zero[0]);
        // Every lane holds E_K(0^128), i.e. a copy of the hash key.
        wipe(&mut zero);
        Self { aes, h }
    }

    /// E_K of the counter blocks `iv || ctr` for `ctr = start .. start + LANES` (wrapping).
    fn keystream(&self, iv: &[u8; GCM_NONCE_LEN], start: u32) -> [[u8; 16]; LANES] {
        let mut blocks = [[0u8; 16]; LANES];
        for (l, block) in blocks.iter_mut().enumerate() {
            block[..12].copy_from_slice(iv);
            block[12..].copy_from_slice(&start.wrapping_add(l as u32).to_be_bytes());
        }
        self.aes.encrypt_blocks(&mut blocks);
        blocks
    }

    /// Applies CTR keystream to `buf` and returns E_K(J0) (the tag mask). Counter 1 is the
    /// tag mask; data block `i` uses counter `i + 2`, so the first batch (counters 1 to `LANES`)
    /// carries the mask and the first `LANES - 1` data blocks.
    fn ctr(&self, iv: &[u8; GCM_NONCE_LEN], buf: &mut [u8]) -> [u8; 16] {
        let mut mask = [0u8; 16];
        let mut next = 1u32;
        let mut first = true;
        let mut offset = 0usize;
        loop {
            let mut ks = self.keystream(iv, next);
            let blocks = if first {
                mask = ks[0];
                &ks[1..]
            } else {
                &ks[..]
            };
            for block in blocks {
                if offset >= buf.len() {
                    break;
                }
                let end = (offset + 16).min(buf.len());
                for (b, k) in buf[offset..end].iter_mut().zip(block.iter()) {
                    *b ^= k;
                }
                offset = end;
            }
            for block in ks.iter_mut() {
                wipe(block);
            }
            if offset >= buf.len() {
                return mask;
            }
            first = false;
            next = next.wrapping_add(LANES as u32);
        }
    }

    fn tag(&self, mask: &[u8; 16], aad: &[u8], ct: &[u8]) -> [u8; GCM_TAG_LEN] {
        let s = ghash(self.h, aad, ct).to_be_bytes();
        let mut t = [0u8; 16];
        for i in 0..16 {
            t[i] = s[i] ^ mask[i];
        }
        t
    }

    fn seal(
        &self,
        iv: &[u8; GCM_NONCE_LEN],
        aad: &[u8],
        buf: &mut [u8],
    ) -> Result<[u8; GCM_TAG_LEN], Error> {
        if !within_limits(aad.len() as u64, buf.len() as u64) {
            return Err(Error);
        }
        let mut mask = self.ctr(iv, buf);
        let tag = self.tag(&mask, aad, buf);
        wipe(&mut mask);
        Ok(tag)
    }

    fn open(
        &self,
        iv: &[u8; GCM_NONCE_LEN],
        aad: &[u8],
        buf: &mut [u8],
        tag: &[u8],
    ) -> Result<(), Error> {
        if !within_limits(aad.len() as u64, buf.len() as u64) {
            return Err(Error);
        }
        // The mask needs counter 1 only, so compute it from a one-block pass over an empty
        // buffer; the data keystream is applied only after the tag verifies, leaving `buf`
        // unchanged on failure.
        let mut mask = self.ctr(iv, &mut []);
        let mut expected = self.tag(&mask, aad, buf);
        let ok = constant_time_eq(&expected, tag);
        wipe(&mut expected);
        wipe(&mut mask);
        if !ok {
            return Err(Error);
        }
        self.ctr(iv, buf);
        Ok(())
    }
}

macro_rules! gcm_type {
    ($name:ident, $key_len:expr, $ctor:ident, $label:expr) => {
        /// An AES-GCM key. **Never reuse a nonce under one key**: it leaks the XOR of the
        /// plaintexts and the GHASH key, after which tags can be forged.
        pub struct $name(Gcm);

        impl $name {
            /// Key length in bytes.
            pub const KEY_LEN: usize = $key_len;

            /// A cipher under `key`. Round keys are wiped on drop.
            pub fn new(key: &[u8; $key_len]) -> Self {
                Self(Gcm::new(Aes::$ctor(key)))
            }

            /// Encrypts `buf` in place and returns the tag over `aad` and the ciphertext.
            /// Fails only if `buf` is longer than 2^36 - 32 bytes or `aad` is longer than
            /// 2^61 - 1 bytes, the limits SP 800-38D sets.
            pub fn seal_in_place(
                &self,
                nonce: &[u8; GCM_NONCE_LEN],
                aad: &[u8],
                buf: &mut [u8],
            ) -> Result<[u8; GCM_TAG_LEN], Error> {
                self.0.seal(nonce, aad, buf)
            }

            /// Verifies `tag` over `aad` and the ciphertext `buf`, and only then decrypts
            /// `buf` in place. Fails if the tag does not verify or if `buf` or `aad` exceeds the
            /// limits above. On failure `buf` is left unchanged.
            pub fn open_in_place(
                &self,
                nonce: &[u8; GCM_NONCE_LEN],
                aad: &[u8],
                buf: &mut [u8],
                tag: &[u8],
            ) -> Result<(), Error> {
                self.0.open(nonce, aad, buf, tag)
            }
        }

        impl fmt::Debug for $name {
            fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
                f.write_str(concat!($label, "(<redacted>)"))
            }
        }
    };
}

gcm_type!(Aes128Gcm, 16, new_128, "Aes128Gcm");
gcm_type!(Aes256Gcm, 32, new_256, "Aes256Gcm");

impl Drop for Gcm {
    fn drop(&mut self) {
        wipe(core::slice::from_mut(&mut self.h));
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn length_limits_are_exact_at_their_boundaries() {
        assert!(within_limits(0, 0));
        assert!(within_limits(MAX_AAD, MAX_LEN));
        assert!(!within_limits(MAX_AAD + 1, 0));
        assert!(!within_limits(0, MAX_LEN + 1));
        assert!(!within_limits(u64::MAX, 0));
        assert!(!within_limits(0, u64::MAX));
        // 2^61 - 1 bytes is the largest AAD whose bit length fits in 64 bits.
        assert_eq!(u128::from(MAX_AAD) * 8, (1u128 << 64) - 8);
        assert!((u128::from(MAX_AAD) + 1) * 8 > u128::from(u64::MAX));
        // 2^36 - 32 bytes is 2^39 - 256 bits, the specification's plaintext limit.
        assert_eq!(u128::from(MAX_LEN) * 8, (1u128 << 39) - 256);
    }
}
