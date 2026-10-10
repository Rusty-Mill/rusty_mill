//! AES-128 and AES-256 encryption (FIPS 197), bitsliced.
//!
//! No table is indexed by key or data and nothing branches on either: the S-box is the
//! Boyar-Peralta boolean circuit (113 gates) applied to bit planes, and the state holds 64
//! blocks at once, one bit plane per `u64` (bit `l` of plane `p` is bit `p % 8` of byte
//! `p / 8` of block `l`). Encryption only; GCM never needs the inverse cipher.
//!
//! This is the design, not a proof. Whether the compiled code keeps the property is checked
//! by the taint, jump-count and timing evidence described in `docs/research/AES-GCM-DESIGN.md`.

use rusty_crypto_key::wipe;

/// Blocks processed per batch.
pub(crate) const LANES: usize = 64;

/// One batch of bit planes.
type Planes = [u64; 128];

const MAX_ROUNDS: usize = 14;

/// Expanded AES key. Wiped on drop.
pub(crate) struct Aes {
    round_keys: [[u8; 16]; MAX_ROUNDS + 1],
    rounds: usize,
}

impl Aes {
    pub(crate) fn new_128(key: &[u8; 16]) -> Self {
        Self::expand(key, 4, 10)
    }

    pub(crate) fn new_256(key: &[u8; 32]) -> Self {
        Self::expand(key, 8, 14)
    }

    /// FIPS 197 section 5.2. `nk` and `rounds` are public constants, so the branches on the
    /// word index below depend on nothing secret.
    fn expand(key: &[u8], nk: usize, rounds: usize) -> Self {
        let total = 4 * (rounds + 1);
        let mut w = [[0u8; 4]; 4 * (MAX_ROUNDS + 1)];
        for (i, word) in w.iter_mut().take(nk).enumerate() {
            word.copy_from_slice(&key[4 * i..4 * i + 4]);
        }
        let mut rcon = 1u8;
        for i in nk..total {
            let mut t = w[i - 1];
            if i % nk == 0 {
                t = sub_word([t[1], t[2], t[3], t[0]]);
                t[0] ^= rcon;
                rcon = xtime_byte(rcon);
            } else if nk > 6 && i % nk == 4 {
                t = sub_word(t);
            }
            for k in 0..4 {
                w[i][k] = w[i - nk][k] ^ t[k];
            }
        }
        let mut round_keys = [[0u8; 16]; MAX_ROUNDS + 1];
        for (r, rk) in round_keys.iter_mut().enumerate().take(rounds + 1) {
            for c in 0..4 {
                rk[4 * c..4 * c + 4].copy_from_slice(&w[4 * r + c]);
            }
        }
        wipe_words(&mut w);
        Self { round_keys, rounds }
    }

    /// Encrypts up to 64 blocks in place; unused lanes are ignored (and cost the same).
    pub(crate) fn encrypt_blocks(&self, blocks: &mut [[u8; 16]; LANES]) {
        let mut st = to_planes(blocks);
        self.add_round_key(&mut st, 0);
        for r in 1..self.rounds {
            sub_bytes(&mut st);
            shift_rows(&mut st);
            mix_columns(&mut st);
            self.add_round_key(&mut st, r);
        }
        sub_bytes(&mut st);
        shift_rows(&mut st);
        self.add_round_key(&mut st, self.rounds);
        from_planes(&mut st, blocks);
        wipe(&mut st);
    }

    fn add_round_key(&self, st: &mut Planes, round: usize) {
        let rk = &self.round_keys[round];
        for (b, byte) in rk.iter().enumerate() {
            for k in 0..8 {
                // All ones when the key bit is set, all zeros otherwise: no branch.
                let mask = 0u64.wrapping_sub(u64::from((byte >> k) & 1));
                st[b * 8 + k] ^= mask;
            }
        }
    }
}

impl Drop for Aes {
    fn drop(&mut self) {
        for rk in self.round_keys.iter_mut() {
            wipe(rk);
        }
    }
}

fn wipe_words(w: &mut [[u8; 4]]) {
    for word in w.iter_mut() {
        wipe(word);
    }
}

fn xtime_byte(b: u8) -> u8 {
    (b << 1) ^ (0x1b & 0u8.wrapping_sub(b >> 7))
}

