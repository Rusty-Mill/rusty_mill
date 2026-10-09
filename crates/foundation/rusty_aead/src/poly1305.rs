//! Poly1305 (RFC 8439 section 2.5) with 5x26-bit limbs.

use rusty_crypto_key::wipe;

const MASK: u32 = 0x03ff_ffff;

/// A Poly1305 one-time authenticator. **The key must never be reused** for a
/// second message; the AEAD derives a fresh one per nonce.
pub struct Poly1305 {
    r: [u32; 5],
    h: [u32; 5],
    pad: [u32; 4],
    buffer: [u8; 16],
    len: usize,
}

fn le32(bytes: &[u8]) -> u32 {
    u32::from_le_bytes([bytes[0], bytes[1], bytes[2], bytes[3]])
}

impl Poly1305 {
    /// Starts a MAC under the 32-byte one-time `key` (`r || s`).
    pub fn new(key: &[u8; 32]) -> Self {
        // r is clamped as RFC 8439 section 2.5.1 requires.
        let r = [
            le32(&key[0..]) & 0x03ff_ffff,
            (le32(&key[3..]) >> 2) & 0x03ff_ff03,
            (le32(&key[6..]) >> 4) & 0x03ff_c0ff,
            (le32(&key[9..]) >> 6) & 0x03f0_3fff,
            (le32(&key[12..]) >> 8) & 0x000f_ffff,
        ];
        let pad = [
            le32(&key[16..]),
            le32(&key[20..]),
            le32(&key[24..]),
            le32(&key[28..]),
        ];
        Self {
            r,
            h: [0; 5],
            pad,
            buffer: [0; 16],
            len: 0,
        }
    }

    /// Absorbs `data`.
    pub fn update(&mut self, mut data: &[u8]) {
        if self.len > 0 {
            let take = (16 - self.len).min(data.len());
            self.buffer[self.len..self.len + take].copy_from_slice(&data[..take]);
            self.len += take;
            data = &data[take..];
            if self.len < 16 {
                return;
            }
            let block = self.buffer;
            self.block(&block, 1 << 24);
            self.len = 0;
        }
        let (blocks, rest) = data.as_chunks::<16>();
        for block in blocks {
            self.block(block, 1 << 24);
        }
        self.buffer[..rest.len()].copy_from_slice(rest);
        self.len = rest.len();
    }

    /// Finishes and returns the 16-byte tag.
    pub fn finalize(mut self) -> [u8; 16] {
        if self.len > 0 {
            let mut last = [0u8; 16];
            last[..self.len].copy_from_slice(&self.buffer[..self.len]);
            last[self.len] = 1;
            self.block(&last, 0);
        }
        let tag = self.tag();
        wipe(&mut self.h);
        tag
    }

    /// Processes one 16-byte block; `hibit` is `1 << 24` for full blocks.
    fn block(&mut self, m: &[u8], hibit: u32) {
        let [r0, r1, r2, r3, r4] = self.r.map(u64::from);
        let [s1, s2, s3, s4] = [r1 * 5, r2 * 5, r3 * 5, r4 * 5];
        let h = &mut self.h;
        let h0 = u64::from(h[0] + (le32(&m[0..]) & MASK));
        let h1 = u64::from(h[1] + ((le32(&m[3..]) >> 2) & MASK));
        let h2 = u64::from(h[2] + ((le32(&m[6..]) >> 4) & MASK));
        let h3 = u64::from(h[3] + ((le32(&m[9..]) >> 6) & MASK));
        let h4 = u64::from(h[4] + ((le32(&m[12..]) >> 8) | hibit));

        let d0 = h0 * r0 + h1 * s4 + h2 * s3 + h3 * s2 + h4 * s1;
        let d1 = h0 * r1 + h1 * r0 + h2 * s4 + h3 * s3 + h4 * s2;
        let d2 = h0 * r2 + h1 * r1 + h2 * r0 + h3 * s4 + h4 * s3;
        let d3 = h0 * r3 + h1 * r2 + h2 * r1 + h3 * r0 + h4 * s4;
        let d4 = h0 * r4 + h1 * r3 + h2 * r2 + h3 * r1 + h4 * r0;

        let m26 = u64::from(MASK);
        let mut c = d0 >> 26;
        let n0 = d0 & m26;
        let d1 = d1 + c;
        c = d1 >> 26;
        let n1 = d1 & m26;
        let d2 = d2 + c;
        c = d2 >> 26;
        let n2 = d2 & m26;
        let d3 = d3 + c;
        c = d3 >> 26;
        let n3 = d3 & m26;
        let d4 = d4 + c;
        c = d4 >> 26;
        let n4 = d4 & m26;
        let n0 = n0 + c * 5;
        c = n0 >> 26;
        let n0 = n0 & m26;
        let n1 = n1 + c;
        *h = [n0 as u32, n1 as u32, n2 as u32, n3 as u32, n4 as u32];
    }

