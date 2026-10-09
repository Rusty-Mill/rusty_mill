//! The ChaCha20-Poly1305 AEAD construction (RFC 8439 section 2.8).

use core::fmt;

use rusty_crypto_key::{constant_time_eq, wipe};

use crate::chacha20::ChaCha20;
use crate::poly1305::Poly1305;

/// Key length in bytes.
pub const KEY_LEN: usize = 32;
/// Nonce length in bytes.
pub const NONCE_LEN: usize = 12;
/// Authentication tag length in bytes.
pub const TAG_LEN: usize = 16;

/// Longest plaintext, RFC 8439 section 2.8: data uses block counters
/// 1 ..= 2^32 - 1 (block 0 makes the Poly1305 key), so 2^32 - 1 blocks of 64
/// bytes = 274,877,906,880 bytes, the same limit `ring` documents.
const MAX_LEN: u64 = u32::MAX as u64 * 64;

/// Sealing or opening failed. Carries no detail: a decryption failure must
/// not say whether the tag, the nonce or the data was wrong.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Error;

impl fmt::Display for Error {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str("AEAD operation failed")
    }
}

/// A ChaCha20-Poly1305 key. **Never reuse a nonce under one key**: it leaks
/// the XOR of the plaintexts and lets an attacker forge tags.
pub struct ChaCha20Poly1305 {
    key: [u8; KEY_LEN],
}

impl ChaCha20Poly1305 {
    /// A cipher under `key`. The key is wiped on drop.
    pub fn new(key: &[u8; KEY_LEN]) -> Self {
        Self { key: *key }
    }

    /// Encrypts `buf` in place and returns the tag over `aad` and the
    /// ciphertext. Fails only if `buf` is longer than the RFC permits.
    pub fn seal_in_place(
        &self,
        nonce: &[u8; NONCE_LEN],
        aad: &[u8],
        buf: &mut [u8],
    ) -> Result<[u8; TAG_LEN], Error> {
        if buf.len() as u64 > MAX_LEN {
            return Err(Error);
        }
        let cipher = ChaCha20::new(&self.key, nonce);
        cipher.apply(1, buf);
        Ok(tag(&cipher, aad, buf))
    }

    /// Verifies `tag` over `aad` and the ciphertext `buf`, and only then
    /// decrypts `buf` in place. On failure `buf` is left unchanged.
    pub fn open_in_place(
        &self,
        nonce: &[u8; NONCE_LEN],
        aad: &[u8],
        buf: &mut [u8],
        tag_bytes: &[u8],
    ) -> Result<(), Error> {
        if buf.len() as u64 > MAX_LEN {
            return Err(Error);
        }
        let cipher = ChaCha20::new(&self.key, nonce);
        let mut expected = tag(&cipher, aad, buf);
        let ok = constant_time_eq(&expected, tag_bytes);
        wipe(&mut expected);
        if !ok {
            return Err(Error);
        }
        cipher.apply(1, buf);
        Ok(())
    }
}

impl Drop for ChaCha20Poly1305 {
    fn drop(&mut self) {
        wipe(&mut self.key);
    }
}

impl fmt::Debug for ChaCha20Poly1305 {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str("ChaCha20Poly1305(<redacted>)")
    }
}

/// `Poly1305(aad || pad || ciphertext || pad || len(aad) || len(ciphertext))`
/// under the one-time key from block 0.
fn tag(cipher: &ChaCha20, aad: &[u8], ciphertext: &[u8]) -> [u8; TAG_LEN] {
    let mut block0 = cipher.block(0);
    let mut one_time_key = [0u8; 32];
    one_time_key.copy_from_slice(&block0[..32]);
    wipe(&mut block0);

    let mut mac = Poly1305::new(&one_time_key);
    wipe(&mut one_time_key);
    let pad = [0u8; 16];
    mac.update(aad);
    mac.update(&pad[..aad.len().wrapping_neg() & 15]);
    mac.update(ciphertext);
    mac.update(&pad[..ciphertext.len().wrapping_neg() & 15]);
    mac.update(&(aad.len() as u64).to_le_bytes());
    mac.update(&(ciphertext.len() as u64).to_le_bytes());
    mac.finalize()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn max_len_uses_every_legal_block_counter_and_no_more() {
        // Data blocks are counters 1 ..= n with n = ceil(MAX_LEN / 64).
        let last_counter = MAX_LEN.div_ceil(64);
        assert_eq!(
            last_counter,
            u32::MAX as u64,
            "the final legal counter is used"
        );
        assert_eq!(MAX_LEN, 274_877_906_880, "RFC 8439 section 2.8");
        // One byte more would need counter 2^32, which wraps.
        assert_eq!((MAX_LEN + 1).div_ceil(64), u32::MAX as u64 + 1);
    }
}
