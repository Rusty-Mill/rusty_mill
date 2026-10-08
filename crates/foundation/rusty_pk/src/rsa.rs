//! RSA signature verification: PKCS#1 v1.5 and PSS with SHA-256/384/512.
//!
//! Implements the acceptance rules of `ring` 0.17.14's `RSA_PKCS1_2048_8192_*` and
//! `RSA_PSS_2048_8192_*` verifiers (read from its source, and compared on a recorded
//! test corpus; agreement on tested inputs is not a proof for all inputs). Only these
//! two parameter sets; `ring`'s other RSA verifiers have other rules:
//!
//! - key: DER `RSAPublicKey { n, e }`, minimal positive integers;
//! - `n` odd, at most 8192 bits, and at least 256 bytes long (ring rounds the
//!   bit length up to whole bytes before checking 2048);
//! - `e` odd, 3 to 2^33 - 1;
//! - signature exactly as long as `n`, non-zero and below `n`;
//! - PSS: MGF1 with the same hash, salt length equal to the hash length,
//!   trailer `0xbc`.
//!
//! All inputs are public, so the exponentiation is variable time in `e`.

use alloc::vec;
use alloc::vec::Vec;
use rusty_sha2::{Hash, Sha256, Sha384, Sha512};

use crate::der::Reader;
use crate::mont::{self, Modulus};
use crate::VerifyError;

const MIN_MODULUS_BYTES: usize = 256;
const MAX_MODULUS_BITS: usize = 8192;
const MAX_EXPONENT: u64 = (1 << 33) - 1;

/// Signature scheme and hash.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Scheme {
    /// RSASSA-PKCS1-v1_5 with SHA-256.
    Pkcs1Sha256,
    /// RSASSA-PKCS1-v1_5 with SHA-384.
    Pkcs1Sha384,
    /// RSASSA-PKCS1-v1_5 with SHA-512.
    Pkcs1Sha512,
    /// RSASSA-PSS with SHA-256, MGF1-SHA-256, 32-byte salt.
    PssSha256,
    /// RSASSA-PSS with SHA-384, MGF1-SHA-384, 48-byte salt.
    PssSha384,
    /// RSASSA-PSS with SHA-512, MGF1-SHA-512, 64-byte salt.
    PssSha512,
}

#[derive(Clone, Copy)]
enum Digest {
    Sha256,
    Sha384,
    Sha512,
}

impl Digest {
    fn len(self) -> usize {
        match self {
            Self::Sha256 => 32,
            Self::Sha384 => 48,
            Self::Sha512 => 64,
        }
    }

    fn hash(self, parts: &[&[u8]]) -> Vec<u8> {
        fn run<H: Hash>(parts: &[&[u8]]) -> Vec<u8> {
            let mut h = H::new();
            for p in parts {
                h.update(p);
            }
            h.finalize().as_ref().to_vec()
        }
        match self {
            Self::Sha256 => run::<Sha256>(parts),
            Self::Sha384 => run::<Sha384>(parts),
            Self::Sha512 => run::<Sha512>(parts),
        }
    }

    /// The DER `DigestInfo` prefix that precedes the hash in PKCS#1 v1.5.
    fn digest_info_prefix(self) -> &'static [u8] {
        match self {
            Self::Sha256 => &[
                0x30, 0x31, 0x30, 0x0d, 0x06, 0x09, 0x60, 0x86, 0x48, 0x01, 0x65, 0x03, 0x04, 0x02,
                0x01, 0x05, 0x00, 0x04, 0x20,
            ],
            Self::Sha384 => &[
                0x30, 0x41, 0x30, 0x0d, 0x06, 0x09, 0x60, 0x86, 0x48, 0x01, 0x65, 0x03, 0x04, 0x02,
                0x02, 0x05, 0x00, 0x04, 0x30,
            ],
            Self::Sha512 => &[
                0x30, 0x51, 0x30, 0x0d, 0x06, 0x09, 0x60, 0x86, 0x48, 0x01, 0x65, 0x03, 0x04, 0x02,
                0x03, 0x05, 0x00, 0x04, 0x40,
            ],
        }
    }
}