/// SubWord for the key schedule: the same circuit, with the four bytes in lanes 0 to 3.
fn sub_word(word: [u8; 4]) -> [u8; 4] {
    let mut q = [0u64; 8];
    for (l, byte) in word.iter().enumerate() {
        for (k, plane) in q.iter_mut().enumerate() {
            *plane |= u64::from((byte >> k) & 1) << l;
        }
    }
    sbox(&mut q);
    let mut out = [0u8; 4];
    for (l, byte) in out.iter_mut().enumerate() {
        for (k, plane) in q.iter().enumerate() {
            *byte |= (((plane >> l) & 1) as u8) << k;
        }
    }
    wipe(&mut q);
    out
}

fn sub_bytes(st: &mut Planes) {
    for b in 0..16 {
        let mut q = [0u64; 8];
        q.copy_from_slice(&st[b * 8..b * 8 + 8]);
        sbox(&mut q);
        st[b * 8..b * 8 + 8].copy_from_slice(&q);
    }
}

/// Output byte `4c + r` takes input byte `4((c + r) mod 4) + r` (state is column-major).
fn shift_rows(st: &mut Planes) {
    let old = *st;
    for c in 0..4 {
        for r in 0..4 {
            let to = (4 * c + r) * 8;
            let from = (4 * ((c + r) % 4) + r) * 8;
            st[to..to + 8].copy_from_slice(&old[from..from + 8]);
        }
    }
}

/// Multiplication by x in GF(2^8) on one byte's eight planes (reduction polynomial 0x11b).
fn xtime(v: &[u64; 8]) -> [u64; 8] {
    [
        v[7],
        v[0] ^ v[7],
        v[1],
        v[2] ^ v[7],
        v[3] ^ v[7],
        v[4],
        v[5],
        v[6],
    ]
}

fn xor8(a: &[u64; 8], b: &[u64; 8]) -> [u64; 8] {
    let mut o = [0u64; 8];
    for i in 0..8 {
        o[i] = a[i] ^ b[i];
    }
    o
}

fn load8(st: &Planes, byte: usize) -> [u64; 8] {
    let mut o = [0u64; 8];
    o.copy_from_slice(&st[byte * 8..byte * 8 + 8]);
    o
}

/// out_r = 2 a_r + 3 a_{r+1} + a_{r+2} + a_{r+3} = xtime(a_r + a_{r+1}) + a_{r+1} + a_{r+2} + a_{r+3}.
fn mix_columns(st: &mut Planes) {
    for c in 0..4 {
        let a = [
            load8(st, 4 * c),
            load8(st, 4 * c + 1),
            load8(st, 4 * c + 2),
            load8(st, 4 * c + 3),
        ];
        for r in 0..4 {
            let a1 = &a[(r + 1) % 4];
            let t = xtime(&xor8(&a[r], a1));
            let o = xor8(&xor8(&t, a1), &xor8(&a[(r + 2) % 4], &a[(r + 3) % 4]));
            st[(4 * c + r) * 8..(4 * c + r) * 8 + 8].copy_from_slice(&o);
        }
    }
}

/// Transposes a 64x64 bit matrix held as 64 `u64`s (bit `i` of `a[l]` becomes bit `l` of
/// `a[i]`). Fixed shifts and masks only.
fn transpose64(a: &mut [u64; 64]) {
    let mut j = 32usize;
    let mut m = 0x0000_0000_ffff_ffffu64;
    while j != 0 {
        for k in 0..64 {
            if k & j == 0 {
                let t = ((a[k] >> j) ^ a[k + j]) & m;
                a[k] ^= t << j;
                a[k + j] ^= t;
            }
        }
        j >>= 1;
        m ^= m << j;
    }
}

fn to_planes(blocks: &[[u8; 16]; LANES]) -> Planes {
    let mut st = [0u64; 128];
    for g in 0..2 {
        let mut w = [0u64; 64];
        for (l, block) in blocks.iter().enumerate() {
            let mut bytes = [0u8; 8];
            bytes.copy_from_slice(&block[g * 8..g * 8 + 8]);
            w[l] = u64::from_le_bytes(bytes);
        }
        transpose64(&mut w);
        st[g * 64..g * 64 + 64].copy_from_slice(&w);
        wipe(&mut w);
    }
    st
}

fn from_planes(st: &mut Planes, blocks: &mut [[u8; 16]; LANES]) {
    for g in 0..2 {
        let mut w = [0u64; 64];
        w.copy_from_slice(&st[g * 64..g * 64 + 64]);
        transpose64(&mut w);
        for (l, block) in blocks.iter_mut().enumerate() {
            block[g * 8..g * 8 + 8].copy_from_slice(&w[l].to_le_bytes());
        }
        wipe(&mut w);
    }
}

