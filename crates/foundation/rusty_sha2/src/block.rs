//! Block buffering and Merkle-Damgard padding shared by the SHA-2 family.

use rusty_crypto_key::wipe;

/// Accumulates input into `N`-byte blocks.
#[derive(Clone)]
pub(crate) struct Buffer<const N: usize> {
    buf: [u8; N],
    len: usize,
    total: u128,
}

impl<const N: usize> Buffer<N> {
    pub(crate) const fn new() -> Self {
        Self {
            buf: [0; N],
            len: 0,
            total: 0,
        }
    }

    /// Feeds `data`, calling `compress` once per completed block.
    pub(crate) fn absorb(&mut self, mut data: &[u8], mut compress: impl FnMut(&[u8; N])) {
        self.total = self.total.wrapping_add(data.len() as u128);
        if self.len > 0 {
            let take = (N - self.len).min(data.len());
            self.buf[self.len..self.len + take].copy_from_slice(&data[..take]);
            self.len += take;
            data = &data[take..];
            if self.len < N {
                return;
            }
            compress(&self.buf);
            self.len = 0;
        }
        let (blocks, rest) = data.as_chunks::<N>();
        for block in blocks {
            compress(block);
        }
        self.buf[..rest.len()].copy_from_slice(rest);
        self.len = rest.len();
    }

    /// Appends the `0x80` marker, zero fill and the big-endian bit length in
    /// the last `length_bytes` bytes, compressing one or two final blocks.
    pub(crate) fn finish(&mut self, length_bytes: usize, mut compress: impl FnMut(&[u8; N])) {
        let bits = self.total.wrapping_mul(8).to_be_bytes();
        let mut tail = [[0u8; N]; 2];
        let used = self.len;
        tail[0][..used].copy_from_slice(&self.buf[..used]);
        tail[0][used] = 0x80;
        let blocks = if used + 1 + length_bytes <= N { 1 } else { 2 };
        let end = &mut tail[blocks - 1];
        end[N - length_bytes..].copy_from_slice(&bits[16 - length_bytes..]);
        for block in &tail[..blocks] {
            compress(block);
        }
        wipe(&mut tail[0]);
        wipe(&mut tail[1]);
    }

    pub(crate) fn wipe(&mut self) {
        wipe(&mut self.buf);
        self.len = 0;
        self.total = 0;
    }
}