/// Verifies `signature` over `message` under the DER `RSAPublicKey`
/// `public_key`.
pub fn verify(
    scheme: Scheme,
    public_key: &[u8],
    message: &[u8],
    signature: &[u8],
) -> Result<(), VerifyError> {
    let (n_bytes, e) = parse_public_key(public_key).ok_or(VerifyError)?;
    let mod_bits = bit_length(n_bytes);
    let mod_len = mod_bits.div_ceil(8);
    if mod_len < MIN_MODULUS_BYTES || mod_bits > MAX_MODULUS_BITS || signature.len() != mod_len {
        return Err(VerifyError);
    }
    let limbs = mod_len.div_ceil(8);
    let n = mont::from_be_bytes(n_bytes, limbs).ok_or(VerifyError)?;
    let modulus = Modulus::new(&n).ok_or(VerifyError)?;

    let s = mont::from_be_bytes(signature, limbs).ok_or(VerifyError)?;
    if mont::is_zero_vartime(&s) || !mont::lt_vartime(&s, &n) {
        return Err(VerifyError);
    }
    let (mut sm, mut power, mut m) = (vec![0; limbs], vec![0; limbs], vec![0; limbs]);
    modulus.enter(&mut sm, &s);
    modulus.pow_vartime(&mut power, &sm, &[e]);
    modulus.leave(&mut m, &power);
    let mut em = vec![0u8; mod_len];
    mont::to_be_bytes(&m, &mut em).ok_or(VerifyError)?;

    let ok = match scheme {
        Scheme::Pkcs1Sha256 => pkcs1(Digest::Sha256, message, &em),
        Scheme::Pkcs1Sha384 => pkcs1(Digest::Sha384, message, &em),
        Scheme::Pkcs1Sha512 => pkcs1(Digest::Sha512, message, &em),
        Scheme::PssSha256 => pss(Digest::Sha256, message, &em, mod_bits),
        Scheme::PssSha384 => pss(Digest::Sha384, message, &em, mod_bits),
        Scheme::PssSha512 => pss(Digest::Sha512, message, &em, mod_bits),
    };
    if ok {
        Ok(())
    } else {
        Err(VerifyError)
    }
}

/// `(n magnitude, e)` from DER `RSAPublicKey`, with ring's exponent rules.
fn parse_public_key(der: &[u8]) -> Option<(&[u8], u64)> {
    let mut outer = Reader::new(der);
    let body = outer.tlv(0x30)?;
    if !outer.at_end() {
        return None;
    }
    let mut seq = Reader::new(body);
    let n = seq.positive_integer()?;
    let e = seq.positive_integer()?;
    if !seq.at_end() || e.len() > 5 || n.last()? & 1 == 0 {
        return None;
    }
    let e = e.iter().fold(0u64, |acc, &b| (acc << 8) | b as u64);
    if !(3..=MAX_EXPONENT).contains(&e) || e & 1 == 0 {
        return None;
    }
    Some((n, e))
}

fn bit_length(be: &[u8]) -> usize {
    match be.first() {
        Some(&top) => be.len() * 8 - top.leading_zeros() as usize,
        None => 0,
    }
}

/// EMSA-PKCS1-v1_5: the decoded value must equal the re-encoded expectation.
fn pkcs1(digest: Digest, message: &[u8], em: &[u8]) -> bool {
    let prefix = digest.digest_info_prefix();
    let hash = digest.hash(&[message]);
    let tail = prefix.len() + hash.len();
    if em.len() < tail + 11 {
        return false;
    }
    let pad = em.len() - tail - 3;
    em[0] == 0
        && em[1] == 1
        && em[2..2 + pad].iter().all(|&b| b == 0xff)
        && em[2 + pad] == 0
        && em[3 + pad..3 + pad + prefix.len()] == *prefix
        && em[3 + pad + prefix.len()..] == hash[..]
}

