//! ECDSA signature verification over P-256 and P-384 with DER signatures.
//!
//! Accepts what `ring`'s `ECDSA_P{256,384}_SHA{256,384}_ASN1` accept:
//! uncompressed SEC1 public keys on the curve, strict DER `SEQUENCE { r, s }`
//! with minimal positive integers, `0 < r, s < n`. The hash may be longer or
//! shorter than the curve order; the leftmost `min(hash, order)` bytes are the
//! integer `e`. All inputs are public, so the arithmetic is variable time.

use rusty_sha2::{Hash, Sha256, Sha384};

use crate::der::Reader;
use crate::field::E;
use crate::weierstrass::Curve;
use crate::VerifyError;

/// The curve of the public key.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum CurveId {
    /// NIST P-256 (secp256r1).
    P256,
    /// NIST P-384 (secp384r1).
    P384,
}

/// The message digest.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Digest {
    /// SHA-256.
    Sha256,
    /// SHA-384.
    Sha384,
}

/// Verifies the DER ECDSA `signature` over `message` under the uncompressed
/// SEC1 `public_key` (`0x04 || X || Y`).
pub fn verify(
    curve: CurveId,
    digest: Digest,
    public_key: &[u8],
    message: &[u8],
    signature: &[u8],
) -> Result<(), VerifyError> {
    let curve = match curve {
        CurveId::P256 => Curve::p256(),
        CurveId::P384 => Curve::p384(),
    }
    .ok_or(VerifyError)?;
    let len = curve.len;

    let q = match public_key.split_first() {
        Some((0x04, xy)) if xy.len() == 2 * len => curve
            .point_from_be(&xy[..len], &xy[len..])
            .ok_or(VerifyError)?,
        _ => return Err(VerifyError),
    };

    let (r, s) = parse_signature(&curve, signature).ok_or(VerifyError)?;
    let e = hash_to_scalar(&curve, digest, message).ok_or(VerifyError)?;

    let n = &curve.fnn;
    let w = n.invert(&n.enter(&s));
    let u1 = n.leave(&n.mul(&n.enter(&e), &w));
    let u2 = n.leave(&n.mul(&n.enter(&r), &w));

    let point = curve.mul2_vartime(&u1, &u2, &q);
    let x = curve.affine_x(&point).ok_or(VerifyError)?;
    // x < p < 2n for both curves, so one conditional subtraction reduces it.
    let v = n.add(&x, &E::ZERO);
    if n.eq_vartime(&v, &r) {
        Ok(())
    } else {
        Err(VerifyError)
    }
}

fn parse_signature(curve: &Curve, der: &[u8]) -> Option<(E, E)> {
    let mut outer = Reader::new(der);
    let body = outer.tlv(0x30)?;
    if !outer.at_end() {
        return None;
    }
    let mut seq = Reader::new(body);
    let (r, s) = (seq.positive_integer()?, seq.positive_integer()?);
    if !seq.at_end() {
        return None;
    }
    let n = &curve.fnn;
    let (r, s) = (n.parse_be(r)?, n.parse_be(s)?);
    let valid = |v: &E| !n.is_zero_vartime(v) && n.is_reduced_vartime(v);
    (valid(&r) && valid(&s)).then_some((r, s))
}

/// The integer `e` of FIPS 186-4 section 6.4, reduced modulo `n`.
fn hash_to_scalar(curve: &Curve, digest: Digest, message: &[u8]) -> Option<E> {
    let hashed: alloc::vec::Vec<u8> = match digest {
        Digest::Sha256 => Sha256::digest(message).to_vec(),
        Digest::Sha384 => Sha384::digest(message).to_vec(),
    };
    let used = &hashed[..hashed.len().min(curve.len)];
    let mut padded = [0u8; 48];
    padded[curve.len - used.len()..curve.len].copy_from_slice(used);
    let e = curve.fnn.parse_be(&padded[..curve.len])?;
    // e < 2^(8*len) < 2n, so adding zero reduces it once.
    Some(curve.fnn.add(&e, &E::ZERO))
}
