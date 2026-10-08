//! The ChaCha20 block function and stream cipher (RFC 8439 section 2.3-2.4).

use rusty_crypto_key::wipe;

const CONSTANTS: [u32; 4] = [0x6170_7865, 0x3320_646e, 0x7962_2d32, 0x6b20_6574];

/// Key and nonce words; the block counter is passed per call.
pub(crate) struct ChaCha20 {
    key: [u32; 8],
    nonce: [u32; 3],
}

impl ChaCha20 {
    pub(crate) fn new(key: &[u8; 32], nonce: &[u8; 12]) -> Self {
        let mut k = [0u32; 8];
        for (word, bytes) in k.iter_mut().zip(key.chunks_exact(4)) {
            *word = u32::from_le_bytes([bytes[0], bytes[1], bytes[2], bytes[3]]);
        }
        let mut n = [0u32; 3];
        for (word, bytes) in n.iter_mut().zip(nonce.chunks_exact(4)) {
            *word = u32::from_le_bytes([bytes[0], bytes[1], bytes[2], bytes[3]]);
        }
        Self { key: k, nonce: n }
    }

    /// One 64-byte keystream block.
    pub(crate) fn block(&self, counter: u32) -> [u8; 64] {
        let mut state = [0u32; 16];
        state[..4].copy_from_slice(&CONSTANTS);
        state[4..12].copy_from_slice(&self.key);
        state[12] = counter;
        state[13..].copy_from_slice(&self.nonce);
        let mut x = state;
        for _ in 0..10 {
            quarter(&mut x, 0, 4, 8, 12);
            quarter(&mut x, 1, 5, 9, 13);
            quarter(&mut x, 2, 6, 10, 14);
            quarter(&mut x, 3, 7, 11, 15);
            quarter(&mut x, 0, 5, 10, 15);
            quarter(&mut x, 1, 6, 11, 12);
            quarter(&mut x, 2, 7, 8, 13);
            quarter(&mut x, 3, 4, 9, 14);
        }
        let mut out = [0u8; 64];
        for ((chunk, mixed), original) in out.chunks_exact_mut(4).zip(x).zip(state) {
            chunk.copy_from_slice(&mixed.wrapping_add(original).to_le_bytes());
        }
        wipe(&mut x);
        wipe(&mut state);
        out
    }

    /// XORs the keystream starting at block `counter` into `data`. The caller
    /// guarantees the counter does not wrap (see `aead::MAX_LEN`).
    pub(crate) fn apply(&self, mut counter: u32, data: &mut [u8]) {
        for chunk in data.chunks_mut(64) {
            let mut stream = self.block(counter);
            for (byte, key) in chunk.iter_mut().zip(stream.iter()) {
                *byte ^= key;
            }
            wipe(&mut stream);
            counter = counter.wrapping_add(1);
        }
    }
}

impl Drop for ChaCha20 {
    fn drop(&mut self) {
        wipe(&mut self.key);
        wipe(&mut self.nonce);
    }
}

#[inline(always)]
fn quarter(x: &mut [u32; 16], a: usize, b: usize, c: usize, d: usize) {
    x[a] = x[a].wrapping_add(x[b]);
    x[d] = (x[d] ^ x[a]).rotate_left(16);
    x[c] = x[c].wrapping_add(x[d]);
    x[b] = (x[b] ^ x[c]).rotate_left(12);
    x[a] = x[a].wrapping_add(x[b]);
    x[d] = (x[d] ^ x[a]).rotate_left(8);
    x[c] = x[c].wrapping_add(x[d]);
    x[b] = (x[b] ^ x[c]).rotate_left(7);
}
