//! SHA-1 (RFC 3174 / FIPS 180-1) — hand-rolled, zero dependencies.
//!
//! Shared by two unrelated call sites that each independently needed the
//! same algorithm as a compatibility requirement, not for security: git's
//! object model is content-addressed by SHA-1 (`rusty_git::sha1`, a
//! re-export of this crate), and the WebSocket handshake's
//! `Sec-WebSocket-Accept` derivation is defined by RFC 6455 §1.3 as
//! `base64(SHA-1(key + GUID))` (`rusty_term`'s `web-bridge` feature). Do
//! not reuse this for anything actually security-sensitive — SHA-1 is
//! broken for collision resistance.

/// Length of a SHA-1 digest in bytes.
pub const SHA1_DIGEST_LEN: usize = 20;

/// Incremental SHA-1 hasher.
#[derive(Clone)]
pub struct Sha1 {
    state: [u32; 5],
    len: u64,
    buffer: [u8; 64],
    buffer_len: usize,
}

impl Default for Sha1 {
    fn default() -> Self {
        Sha1::new()
    }
}

impl Sha1 {
    /// Creates a new hasher.
    pub fn new() -> Self {
        Sha1 {
            state: [0x67452301, 0xEFCDAB89, 0x98BADCFE, 0x10325476, 0xC3D2E1F0],
            len: 0,
            buffer: [0u8; 64],
            buffer_len: 0,
        }
    }

    /// Feeds more data into the hash.
    pub fn update(&mut self, mut data: &[u8]) {
        self.len = self.len.wrapping_add(data.len() as u64);
        if self.buffer_len > 0 {
            let need = 64 - self.buffer_len;
            let take = need.min(data.len());
            self.buffer[self.buffer_len..self.buffer_len + take].copy_from_slice(&data[..take]);
            self.buffer_len += take;
            data = &data[take..];
            if self.buffer_len == 64 {
                let block = self.buffer;
                self.process(&block);
                self.buffer_len = 0;
            }
        }
        while data.len() >= 64 {
            let mut block = [0u8; 64];
            block.copy_from_slice(&data[..64]);
            self.process(&block);
            data = &data[64..];
        }
        if !data.is_empty() {
            self.buffer[..data.len()].copy_from_slice(data);
            self.buffer_len = data.len();
        }
    }

    /// Finishes the hash and returns the 20-byte digest.
    pub fn finish(mut self) -> [u8; SHA1_DIGEST_LEN] {
        let bit_len = self.len.wrapping_mul(8);
        self.update(&[0x80]);
        while self.buffer_len != 56 {
            self.update(&[0x00]);
        }
        self.update(&bit_len.to_be_bytes());

        let mut out = [0u8; SHA1_DIGEST_LEN];
        for (i, word) in self.state.iter().enumerate() {
            out[i * 4..i * 4 + 4].copy_from_slice(&word.to_be_bytes());
        }
        out
    }

    fn process(&mut self, block: &[u8; 64]) {
        let mut w = [0u32; 80];
        for (i, chunk) in block.chunks_exact(4).enumerate() {
            w[i] = u32::from_be_bytes([chunk[0], chunk[1], chunk[2], chunk[3]]);
        }
        for i in 16..80 {
            w[i] = (w[i - 3] ^ w[i - 8] ^ w[i - 14] ^ w[i - 16]).rotate_left(1);
        }

        let [mut a, mut b, mut c, mut d, mut e] = self.state;
        for (i, &word) in w.iter().enumerate() {
            let (f, k) = match i {
                0..=19 => ((b & c) | (!b & d), 0x5A827999),
                20..=39 => (b ^ c ^ d, 0x6ED9EBA1),
                40..=59 => ((b & c) | (b & d) | (c & d), 0x8F1BBCDC),
                _ => (b ^ c ^ d, 0xCA62C1D6),
            };
            let tmp = a
                .rotate_left(5)
                .wrapping_add(f)
                .wrapping_add(e)
                .wrapping_add(k)
                .wrapping_add(word);
            e = d;
            d = c;
            c = b.rotate_left(30);
            b = a;
            a = tmp;
        }
        self.state[0] = self.state[0].wrapping_add(a);
        self.state[1] = self.state[1].wrapping_add(b);
        self.state[2] = self.state[2].wrapping_add(c);
        self.state[3] = self.state[3].wrapping_add(d);
        self.state[4] = self.state[4].wrapping_add(e);
    }
}

/// One-shot SHA-1 of `data`.
pub fn sha1(data: &[u8]) -> [u8; SHA1_DIGEST_LEN] {
    let mut h = Sha1::new();
    h.update(data);
    h.finish()
}

/// Formats a 20-byte digest as 40 lowercase hex characters — git's own
/// object-id string form.
pub fn hex(bytes: &[u8]) -> String {
    let mut s = String::with_capacity(bytes.len() * 2);
    for b in bytes {
        s.push_str(&format!("{b:02x}"));
    }
    s
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn rfc3174_vectors() {
        assert_eq!(hex(&sha1(b"")), "da39a3ee5e6b4b0d3255bfef95601890afd80709");
        assert_eq!(
            hex(&sha1(b"abc")),
            "a9993e364706816aba3e25717850c26c9cd0d89d"
        );
        assert_eq!(
            hex(&sha1(
                b"abcdbcdecdefdefgefghfghighijhijkijkljklmklmnlmnomnopnopq"
            )),
            "84983e441c3bd26ebaae4aa1f95129e5e54670f1"
        );
    }

    #[test]
    fn two_block_vector() {
        // Exactly 64 bytes: the length falls in a second, padding-only block.
        assert_eq!(
            hex(&sha1(&[b'a'; 64])),
            "0098ba824b5c16427bd7a1122a5a442a25ec644d"
        );
    }

    #[test]
    fn million_a_vector() {
        let data = vec![0x61u8; 1_000_000];
        assert_eq!(
            hex(&sha1(&data)),
            "34aa973cd4c4daa4f61eeb2bdbad27316534016f"
        );
    }

    #[test]
    fn incremental_matches_oneshot() {
        let mut h = Sha1::new();
        h.update(b"ab");
        h.update(b"c");
        assert_eq!(h.finish(), sha1(b"abc"));
    }

    #[test]
    fn matches_real_git_blob_hash_for_hello_world() {
        // `printf 'hello world' | git hash-object --stdin` == this value.
        let content = b"hello world";
        let header = format!("blob {}\0", content.len());
        let mut buf = header.into_bytes();
        buf.extend_from_slice(content);
        assert_eq!(hex(&sha1(&buf)), "95d09f2b10159347eece71399a7e2e907ea3df4f");
    }
}
