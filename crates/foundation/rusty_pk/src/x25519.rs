//! X25519 Diffie-Hellman (RFC 7748 section 5) on the Montgomery ladder.
//!
//! The scalar is secret. The ladder runs a fixed 255 iterations, selects with
//! [`Field::cswap`] (masks, no branch) and uses only the branch-free field
//! routines; the final inversion has a public exponent. There is no
//! secret-dependent branch or index by construction. That is checked by
//! valgrind taint runs, a disassembly budget and a timing test
//! (`scripts/ct_check.sh`), and **not proven**.
//!
//! Intermediate field elements live in stack copies this crate cannot wipe
//! individually; only the clamped scalar and the final ladder state are wiped.

use rusty_crypto_key::{constant_time_eq, wipe};

use crate::field::{Field, E};
use crate::params::F25519_P;

/// Length of scalars, coordinates and shared secrets.
pub const KEY_LEN: usize = 32;

/// The u-coordinate of the base point.
const BASE_POINT: [u8; KEY_LEN] = {
    let mut u = [0u8; KEY_LEN];
    u[0] = 9;
    u
};

/// The peer's point has small order, so the shared secret is all zeros
/// (RFC 7748 section 6.1; RFC 8446 section 7.4.2 requires aborting).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct LowOrderPoint;

impl core::fmt::Display for LowOrderPoint {
    fn fmt(&self, f: &mut core::fmt::Formatter<'_>) -> core::fmt::Result {
        f.write_str("X25519 peer public key has small order")
    }
}

/// `scalar * u` (RFC 7748 `X25519(k, u)`), constant time in `scalar`.
/// `u` is public. An all-zero result is returned as is; use [`agree`] to
/// reject it.
pub fn x25519(scalar: &[u8; KEY_LEN], u: &[u8; KEY_LEN]) -> [u8; KEY_LEN] {
    // The prime is a compile-time constant, so construction cannot fail; the
    // zero result is only a panic-free fallback for that impossible case.
    let Some(f) = Field::new(&F25519_P) else {
        return [0; KEY_LEN];
    };

    let mut k = *scalar;
    k[0] &= 248;
    k[31] &= 127;
    k[31] |= 64;

    let mut be = *u;
    be[31] &= 0x7f; // RFC 7748: ignore the top bit
    be.reverse();
    let x1 = match f.parse_be(&be) {
        // < 2^255 < 2p, so adding zero reduces once.
        Some(x) => f.enter(&f.add(&x, &E::ZERO)),
        None => return [0; KEY_LEN],
    };
    let a24 = f.enter(&E::from_limbs(&[121_665]));

    let (mut x2, mut z2) = (f.one(), E::ZERO);
    let (mut x3, mut z3) = (x1, f.one());
    let mut swap = 0u64;
    for t in (0..255).rev() {
        let bit = u64::from((k[t / 8] >> (t % 8)) & 1);
        swap ^= bit;
        Field::cswap(&mut x2, &mut x3, swap);
        Field::cswap(&mut z2, &mut z3, swap);
        swap = bit;

        let a = f.add(&x2, &z2);
        let aa = f.sqr(&a);
        let b = f.sub(&x2, &z2);
        let bb = f.sqr(&b);
        let e = f.sub(&aa, &bb);
        let c = f.add(&x3, &z3);
        let d = f.sub(&x3, &z3);
        let da = f.mul(&d, &a);
        let cb = f.mul(&c, &b);
        x3 = f.sqr(&f.add(&da, &cb));
        z3 = f.mul(&x1, &f.sqr(&f.sub(&da, &cb)));
        x2 = f.mul(&aa, &bb);
        z2 = f.mul(&e, &f.add(&aa, &f.mul(&a24, &e)));
    }
    Field::cswap(&mut x2, &mut x3, swap);
    Field::cswap(&mut z2, &mut z3, swap);

    let result = f.leave(&f.mul(&x2, &f.invert(&z2)));
    let mut out_be = [0u8; KEY_LEN];
    // A reduced element always fits in 32 bytes.
    let _ = f.to_be_bytes(&result, &mut out_be);
    out_be.reverse();

    wipe(&mut k);
    wipe(&mut x2.0);
    wipe(&mut z2.0);
    wipe(&mut x3.0);
    wipe(&mut z3.0);
    out_be
}

