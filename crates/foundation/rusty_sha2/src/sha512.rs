//! SHA-512 and SHA-384 (FIPS 180-4); SHA-384 is SHA-512 with another IV,
//! truncated to 48 bytes.

use crate::block::Buffer;
use crate::constants::{SHA384_IV, SHA512_IV, SHA512_K};
use crate::Hash;
use rusty_crypto_key::wipe;

#[derive(Clone)]
struct Core {
    state: [u64; 8],
    buffer: Buffer<128>,
}

impl Core {
    fn new(iv: [u64; 8]) -> Self {
        Self {
            state: iv,
            buffer: Buffer::new(),
        }
    }

    fn update(&mut self, data: &[u8]) {
        let state = &mut self.state;
        self.buffer.absorb(data, |block| compress(state, block));
    }

    fn finish(&mut self) -> [u8; 64] {
        let state = &mut self.state;
        self.buffer.finish(16, |block| compress(state, block));
        let mut out = [0u8; 64];
        for (chunk, word) in out.as_chunks_mut::<8>().0.iter_mut().zip(self.state) {
            *chunk = word.to_be_bytes();
        }
        out
    }
}

impl Drop for Core {
    fn drop(&mut self) {
        wipe(&mut self.state);
        self.buffer.wipe();
    }
}

/// Streaming SHA-512.
#[derive(Clone)]
pub struct Sha512(Core);

impl Hash for Sha512 {
    const BLOCK_LEN: usize = 128;
    const OUTPUT_LEN: usize = 64;
    type Output = [u8; 64];

    fn new() -> Self {
        Self(Core::new(SHA512_IV))
    }

    fn update(&mut self, data: &[u8]) {
        self.0.update(data);
    }

    fn finalize(mut self) -> [u8; 64] {
        self.0.finish()
    }
}

impl Default for Sha512 {
    fn default() -> Self {
        Self::new()
    }
}

/// Streaming SHA-384.
#[derive(Clone)]
pub struct Sha384(Core);

impl Hash for Sha384 {
    const BLOCK_LEN: usize = 128;
    const OUTPUT_LEN: usize = 48;
    type Output = [u8; 48];

    fn new() -> Self {
        Self(Core::new(SHA384_IV))
    }

    fn update(&mut self, data: &[u8]) {
        self.0.update(data);
    }

    fn finalize(mut self) -> [u8; 48] {
        let mut full = self.0.finish();
        let mut out = [0u8; 48];
        out.copy_from_slice(&full[..48]);
        wipe(&mut full);
        out
    }
}

impl Default for Sha384 {
    fn default() -> Self {
        Self::new()
    }
}

fn compress(state: &mut [u64; 8], block: &[u8; 128]) {
    let mut w = [0u64; 80];
    for (word, bytes) in w.iter_mut().zip(block.as_chunks::<8>().0) {
        *word = u64::from_be_bytes(*bytes);
    }
    for i in 16..80 {
        let s0 = w[i - 15].rotate_right(1) ^ w[i - 15].rotate_right(8) ^ (w[i - 15] >> 7);
        let s1 = w[i - 2].rotate_right(19) ^ w[i - 2].rotate_right(61) ^ (w[i - 2] >> 6);
        w[i] = w[i - 16]
            .wrapping_add(s0)
            .wrapping_add(w[i - 7])
            .wrapping_add(s1);
    }
    let [mut a, mut b, mut c, mut d, mut e, mut f, mut g, mut h] = *state;
    for i in 0..80 {
        let s1 = e.rotate_right(14) ^ e.rotate_right(18) ^ e.rotate_right(41);
        let ch = (e & f) ^ (!e & g);
        let t1 = h
            .wrapping_add(s1)
            .wrapping_add(ch)
            .wrapping_add(SHA512_K[i])
            .wrapping_add(w[i]);
        let s0 = a.rotate_right(28) ^ a.rotate_right(34) ^ a.rotate_right(39);
        let maj = (a & b) ^ (a & c) ^ (b & c);
        let t2 = s0.wrapping_add(maj);
        h = g;
        g = f;
        f = e;
        e = d.wrapping_add(t1);
        d = c;
        c = b;
        b = a;
        a = t1.wrapping_add(t2);
    }
    for (s, v) in state.iter_mut().zip([a, b, c, d, e, f, g, h]) {
        *s = s.wrapping_add(v);
    }
    wipe(&mut w);
}