/// EMSA-PSS-VERIFY (RFC 8017 section 9.1.2) with salt length = hash length.
fn pss(digest: Digest, message: &[u8], decoded: &[u8], mod_bits: usize) -> bool {
    let h_len = digest.len();
    let s_len = h_len;
    let em_bits = mod_bits - 1;
    let em_len = em_bits.div_ceil(8);
    let top_mask = 0xffu8 >> (8 * em_len - em_bits);
    // `decoded` is k bytes; EM is one byte shorter when modBits - 1 is a
    // multiple of 8, and the dropped byte must be zero.
    let em = if top_mask == 0xff {
        match decoded.split_first() {
            Some((0, rest)) => rest,
            _ => return false,
        }
    } else {
        decoded
    };
    let Some(db_len) = em_len.checked_sub(1 + s_len) else {
        return false;
    };
    let Some(ps_len) = db_len.checked_sub(h_len + 1) else {
        return false;
    };
    if em.len() != em_len || em[em_len - 1] != 0xbc {
        return false;
    }
    let (masked_db, rest) = em.split_at(db_len);
    let h = &rest[..h_len];
    if masked_db[0] & !top_mask != 0 {
        return false;
    }
    let mut db = mgf1(digest, h, db_len);
    for (d, m) in db.iter_mut().zip(masked_db) {
        *d ^= m;
    }
    db[0] &= top_mask;
    if db[..ps_len].iter().any(|&b| b != 0) || db[ps_len] != 1 {
        return false;
    }
    let salt = &db[db_len - s_len..];
    let m_hash = digest.hash(&[message]);
    let expected = digest.hash(&[&[0u8; 8], &m_hash, salt]);
    expected == h
}

/// MGF1 (RFC 8017 appendix B.2.1).
fn mgf1(digest: Digest, seed: &[u8], len: usize) -> Vec<u8> {
    let mut out = Vec::with_capacity(len + digest.len());
    let mut counter = 0u32;
    while out.len() < len {
        out.extend_from_slice(&digest.hash(&[seed, &counter.to_be_bytes()]));
        counter += 1;
    }
    out.truncate(len);
    out
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn rejects_malformed_keys() {
        let der = |n: &[u8], e: &[u8]| {
            let mut body = vec![0x02, n.len() as u8];
            body.extend_from_slice(n);
            body.extend_from_slice(&[0x02, e.len() as u8]);
            body.extend_from_slice(e);
            let mut out = vec![0x30, body.len() as u8];
            out.extend(body);
            out
        };
        // too small, even modulus, bad exponents, trailing garbage
        assert!(verify(Scheme::Pkcs1Sha256, &der(&[0x7f; 3], &[3]), b"", &[0; 3]).is_err());
        assert!(parse_public_key(&der(&[0x7f, 0x7e], &[3])).is_none());
        assert!(parse_public_key(&der(&[0x7f, 0x7f], &[2])).is_none());
        // even and large enough: 65538 must still be refused
        assert!(parse_public_key(&der(&[0x7f, 0x7f], &[1, 0, 2])).is_none());
        assert!(parse_public_key(&der(&[0x7f, 0x7f], &[1, 0, 1])).is_some());
        assert!(parse_public_key(&der(&[0x7f, 0x7f], &[1])).is_none());
        assert!(parse_public_key(&der(&[0x7f, 0x7f], &[1, 0, 0, 0, 0, 1])).is_none());
        assert!(parse_public_key(&der(&[0x7f, 0x7f], &[3])).is_some());
        let mut trailing = der(&[0x7f, 0x7f], &[3]);
        trailing.push(0);
        assert!(parse_public_key(&trailing).is_none());
    }

    #[test]
    fn bit_length_counts_from_the_top_bit() {
        assert_eq!(bit_length(&[0x01]), 1);
        assert_eq!(bit_length(&[0x80, 0]), 16);
        assert_eq!(bit_length(&[0x00, 0xff]), 8);
        assert_eq!(bit_length(&[]), 0);
    }
}