/// The AES S-box on eight bit planes (`q[k]` holds bit `k`), the Boyar-Peralta circuit.
#[allow(clippy::many_single_char_names)]
fn sbox(q: &mut [u64; 8]) {
    let (x0, x1, x2, x3, x4, x5, x6, x7) = (q[7], q[6], q[5], q[4], q[3], q[2], q[1], q[0]);

    // Top linear transformation.
    let y14 = x3 ^ x5;
    let y13 = x0 ^ x6;
    let y9 = x0 ^ x3;
    let y8 = x0 ^ x5;
    let t0 = x1 ^ x2;
    let y1 = t0 ^ x7;
    let y4 = y1 ^ x3;
    let y12 = y13 ^ y14;
    let y2 = y1 ^ x0;
    let y5 = y1 ^ x6;
    let y3 = y5 ^ y8;
    let t1 = x4 ^ y12;
    let y15 = t1 ^ x5;
    let y20 = t1 ^ x1;
    let y6 = y15 ^ x7;
    let y10 = y15 ^ t0;
    let y11 = y20 ^ y9;
    let y7 = x7 ^ y11;
    let y17 = y10 ^ y11;
    let y19 = y10 ^ y8;
    let y16 = t0 ^ y11;
    let y21 = y13 ^ y16;
    let y18 = x0 ^ y16;

    // Non-linear section.
    let t2 = y12 & y15;
    let t3 = y3 & y6;
    let t4 = t3 ^ t2;
    let t5 = y4 & x7;
    let t6 = t5 ^ t2;
    let t7 = y13 & y16;
    let t8 = y5 & y1;
    let t9 = t8 ^ t7;
    let t10 = y2 & y7;
    let t11 = t10 ^ t7;
    let t12 = y9 & y11;
    let t13 = y14 & y17;
    let t14 = t13 ^ t12;
    let t15 = y8 & y10;
    let t16 = t15 ^ t12;
    let t17 = t4 ^ t14;
    let t18 = t6 ^ t16;
    let t19 = t9 ^ t14;
    let t20 = t11 ^ t16;
    let t21 = t17 ^ y20;
    let t22 = t18 ^ y19;
    let t23 = t19 ^ y21;
    let t24 = t20 ^ y18;
    let t25 = t21 ^ t22;
    let t26 = t21 & t23;
    let t27 = t24 ^ t26;
    let t28 = t25 & t27;
    let t29 = t28 ^ t22;
    let t30 = t23 ^ t24;
    let t31 = t22 ^ t26;
    let t32 = t31 & t30;
    let t33 = t32 ^ t24;
    let t34 = t23 ^ t33;
    let t35 = t27 ^ t33;
    let t36 = t24 & t35;
    let t37 = t36 ^ t34;
    let t38 = t27 ^ t36;
    let t39 = t29 & t38;
    let t40 = t25 ^ t39;
    let t41 = t40 ^ t37;
    let t42 = t29 ^ t33;
    let t43 = t29 ^ t40;
    let t44 = t33 ^ t37;
    let t45 = t42 ^ t41;
    let z0 = t44 & y15;
    let z1 = t37 & y6;
    let z2 = t33 & x7;
    let z3 = t43 & y16;
    let z4 = t40 & y1;
    let z5 = t29 & y7;
    let z6 = t42 & y11;
    let z7 = t45 & y17;
    let z8 = t41 & y10;
    let z9 = t44 & y12;
    let z10 = t37 & y3;
    let z11 = t33 & y4;
    let z12 = t43 & y13;
    let z13 = t40 & y5;
    let z14 = t29 & y2;
    let z15 = t42 & y9;
    let z16 = t45 & y14;
    let z17 = t41 & y8;

    // Bottom linear transformation.
    let t46 = z15 ^ z16;
    let t47 = z10 ^ z11;
    let t48 = z5 ^ z13;
    let t49 = z9 ^ z10;
    let t50 = z2 ^ z12;
    let t51 = z2 ^ z5;
    let t52 = z7 ^ z8;
    let t53 = z0 ^ z3;
    let t54 = z6 ^ z7;
    let t55 = z16 ^ z17;
    let t56 = z12 ^ t48;
    let t57 = t50 ^ t53;
    let t58 = z4 ^ t46;
    let t59 = z3 ^ t54;
    let t60 = t46 ^ t57;
    let t61 = z14 ^ t57;
    let t62 = t52 ^ t58;
    let t63 = t49 ^ t58;
    let t64 = z4 ^ t59;
    let t65 = t61 ^ t62;
    let t66 = z1 ^ t63;
    let s0 = t59 ^ t63;
    let s6 = t56 ^ !t62;
    let s7 = t48 ^ !t60;
    let t67 = t64 ^ t65;
    let s3 = t53 ^ t66;
    let s4 = t51 ^ t66;
    let s5 = t47 ^ t65;
    let s1 = t64 ^ !s3;
    let s2 = t55 ^ !t67;

    q[7] = s0;
    q[6] = s1;
    q[5] = s2;
    q[4] = s3;
    q[3] = s4;
    q[2] = s5;
    q[1] = s6;
    q[0] = s7;
}