/// The public key for `scalar`: `X25519(scalar, 9)`.
pub fn public_key(scalar: &[u8; KEY_LEN]) -> [u8; KEY_LEN] {
    x25519(scalar, &BASE_POINT)
}

/// The shared secret with `peer`, or [`LowOrderPoint`] if it is all zeros.
///
/// The all-zero test depends only on `peer`, never on `scalar` (a zero result
/// means `peer` has small order), so branching on it reveals nothing secret.
pub fn agree(scalar: &[u8; KEY_LEN], peer: &[u8; KEY_LEN]) -> Result<[u8; KEY_LEN], LowOrderPoint> {
    let mut shared = x25519(scalar, peer);
    if constant_time_eq(&shared, &[0u8; KEY_LEN]) {
        wipe(&mut shared);
        return Err(LowOrderPoint);
    }
    Ok(shared)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn h(s: &str) -> [u8; 32] {
        let mut out = [0u8; 32];
        for (i, b) in out.iter_mut().enumerate() {
            *b = u8::from_str_radix(&s[2 * i..2 * i + 2], 16).unwrap();
        }
        out
    }

    #[test]
    fn rfc_7748_section_5_2_vectors() {
        assert_eq!(
            x25519(
                &h("a546e36bf0527c9d3b16154b82465edd62144c0ac1fc5a18506a2244ba449ac4"),
                &h("e6db6867583030db3594c1a424b15f7c726624ec26b3353b10a903a6d0ab1c4c")
            ),
            h("c3da55379de9c6908e94ea4df28d084f32eccf03491c71f754b4075577a28552")
        );
        assert_eq!(
            x25519(
                &h("4b66e9d4d1b4673c5ad22691957d6af5c11b6421e0ea01d42ca4169e7918ba0d"),
                &h("e5210f12786811d3f4b7959d0538ae2c31dbe7106fc03c3efc4cd549c715a493")
            ),
            h("95cbde9476e8907d7aade45cb4b873f88b595a68799fa152e6f8f7647aac7957")
        );
    }

    #[test]
    fn rfc_7748_section_6_1_key_agreement() {
        let alice = h("77076d0a7318a57d3c16c17251b26645df4c2f87ebc0992ab177fba51db92c2a");
        let bob = h("5dab087e624a8a4b79e17f8b83800ee66f3bb1292618b6fd1c2f8b27ff88e0eb");
        let (pa, pb) = (public_key(&alice), public_key(&bob));
        assert_eq!(
            pa,
            h("8520f0098930a754748b7ddcb43ef75a0dbf3a0d26381af4eba4a98eaa9b4e6a")
        );
        assert_eq!(
            pb,
            h("de9edb7d7b7dc1b4d35b61c2ece435373f8343c85b78674dadfc7e146f882b4f")
        );
        let shared = h("4a5d9d5ba4ce2de1728e3bf480350f25e07e21c947d19e3376f09b3c1e161742");
        assert_eq!(agree(&alice, &pb), Ok(shared));
        assert_eq!(agree(&bob, &pa), Ok(shared));
    }

    #[test]
    fn rfc_7748_iterated_1_and_1000() {
        let (mut k, mut u) = (BASE_POINT, BASE_POINT);
        for i in 1..=1000 {
            let next = x25519(&k, &u);
            u = k;
            k = next;
            if i == 1 {
                assert_eq!(
                    k,
                    h("422c8e7a6227d7bca1350b3e2bb7279f7897b87bb6854b783c60e80311ae3079")
                );
            }
        }
        assert_eq!(
            k,
            h("684cf59ba83309552800ef566f2f4d3c1c3887c49360e3875f2eb94d99532c51")
        );
    }

    #[test]
    fn small_order_points_are_rejected_by_agree() {
        let scalar = [0x77u8; 32];
        let zero = [0u8; 32];
        let mut one = [0u8; 32];
        one[0] = 1;
        assert_eq!(agree(&scalar, &zero), Err(LowOrderPoint));
        assert_eq!(agree(&scalar, &one), Err(LowOrderPoint));
        // The raw function still returns the zeros.
        assert_eq!(x25519(&scalar, &zero), [0u8; 32]);
    }

    #[test]
    fn top_bit_of_u_is_ignored() {
        let scalar = [0x31u8; 32];
        let mut u = BASE_POINT;
        let plain = x25519(&scalar, &u);
        u[31] |= 0x80;
        assert_eq!(x25519(&scalar, &u), plain);
    }
}
