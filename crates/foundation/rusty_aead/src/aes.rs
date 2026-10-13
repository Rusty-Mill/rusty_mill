//! AES-128 and AES-256 encryption (FIPS 197), bitsliced, eight blocks per batch.
//!
//! No table is indexed by key or data and nothing branches on either: the S-box is the
//! Boyar-Peralta boolean circuit (113 gates) applied to bit planes. The state is eight `u64`
//! planes; bit `16 * b + p` of plane `k` is bit `k` of byte `p` of block `b` (`p` is the
//! column-major byte position, `4 * column + row`). One S-box circuit therefore handles all
//! 64 bytes of four blocks at once (the layout BearSSL calls `ct64`), and two such plane
//! sets run side by side so the CPU can overlap them, making a batch eight blocks.
//! Encryption only; GCM never needs the inverse cipher.
//!
//! This is the design, not a proof. Whether the compiled code keeps the property is checked
//! by the taint, jump-count and timing evidence described in `docs/research/AES-GCM-DESIGN.md`.

use rusty_crypto_key::wipe;

/// Blocks per plane set (the width of the `ct64` layout).
const GROUP: usize = 4;
/// Independent plane sets processed together, so the CPU can overlap their work.
const GROUPS: usize = 2;
/// Blocks processed per batch.
pub(crate) const LANES: usize = GROUP * GROUPS;

/// One batch of bit planes.
type Planes = [u64; 8];

const MAX_ROUNDS: usize = 14;