    /// Fully reduces `h` modulo 2^130 - 5 without branching, then adds `s`.
    fn tag(&self) -> [u8; 16] {
        let [mut h0, mut h1, mut h2, mut h3, mut h4] = self.h;
        let mut c = h1 >> 26;
        h1 &= MASK;
        h2 += c;
        c = h2 >> 26;
        h2 &= MASK;
        h3 += c;
        c = h3 >> 26;
        h3 &= MASK;
        h4 += c;
        c = h4 >> 26;
        h4 &= MASK;
        h0 += c * 5;
        c = h0 >> 26;
        h0 &= MASK;
        h1 += c;

        // g = h + 5 - 2^130; if g did not underflow, h >= p and g is the answer.
        let mut g0 = h0 + 5;
        c = g0 >> 26;
        g0 &= MASK;
        let mut g1 = h1 + c;
        c = g1 >> 26;
        g1 &= MASK;
        let mut g2 = h2 + c;
        c = g2 >> 26;
        g2 &= MASK;
        let mut g3 = h3 + c;
        c = g3 >> 26;
        g3 &= MASK;
        let g4 = (h4 + c).wrapping_sub(1 << 26);

        let select_g = (g4 >> 31).wrapping_sub(1); // all ones iff g4 did not underflow
        let keep_h = !select_g;
        h0 = (h0 & keep_h) | (g0 & select_g);
        h1 = (h1 & keep_h) | (g1 & select_g);
        h2 = (h2 & keep_h) | (g2 & select_g);
        h3 = (h3 & keep_h) | (g3 & select_g);
        h4 = (h4 & keep_h) | (g4 & select_g);

        // Repack into four 32-bit words and add the pad modulo 2^128.
        let w0 = h0 | (h1 << 26);
        let w1 = (h1 >> 6) | (h2 << 20);
        let w2 = (h2 >> 12) | (h3 << 14);
        let w3 = (h3 >> 18) | (h4 << 8);
        let mut f = u64::from(w0) + u64::from(self.pad[0]);
        let t0 = f as u32;
        f = u64::from(w1) + u64::from(self.pad[1]) + (f >> 32);
        let t1 = f as u32;
        f = u64::from(w2) + u64::from(self.pad[2]) + (f >> 32);
        let t2 = f as u32;
        f = u64::from(w3) + u64::from(self.pad[3]) + (f >> 32);
        let t3 = f as u32;

        let mut tag = [0u8; 16];
        for (chunk, word) in tag.as_chunks_mut::<4>().0.iter_mut().zip([t0, t1, t2, t3]) {
            *chunk = word.to_le_bytes();
        }
        tag
    }
}

impl Drop for Poly1305 {
    fn drop(&mut self) {
        wipe(&mut self.r);
        wipe(&mut self.h);
        wipe(&mut self.pad);
        wipe(&mut self.buffer);
    }
}
