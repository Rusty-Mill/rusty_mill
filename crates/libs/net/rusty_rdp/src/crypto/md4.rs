//! MD4 message digest (RFC 1320), std-only.
//!
//! MD4 is thoroughly broken as a cryptographic hash. It is implemented here
//! for one reason only: NTLM (MS-NLMP) derives the NT password hash as
//! `MD4(UTF-16LE(password))`. It must not be used for anything else.

/// Length of an MD4 digest in bytes.
pub const MD4_DIGEST_LEN: usize = 16;

#[inline]
fn f(x: u32, y: u32, z: u32) -> u32 {
    (x & y) | (!x & z)
}

#[inline]
fn g(x: u32, y: u32, z: u32) -> u32 {
    (x & y) | (x & z) | (y & z)
}

#[inline]
fn h(x: u32, y: u32, z: u32) -> u32 {
    x ^ y ^ z
}

/// Compress one 64-byte block into the running state.
fn process_block(state: &mut [u32; 4], block: &[u8; 64]) {
    let mut x = [0u32; 16];
    for (i, word) in x.iter_mut().enumerate() {
        *word = u32::from_le_bytes([
            block[i * 4],
            block[i * 4 + 1],
            block[i * 4 + 2],
            block[i * 4 + 3],
        ]);
    }

    let mut s = *state;
    // Each operation updates one of the four words; the target index cycles
    // A, D, C, B (see RFC 1320), with (b, c, d) = (a+1, a+2, a+3) mod 4.
    let order = [0usize, 3, 2, 1];

    // Round 1: k = 0..15, shifts cycle [3, 7, 11, 19].
    let r1_shifts = [3u32, 7, 11, 19];
    for i in 0..16 {
        let a = order[i % 4];
        let (b, c, d) = ((a + 1) % 4, (a + 2) % 4, (a + 3) % 4);
        let sum = s[a].wrapping_add(f(s[b], s[c], s[d])).wrapping_add(x[i]);
        s[a] = sum.rotate_left(r1_shifts[i % 4]);
    }

    // Round 2: k = [0,4,8,12,1,5,9,13,2,6,10,14,3,7,11,15], shifts [3,5,9,13].
    let r2_k = [0usize, 4, 8, 12, 1, 5, 9, 13, 2, 6, 10, 14, 3, 7, 11, 15];
    let r2_shifts = [3u32, 5, 9, 13];
    for i in 0..16 {
        let a = order[i % 4];
        let (b, c, d) = ((a + 1) % 4, (a + 2) % 4, (a + 3) % 4);
        let sum = s[a]
            .wrapping_add(g(s[b], s[c], s[d]))
            .wrapping_add(x[r2_k[i]])
            .wrapping_add(0x5A82_7999);
        s[a] = sum.rotate_left(r2_shifts[i % 4]);
    }

    // Round 3: k = [0,8,4,12,2,10,6,14,1,9,5,13,3,11,7,15], shifts [3,9,11,15].
    let r3_k = [0usize, 8, 4, 12, 2, 10, 6, 14, 1, 9, 5, 13, 3, 11, 7, 15];
    let r3_shifts = [3u32, 9, 11, 15];
    for i in 0..16 {
        let a = order[i % 4];
        let (b, c, d) = ((a + 1) % 4, (a + 2) % 4, (a + 3) % 4);
        let sum = s[a]
            .wrapping_add(h(s[b], s[c], s[d]))
            .wrapping_add(x[r3_k[i]])
            .wrapping_add(0x6ED9_EBA1);
        s[a] = sum.rotate_left(r3_shifts[i % 4]);
    }

    for (dst, add) in state.iter_mut().zip(s.iter()) {
        *dst = dst.wrapping_add(*add);
    }
}

/// Compute the MD4 digest of `data`.
pub fn md4(data: &[u8]) -> [u8; MD4_DIGEST_LEN] {
    let mut state: [u32; 4] = [0x6745_2301, 0xefcd_ab89, 0x98ba_dcfe, 0x1032_5476];

    let mut block = [0u8; 64];
    let mut chunks = data.chunks_exact(64);
    for chunk in chunks.by_ref() {
        block.copy_from_slice(chunk);
        process_block(&mut state, &block);
    }
    let rest = chunks.remainder();

    // Final block(s): 0x80, zero padding, then the bit length as u64 LE.
    let mut tail = [0u8; 128];
    tail[..rest.len()].copy_from_slice(rest);
    tail[rest.len()] = 0x80;
    let bit_len = (data.len() as u64).wrapping_mul(8);
    let pad_to = if rest.len() < 56 { 64 } else { 128 };
    tail[pad_to - 8..pad_to].copy_from_slice(&bit_len.to_le_bytes());

    block.copy_from_slice(&tail[..64]);
    process_block(&mut state, &block);
    if pad_to == 128 {
        block.copy_from_slice(&tail[64..128]);
        process_block(&mut state, &block);
    }

    let mut out = [0u8; MD4_DIGEST_LEN];
    for (i, word) in state.iter().enumerate() {
        out[i * 4..i * 4 + 4].copy_from_slice(&word.to_le_bytes());
    }
    out
}

#[cfg(test)]
mod tests {
    use super::*;

    fn hex(bytes: &[u8]) -> String {
        bytes.iter().map(|b| format!("{b:02x}")).collect()
    }

    #[test]
    fn rfc1320_test_suite() {
        // The exact digests listed in RFC 1320, Appendix A.5.
        assert_eq!(hex(&md4(b"")), "31d6cfe0d16ae931b73c59d7e0c089c0");
        assert_eq!(hex(&md4(b"a")), "bde52cb31de33e46245e05fbdbd6fb24");
        assert_eq!(hex(&md4(b"abc")), "a448017aaf21d8525fc10ae87aa6729d");
        assert_eq!(
            hex(&md4(b"message digest")),
            "d9130a8164549fe818874806e1c7014b"
        );
        assert_eq!(
            hex(&md4(b"abcdefghijklmnopqrstuvwxyz")),
            "d79e1c308aa5bbcdeea8ed63df412da9"
        );
        assert_eq!(
            hex(&md4(
                b"ABCDEFGHIJKLMNOPQRSTUVWXYZabcdefghijklmnopqrstuvwxyz0123456789"
            )),
            "043f8582f241db351ce627e153e7f0e4"
        );
        assert_eq!(
            hex(&md4(
                b"12345678901234567890123456789012345678901234567890123456789012345678901234567890"
            )),
            "e33b4ddc9c38f2199c3e7b164fcc0536"
        );
    }

    #[test]
    fn ntlm_nt_hash_of_password() {
        // MS-NLMP 4.2.2.1.1: MD4(UTF-16LE("Password")) is the NTOWFv1 value.
        let unicode: Vec<u8> = "Password"
            .encode_utf16()
            .flat_map(|u| u.to_le_bytes())
            .collect();
        assert_eq!(hex(&md4(&unicode)), "a4f49c406510bdcab6824ee7c30fd852");
    }
}