/// Expanded AES key, held as round-key planes. Wiped on drop.
pub(crate) struct Aes {
    round_keys: [Planes; MAX_ROUNDS + 1],
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
        let mut round_keys = [[0u64; 8]; MAX_ROUNDS + 1];
        for (r, planes) in round_keys.iter_mut().enumerate().take(rounds + 1) {
            let mut rk = [0u8; 16];
            for c in 0..4 {
                rk[4 * c..4 * c + 4].copy_from_slice(&w[4 * r + c]);
            }
            // The same round key in every block position.
            *planes = to_planes(&[rk; GROUP]);
            wipe(&mut rk);
        }
        for word in w.iter_mut() {
            wipe(word);
        }
        Self { round_keys, rounds }
    }

    /// Encrypts `LANES` blocks in place; unused blocks are ignored (and cost the same).
    pub(crate) fn encrypt_blocks(&self, blocks: &mut [[u8; 16]; LANES]) {
        let mut st: [Planes; GROUPS] = core::array::from_fn(|g| {
            let group: [[u8; 16]; GROUP] = core::array::from_fn(|i| blocks[g * GROUP + i]);
            to_planes(&group)
        });
        for s in st.iter_mut() {
            xor_planes(s, &self.round_keys[0]);
        }
        for r in 1..self.rounds {
            for s in st.iter_mut() {
                sbox(s);
                shift_rows(s);
                mix_columns(s);
                xor_planes(s, &self.round_keys[r]);
            }
        }
        for s in st.iter_mut() {
            sbox(s);
            shift_rows(s);
            xor_planes(s, &self.round_keys[self.rounds]);
        }
        for (g, s) in st.iter_mut().enumerate() {
            let mut group = [[0u8; 16]; GROUP];
            from_planes(s, &mut group);
            blocks[g * GROUP..(g + 1) * GROUP].copy_from_slice(&group);
            wipe(s);
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

fn xor_planes(st: &mut Planes, k: &Planes) {
    for i in 0..8 {
        st[i] ^= k[i];
    }
}

fn xtime_byte(b: u8) -> u8 {
    (b << 1) ^ (0x1b & 0u8.wrapping_sub(b >> 7))
}

/// SubWord for the key schedule: the same circuit, with the four bytes in bit positions 0 to 3.
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

/// Row `r` of the column-major state sits at bit `r` of every nibble.
const ROW: [u64; 4] = [
    0x1111_1111_1111_1111,
    0x2222_2222_2222_2222,
    0x4444_4444_4444_4444,
    0x8888_8888_8888_8888,
];
/// Low `16 - 4r` and high `4r` bits of every 16-bit group (one group per block).
const GROUP_LOW: [u64; 4] = [
    0xffff_ffff_ffff_ffff,
    0x0fff_0fff_0fff_0fff,
    0x00ff_00ff_00ff_00ff,
    0x000f_000f_000f_000f,
];
const GROUP_HIGH: [u64; 4] = [
    0,
    0xf000_f000_f000_f000,
    0xff00_ff00_ff00_ff00,
    0xfff0_fff0_fff0_fff0,
];

/// Output byte `4c + r` takes input byte `4((c + r) mod 4) + r`: row `r` rotates by `r`
/// nibbles inside each 16-bit group.
fn shift_rows(st: &mut Planes) {
    for x in st.iter_mut() {
        let mut out = *x & ROW[0];
        for r in 1..4 {
            let row = *x & ROW[r];
            out |= ((row >> (4 * r)) & GROUP_LOW[r]) | ((row << (16 - 4 * r)) & GROUP_HIGH[r]);
        }
        *x = out;
    }
}

/// Multiplication by x in GF(2^8) on the eight planes (reduction polynomial 0x11b).
fn xtime(v: &Planes) -> Planes {
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

/// out_r = 2 a_r + 3 a_{r+1} + a_{r+2} + a_{r+3} = xtime(a_r + a_{r+1}) + a_{r+1} + a_{r+2} + a_{r+3},
/// where `a_{r+n}` is the column's row `r + n`: a rotation of the rows inside every nibble.
fn mix_columns(st: &mut Planes) {
    let mut a1 = [0u64; 8];
    let mut a2 = [0u64; 8];
    let mut a3 = [0u64; 8];
    let mut v = [0u64; 8];
    for i in 0..8 {
        let x = st[i];
        a1[i] = ((x >> 1) & 0x7777_7777_7777_7777) | ((x << 3) & 0x8888_8888_8888_8888);
        a2[i] = ((x >> 2) & 0x3333_3333_3333_3333) | ((x << 2) & 0xcccc_cccc_cccc_cccc);
        a3[i] = ((x >> 3) & 0x1111_1111_1111_1111) | ((x << 1) & 0xeeee_eeee_eeee_eeee);
        v[i] = x ^ a1[i];
    }
    let t = xtime(&v);
    for i in 0..8 {
        st[i] = t[i] ^ a1[i] ^ a2[i] ^ a3[i];
    }
}

/// Transposes an 8x8 bit matrix held in a `u64` (bit `8r + c` moves to bit `8c + r`).
fn transpose_bits8(mut x: u64) -> u64 {
    let t = (x ^ (x >> 7)) & 0x00aa_00aa_00aa_00aa;
    x ^= t ^ (t << 7);
    let t = (x ^ (x >> 14)) & 0x0000_cccc_0000_cccc;
    x ^= t ^ (t << 14);
    let t = (x ^ (x >> 28)) & 0x0000_0000_f0f0_f0f0;
    x ^ t ^ (t << 28)
}

/// Transposes an 8x8 matrix of bytes held in eight `u64`s (byte `k` of word `j` moves to
/// byte `j` of word `k`).
fn transpose_bytes8(a: &mut [u64; 8]) {
    const MASKS: [u64; 3] = [
        0x00ff_00ff_00ff_00ff,
        0x0000_ffff_0000_ffff,
        0x0000_0000_ffff_ffff,
    ];
    for (stage, mask) in MASKS.iter().enumerate() {
        let d = 1usize << stage;
        for i in 0..8 {
            if i & d == 0 {
                let t = ((a[i] >> (8 * d)) ^ a[i + d]) & mask;
                a[i] ^= t << (8 * d);
                a[i + d] ^= t;
            }
        }
    }
}

fn to_planes(blocks: &[[u8; 16]; GROUP]) -> Planes {
    let mut u = [0u64; 8];
    for (j, word) in u.iter_mut().enumerate() {
        let mut bytes = [0u8; 8];
        bytes.copy_from_slice(&blocks[j / 2][(j % 2) * 8..(j % 2) * 8 + 8]);
        *word = transpose_bits8(u64::from_le_bytes(bytes));
    }
    transpose_bytes8(&mut u);
    u
}

fn from_planes(st: &Planes, blocks: &mut [[u8; 16]; GROUP]) {
    let mut u = *st;
    transpose_bytes8(&mut u);
    for (j, word) in u.iter().enumerate() {
        let bytes = transpose_bits8(*word).to_le_bytes();
        blocks[j / 2][(j % 2) * 8..(j % 2) * 8 + 8].copy_from_slice(&bytes);
    }
    wipe(&mut u);
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

    fn xorshift(s: &mut u64) -> u64 {
        *s ^= *s << 13;
        *s ^= *s >> 7;
        *s ^= *s << 17;
        *s
    }

    #[test]
    fn bit_and_byte_transposes_match_their_definitions() {
        let mut s = 0x1234_5678_9abc_def1u64;
        for _ in 0..200 {
            let x = xorshift(&mut s);
            let y = transpose_bits8(x);
            for r in 0..8 {
                for c in 0..8 {
                    assert_eq!((y >> (8 * c + r)) & 1, (x >> (8 * r + c)) & 1);
                }
            }
            assert_eq!(transpose_bits8(y), x);

            let mut a = [0u64; 8];
            for w in a.iter_mut() {
                *w = xorshift(&mut s);
            }
            let orig = a;
            transpose_bytes8(&mut a);
            for j in 0..8 {
                for k in 0..8 {
                    assert_eq!((a[k] >> (8 * j)) & 0xff, (orig[j] >> (8 * k)) & 0xff);
                }
            }
        }
    }

    #[test]
    fn planes_put_bit_k_of_byte_p_of_block_b_at_bit_16b_plus_p_of_plane_k() {
        let mut s = 0xfeed_beef_1234_5678u64;
        for _ in 0..50 {
            let mut blocks = [[0u8; 16]; GROUP];
            for b in blocks.iter_mut() {
                for byte in b.iter_mut() {
                    *byte = xorshift(&mut s) as u8;
                }
            }
            let st = to_planes(&blocks);
            for (b, block) in blocks.iter().enumerate() {
                for (p, byte) in block.iter().enumerate() {
                    for k in 0..8 {
                        assert_eq!((st[k] >> (16 * b + p)) & 1, u64::from((byte >> k) & 1));
                    }
                }
            }
            let mut back = [[0u8; 16]; GROUP];
            from_planes(&st, &mut back);
            assert_eq!(back, blocks);
        }
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

    /// Where the time goes in one 4-block batch. Ignored (machine-dependent); run with
    /// `cargo test -p rusty_aead --release --lib -- --ignored --nocapture profile`.
    #[test]
    #[ignore = "timing; run manually"]
    fn profile_stages() {
        use core::hint::black_box;
        use std::time::Instant;

        fn ns_per_call(reps: u32, mut f: impl FnMut()) -> f64 {
            (0..5)
                .map(|_| {
                    let t = Instant::now();
                    for _ in 0..reps {
                        f();
                    }
                    t.elapsed().as_nanos() as f64 / f64::from(reps)
                })
                .fold(f64::MAX, f64::min)
        }

        let mut blocks = [[0x5au8; 16]; LANES];
        let mut group = [[0x5au8; 16]; GROUP];
        let mut st = to_planes(&group);
        let reps = 200_000;
        let t_to = ns_per_call(reps, || st = to_planes(black_box(&group)));
        let t_from = ns_per_call(reps, || from_planes(black_box(&st), &mut group));
        let t_sub = ns_per_call(reps, || sbox(black_box(&mut st)));
        let t_shift = ns_per_call(reps, || shift_rows(black_box(&mut st)));
        let t_mix = ns_per_call(reps, || mix_columns(black_box(&mut st)));
        let aes128 = Aes::new_128(&[1u8; 16]);
        let aes256 = Aes::new_256(&[1u8; 32]);
        let t_e128 = ns_per_call(reps, || aes128.encrypt_blocks(black_box(&mut blocks)));
        let t_e256 = ns_per_call(reps, || aes256.encrypt_blocks(black_box(&mut blocks)));
        let t_ks = ns_per_call(reps / 10, || {
            black_box(Aes::new_128(black_box(&[1u8; 16])));
        });
        let mb = |ns: f64| (16 * LANES) as f64 / ns * 1000.0;
        std::eprintln!("per {GROUP}-block group; encrypt_blocks is {LANES} blocks; best of five:");
        std::eprintln!("  to_planes       {t_to:8.1} ns");
        std::eprintln!("  from_planes     {t_from:8.1} ns");
        std::eprintln!("  sbox            {t_sub:8.1} ns   (x10/14 rounds)");
        std::eprintln!("  shift_rows      {t_shift:8.1} ns   (x10/14)");
        std::eprintln!("  mix_columns     {t_mix:8.1} ns   (x9/13)");
        std::eprintln!("  encrypt_blocks  {t_e128:8.1} ns AES-128 = {:6.1} MB/s;  {t_e256:8.1} ns AES-256 = {:6.1} MB/s", mb(t_e128), mb(t_e256));
        std::eprintln!("  key expansion   {t_ks:8.1} ns (AES-128)");
        let sum = t_to + t_from + 10.0 * (t_sub + t_shift) + 9.0 * t_mix;
        std::eprintln!("  sum of parts (AES-128) {sum:8.1} ns (add_round_key not timed)");
    }
}