#[cfg(test)]
mod tests {
    use super::*;

    /// The S-box from its definition (multiplicative inverse, then the affine map), as an
    /// oracle. Variable time and table-free by construction; tests only.
    fn sbox_ref(x: u8) -> u8 {
        fn mul(mut a: u8, mut b: u8) -> u8 {
            let mut p = 0;
            while b != 0 {
                if b & 1 != 0 {
                    p ^= a;
                }
                a = xtime_byte(a);
                b >>= 1;
            }
            p
        }
        let mut inv = 0u8;
        if x != 0 {
            for c in 1..=255u8 {
                if mul(x, c) == 1 {
                    inv = c;
                }
            }
        }
        let mut s = 0x63u8;
        for k in 0..5 {
            s ^= inv.rotate_left(k);
        }
        s
    }

    #[test]
    fn bitsliced_sbox_matches_the_definition_for_every_byte() {
        // 64 values per call, four calls cover all 256 bytes.
        for base in 0..4u16 {
            let mut q = [0u64; 8];
            for l in 0..64u16 {
                let v = (base * 64 + l) as u8;
                for (k, plane) in q.iter_mut().enumerate() {
                    *plane |= u64::from((v >> k) & 1) << l;
                }
            }
            sbox(&mut q);
            for l in 0..64u16 {
                let v = (base * 64 + l) as u8;
                let mut got = 0u8;
                for (k, plane) in q.iter().enumerate() {
                    got |= (((plane >> l) & 1) as u8) << k;
                }
                assert_eq!(got, sbox_ref(v), "S-box({v:#04x})");
            }
        }
    }

    #[test]
    fn transpose_is_its_own_inverse_and_moves_bit_i_of_word_l_to_bit_l_of_word_i() {
        let mut a = [0u64; 64];
        let mut s = 0x1234_5678_9abc_def0u64;
        for w in a.iter_mut() {
            s ^= s << 13;
            s ^= s >> 7;
            s ^= s << 17;
            *w = s;
        }
        let orig = a;
        transpose64(&mut a);
        for (l, row) in orig.iter().enumerate() {
            for (i, col) in a.iter().enumerate() {
                assert_eq!((col >> l) & 1, (row >> i) & 1);
            }
        }
        transpose64(&mut a);
        assert_eq!(a, orig);
    }

    #[test]
    fn fips_197_appendix_c_example_vectors() {
        let plain = [
            0x00, 0x11, 0x22, 0x33, 0x44, 0x55, 0x66, 0x77, 0x88, 0x99, 0xaa, 0xbb, 0xcc, 0xdd,
            0xee, 0xff,
        ];
        let k128: [u8; 16] = core::array::from_fn(|i| i as u8);
        let k256: [u8; 32] = core::array::from_fn(|i| i as u8);
        let want128 = [
            0x69, 0xc4, 0xe0, 0xd8, 0x6a, 0x7b, 0x04, 0x30, 0xd8, 0xcd, 0xb7, 0x80, 0x70, 0xb4,
            0xc5, 0x5a,
        ];
        let want256 = [
            0x8e, 0xa2, 0xb7, 0xca, 0x51, 0x67, 0x45, 0xbf, 0xea, 0xfc, 0x49, 0x90, 0x4b, 0x49,
            0x60, 0x89,
        ];
        let mut blocks = [plain; LANES];
        Aes::new_128(&k128).encrypt_blocks(&mut blocks);
        assert!(blocks.iter().all(|b| *b == want128));
        let mut blocks = [plain; LANES];
        Aes::new_256(&k256).encrypt_blocks(&mut blocks);
        assert!(blocks.iter().all(|b| *b == want256));
    }
}
