//! ECDSA over P-256 and P-384 with DER signatures: verification ([`verify`]) and
//! signing ([`SigningKey`]).
//!
//! # Signing handles a secret
//!
//! [`SigningKey`] signs P-256 with SHA-256 and P-384 with SHA-384, as TLS pairs them. The
//! nonce follows draft-irtf-cfrg-det-sigs-with-noise-05 section 4: 32 bytes of secret
//! noise on P-256, 48 on P-384, before the private key in padded HMAC inputs. Detected
//! RNG errors fail closed. Undetected repeated noise retains deterministic-style security
//! against algebraic key recovery from weak randomness; it does not produce plain RFC 6979
//! outputs. Identical key, message and noise repeat a signature. Hedging mitigates some
//! attacks exploiting determinism, without guaranteeing uniqueness or preventing replay or
//! instruction-skipping faults. Other side-channel defenses remain necessary. `k * G`
//! uses the constant-time scalar multiplication behind [`crate::ecdh`], `k^-1` is Fermat
//! inversion (public exponent, base not branched on), and `s = k^-1 (z + r d)` uses the
//! order field's branch-free arithmetic. The only branches on secret-derived values are on
//! verdicts: key/nonce range checks, a result at infinity (impossible for a valid
//! nonce), and zero r/s. Invalid candidates and zero r/s are retried as RFC 6979 specifies.
//!
//! # Verification
//!

//! Follows `ring` 0.17.14's `ECDSA_P{256,384}_SHA{256,384}_ASN1` acceptance rules (compared
//! on a recorded test corpus, not proven for all inputs):
//! uncompressed SEC1 public keys on the curve, strict DER `SEQUENCE { r, s }`
//! with minimal positive integers, `0 < r, s < n`. The hash may be longer or
//! shorter than the curve order; the leftmost `min(hash, order)` bytes are the
//! integer `e`. All inputs are public, so the arithmetic is variable time.

use alloc::vec::Vec;

use rusty_crypto_key::wipe;
use rusty_sha2::{Hash, HmacSha256, HmacSha384, Sha256, Sha384};

use crate::der::Reader;
use crate::ecdh::{self, ensure, generator_x, is_nonzero, limbs_from_be, scalar_valid, PrivateKey};
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

/// Why [`SigningKey::from_bytes`] or [`SigningKey::sign`] failed.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum SignError {
    /// The scalar is not a valid private key (wrong length, zero, or not below `n`).
    InvalidKey,
    /// The caller's random source failed. Nothing was signed.
    Random,
    /// No usable nonce or signature after the bounded number of attempts (far below
    /// 2^-100 likely for a correct implementation), or an internal parameter did not build.
    Internal,
}

impl core::fmt::Display for SignError {
    fn fmt(&self, f: &mut core::fmt::Formatter<'_>) -> core::fmt::Result {
        f.write_str(match self {
            SignError::InvalidKey => "invalid ECDSA private key",
            SignError::Random => "the random source failed",
            SignError::Internal => "ECDSA signing failed",
        })
    }
}

/// The longest DER signature: two 49-byte integers (a 0x00 pad on 48 bytes), their
/// headers, and a sequence header with a long-form length.
const MAX_DER: usize = 2 * (2 + 49) + 3;

/// Length of the secret fresh noise mixed into each nonce (draft -05 section 4).
pub const fn hedge_len(curve: CurveId) -> usize {
    match curve {
        CurveId::P256 => 32,
        CurveId::P384 => 48,
    }
}

/// RFC 6979 candidates tried before giving up. The chance that one is rejected is about
/// 2^-32 on P-256 and 2^-190 on P-384.
const NONCE_ATTEMPTS: usize = 8;

/// An ECDSA private key on P-256 (signing with SHA-256) or P-384 (SHA-384). Wiped on drop
/// (it wraps [`ecdh::PrivateKey`]); never printed.
pub struct SigningKey {
    id: CurveId,
    key: PrivateKey,
}

impl core::fmt::Debug for SigningKey {
    fn fmt(&self, f: &mut core::fmt::Formatter<'_>) -> core::fmt::Result {
        f.write_str("ecdsa::SigningKey(..)")
    }
}

fn ecdh_curve(id: CurveId) -> ecdh::Curve {
    match id {
        CurveId::P256 => ecdh::Curve::P256,
        CurveId::P384 => ecdh::Curve::P384,
    }
}

impl SigningKey {
    /// A key from exactly 32 (P-256) or 48 (P-384) big-endian bytes encoding `1 <= d < n`.
    /// The range check does the same work for every scalar.
    pub fn from_bytes(curve: CurveId, scalar: &[u8]) -> Result<Self, SignError> {
        let key =
            PrivateKey::from_bytes(ecdh_curve(curve), scalar).map_err(|_| SignError::InvalidKey)?;
        Ok(Self { id: curve, key })
    }

    /// The curve.
    pub fn curve(&self) -> CurveId {
        self.id
    }

    /// The public key `d * G` as an uncompressed SEC 1 point (`0x04 || X || Y`).
    pub fn public_key(&self) -> Result<ecdh::Point, SignError> {
        self.key.public_key().map_err(|_| SignError::Internal)
    }

    /// The DER `SEQUENCE { r, s }` signature over `message` (hashed with SHA-256 on P-256,
    /// SHA-384 on P-384). `fill` supplies [`hedge_len`] fresh secret random bytes for this curve; if
    /// it fails, nothing is signed.
    pub fn sign<F>(
        &self,
        message: &[u8],
        fill: impl FnOnce(&mut [u8]) -> Result<(), F>,
    ) -> Result<Vec<u8>, SignError> {
        let mut hedge = [0u8; 48];
        let len = hedge_len(self.id);
        let filled = fill(&mut hedge[..len]);
        let signature = match filled {
            Ok(()) => self.sign_with_extra(message, &hedge[..len]),
            Err(_) => Err(SignError::Random),
        };
        wipe(&mut hedge);
        signature
    }

    /// Draft -05 signing with curve-sized noise. Only unit-test builds accept empty
    /// noise as a separate, unmodified RFC 6979 path for appendix A vectors.
    fn sign_with_extra(&self, message: &[u8], extra: &[u8]) -> Result<Vec<u8>, SignError> {
        let curve = match self.id {
            CurveId::P256 => Curve::p256(),
            CurveId::P384 => Curve::p384(),
        }
        .ok_or(SignError::Internal)?;
        let digest = match self.id {
            CurveId::P256 => Digest::Sha256,
            CurveId::P384 => Digest::Sha384,
        };
        let len = curve.len;
        let n = &curve.fnn;
        let z = hash_to_scalar(&curve, digest, message).ok_or(SignError::Internal)?;
        let mut h1 = [0u8; 48];
        n.to_be_bytes(&z, &mut h1[..len])
            .ok_or(SignError::Internal)?;

        let mut drbg = Rfc6979::new(self.id, self.key.scalar(), &h1[..len], extra);
        let mut d = limbs_from_be(self.key.scalar());
        let result = (|| {
            for _ in 0..NONCE_ATTEMPTS {
                let mut k_bytes = [0u8; 48];
                drbg.next(&mut k_bytes[..len]);
                let mut k = limbs_from_be(&k_bytes[..len]);
                let valid = ensure(scalar_valid(&k, &n.modulus(), len / 8));
                let signed = match valid {
                    Ok(()) => {
                        sign_with_nonce(&curve, self.key.curve(), &k_bytes[..len], &k, &d, &z)
                    }
                    Err(_) => None,
                };
                wipe(&mut k_bytes);
                wipe(&mut k.0);
                if let Some(signature) = signed {
                    return Ok(signature);
                }
            }
            Err(SignError::Internal)
        })();
        wipe(&mut d.0);
        result
    }
}

/// One attempt with nonce `k` (bytes and limbs, `1 <= k < n`): `r = x(k G) mod n`,
/// `s = k^-1 (z + r d) mod n`, computed with no branch on any value. A zero `r` or `s`
/// (RFC 6979 section 3.4: draw a new nonce) is folded into one verdict for [`ensure`]; the
/// published `r` and `s` then go to [`output`], the only code that branches on them.
fn sign_with_nonce(
    curve: &Curve,
    id: ecdh::Curve,
    k_bytes: &[u8],
    k: &E,
    d: &E,
    z: &E,
) -> Option<Vec<u8>> {
    let n = &curve.fnn;
    let x = generator_x(id, k_bytes).ok()?;
    // x < p < 2n for both curves, so one conditional subtraction reduces it.
    let r = n.add(&x, &E::ZERO);
    let mut k_inv = n.invert(&n.enter(k));
    let mut rd = n.mul(&n.enter(&r), &n.enter(d));
    let mut t = n.add(&n.enter(z), &rd);
    let s = n.leave(&n.mul(&k_inv, &t));
    wipe(&mut k_inv.0);
    wipe(&mut rd.0);
    wipe(&mut t.0);
    let words = curve.len / 8;
    ensure(is_nonzero(&r, words) & is_nonzero(&s, words)).ok()?;
    output(curve, &r, &s)
}

/// The DER signature from `r` and `s`, which are the **public output**: the encoding's
/// lengths and padding branch on them, and this function (with what it calls) is the only
/// place that does. It is a separate symbol so the taint run can tell its reports (on values
/// that are published anyway) from any on the key or nonce; see
/// `scripts/verdict_sites.py --output`.
#[inline(never)]
fn output(curve: &Curve, r: &E, s: &E) -> Option<Vec<u8>> {
    der_signature(curve, r, s)
}

/// `SEQUENCE { INTEGER r, INTEGER s }` with minimal positive integers.
#[inline(always)]
fn der_signature(curve: &Curve, r: &E, s: &E) -> Option<Vec<u8>> {
    let len = curve.len;
    let integer = |v: &E| -> Option<Vec<u8>> {
        let mut be = [0u8; 48];
        curve.fnn.to_be_bytes(v, &mut be[..len])?;
        let first = be[..len].iter().position(|&b| b != 0)?;
        let mut out = Vec::with_capacity(MAX_DER);
        out.push(0x02);
        let body = &be[first..len];
        let pad = body[0] & 0x80 != 0;
        out.push((body.len() + usize::from(pad)) as u8);
        if pad {
            out.push(0);
        }
        out.extend_from_slice(body);
        Some(out)
    };
    let (r, s) = (integer(r)?, integer(s)?);
    // A fixed capacity, so the allocation (and the `Vec`'s niche) does not depend on r, s.
    let mut out = Vec::with_capacity(MAX_DER);
    out.push(0x30);
    let body = r.len() + s.len();
    if body >= 0x80 {
        out.push(0x81);
    }
    out.push(body as u8);
    out.extend_from_slice(&r);
    out.extend_from_slice(&s);
    Some(out)
}

/// RFC 6979 section 3.2's HMAC-DRBG, with `hlen == qlen` (SHA-256 on P-256, SHA-384 on
/// P-384), so each candidate is exactly one `V`. Its state is wiped on drop.
struct Rfc6979 {
    id: CurveId,
    k: [u8; 48],
    v: [u8; 48],
    started: bool,
}

impl Drop for Rfc6979 {
    fn drop(&mut self) {
        wipe(&mut self.k);
        wipe(&mut self.v);
    }
}

impl Rfc6979 {
    fn len(&self) -> usize {
        match self.id {
            CurveId::P256 => 32,
            CurveId::P384 => 48,
        }
    }

    /// `HMAC_K(parts...)` into `out` (the hash length).
    fn mac(&self, parts: &[&[u8]], out: &mut [u8; 48]) {
        let len = self.len();
        match self.id {
            CurveId::P256 => {
                let mut h = HmacSha256::new(&self.k[..len]);
                parts.iter().for_each(|p| h.update(p));
                out[..32].copy_from_slice(&h.finalize());
            }
            CurveId::P384 => {
                let mut h = HmacSha384::new(&self.k[..len]);
                parts.iter().for_each(|p| h.update(p));
                out[..48].copy_from_slice(&h.finalize());
            }
        }
    }

    /// RFC 6979 a-f with d/f replaced by draft -05 section 4. Padding starts
    /// the key in a new hash block after the noise, then h1 in another new block.
    /// The same noise is reused in d/f; step e updates V between them.
    fn new(id: CurveId, x: &[u8], h1: &[u8], extra: &[u8]) -> Self {
        let mut drbg = Self {
            id,
            k: [0; 48],
            v: [0; 48],
            started: false,
        };
        let len = drbg.len();
        drbg.v[..len].fill(0x01);
        for tag in [0x00u8, 0x01] {
            let mut next = [0u8; 48];
            let zeros = [0u8; 80];
            let (pad1, pad2) = match id {
                CurveId::P256 => (63, 32), // (32 + 1 + 32 + 63) % 64 == 0
                CurveId::P384 => (31, 80), // (48 + 1 + 48 + 31) % 128 == 0
            };
            #[cfg(test)]
            if extra.is_empty() {
                // Test-only original RFC 6979, never an RNG-error fallback.
                drbg.mac(&[&drbg.v[..len], &[tag], x, h1], &mut next);
            } else {
                drbg.mac(
                    &[
                        &drbg.v[..len],
                        &[tag],
                        extra,
                        &zeros[..pad1],
                        x,
                        &zeros[..pad2],
                        h1,
                    ],
                    &mut next,
                );
            }
            #[cfg(not(test))]
            drbg.mac(
                &[
                    &drbg.v[..len],
                    &[tag],
                    extra,
                    &zeros[..pad1],
                    x,
                    &zeros[..pad2],
                    h1,
                ],
                &mut next,
            );
            drbg.k = next;
            drbg.mac(&[&drbg.v[..len]], &mut next);
            drbg.v = next;
            wipe(&mut next);
        }
        drbg
    }

    /// The next candidate (step h): after the first, `K = HMAC_K(V || 0x00)`, `V = HMAC_K(V)`
    /// first; then `V = HMAC_K(V)` and `T = V`.
    fn next(&mut self, out: &mut [u8]) {
        let len = self.len();
        let mut next = [0u8; 48];
        if self.started {
            self.mac(&[&self.v[..len], &[0x00]], &mut next);
            self.k = next;
            self.mac(&[&self.v[..len]], &mut next);
            self.v = next;
        }
        self.started = true;
        self.mac(&[&self.v[..len]], &mut next);
        self.v = next;
        wipe(&mut next);
        out.copy_from_slice(&self.v[..len]);
    }
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

#[cfg(test)]
mod tests {
    use super::*;
    use alloc::format;
    use alloc::string::String;
    use alloc::vec;

    fn hex(s: &str) -> Vec<u8> {
        (0..s.len())
            .step_by(2)
            .map(|i| u8::from_str_radix(&s[i..i + 2], 16).unwrap())
            .collect()
    }

    /// The `(r, s)` of a DER signature, as fixed-width big-endian hex.
    fn r_s(der: &[u8], len: usize) -> (String, String) {
        let mut outer = Reader::new(der);
        let mut seq = Reader::new(outer.tlv(0x30).unwrap());
        let field = |v: &[u8]| {
            let mut out = vec![0u8; len - v.len()];
            out.extend_from_slice(v);
            out.iter().map(|b| format!("{b:02X}")).collect::<String>()
        };
        let (r, s) = (
            seq.positive_integer().unwrap(),
            seq.positive_integer().unwrap(),
        );
        (field(r), field(s))
    }

    /// RFC 6979 appendix A.2.5 (P-256, SHA-256) and A.2.6 (P-384, SHA-384), messages
    /// "sample" and "test", through the deterministic path (no additional data).
    #[test]
    fn rfc_6979_appendix_a_vectors() {
        let cases = [
            (
                CurveId::P256,
                "C9AFA9D845BA75166B5C215767B1D6934E50C3DB36E89B127B8A622B120F6721",
                "sample",
                "EFD48B2AACB6A8FD1140DD9CD45E81D69D2C877B56AAF991C34D0EA84EAF3716",
                "F7CB1C942D657C41D436C7A1B6E29F65F3E900DBB9AFF4064DC4AB2F843ACDA8",
            ),
            (
                CurveId::P256,
                "C9AFA9D845BA75166B5C215767B1D6934E50C3DB36E89B127B8A622B120F6721",
                "test",
                "F1ABB023518351CD71D881567B1EA663ED3EFCF6C5132B354F28D3B0B7D38367",
                "019F4113742A2B14BD25926B49C649155F267E60D3814B4C0CC84250E46F0083",
            ),
            (
                CurveId::P384,
                "6B9D3DAD2E1B8C1C05B19875B6659F4DE23C3B667BF297BA9AA47740787137D896D5724E4C70A825F872C9EA60D2EDF5",
                "sample",
                "94EDBB92A5ECB8AAD4736E56C691916B3F88140666CE9FA73D64C4EA95AD133C81A648152E44ACF96E36DD1E80FABE46",
                "99EF4AEB15F178CEA1FE40DB2603138F130E740A19624526203B6351D0A3A94FA329C145786E679E7B82C71A38628AC8",
            ),
            (
                CurveId::P384,
                "6B9D3DAD2E1B8C1C05B19875B6659F4DE23C3B667BF297BA9AA47740787137D896D5724E4C70A825F872C9EA60D2EDF5",
                "test",
                "8203B63D3C853E8D77227FB377BCF7B7B772E97892A80F36AB775D509D7A5FEB0542A7F0812998DA8F1DD3CA3CF023DB",
                "DDD0760448D42D8A43AF45AF836FCE4DE8BE06B485E9B61B827C2F13173923E06A739F040649A667BF3B828246BAA5A5",
            ),
        ];
        for (curve, x, msg, want_r, want_s) in cases {
            let key = SigningKey::from_bytes(curve, &hex(x)).unwrap();
            let sig = key.sign_with_extra(msg.as_bytes(), &[]).unwrap();
            let len = if curve == CurveId::P256 { 32 } else { 48 };
            assert_eq!(
                r_s(&sig, len),
                (want_r.into(), want_s.into()),
                "{curve:?} {msg}"
            );
            let digest = if curve == CurveId::P256 {
                Digest::Sha256
            } else {
                Digest::Sha384
            };
            let public = key.public_key().unwrap();
            assert!(verify(curve, digest, public.as_bytes(), msg.as_bytes(), &sig).is_ok());
        }
    }

    /// Independently generated by scripts/gen_hedged_ecdsa.py using Python
    /// hashlib/HMAC and affine big-integer curve arithmetic. Pins both HMAC
    /// block boundaries, initial K/V, the nonce and the final signature.
    #[test]
    fn draft_05_fixed_noise_known_answers() {
        for line in include_str!("../tests/vectors/hedged_ecdsa.txt").lines() {
            let fields: Vec<_> = line.split_whitespace().collect();
            let id = if fields[0] == "256" {
                CurveId::P256
            } else {
                CurveId::P384
            };
            let len = hedge_len(id);
            let message_len: usize = fields[1].parse().unwrap();
            let noise_byte: u8 = fields[2].parse().unwrap();
            let message = vec![b'a'; message_len];
            let noise = vec![noise_byte; len];
            let scalar = vec![0x11; len];
            let curve = if id == CurveId::P256 {
                Curve::p256()
            } else {
                Curve::p384()
            }
            .unwrap();
            let digest = if id == CurveId::P256 {
                Digest::Sha256
            } else {
                Digest::Sha384
            };
            let z = hash_to_scalar(&curve, digest, &message).unwrap();
            let mut h = vec![0; len];
            curve.fnn.to_be_bytes(&z, &mut h).unwrap();
            let mut drbg = Rfc6979::new(id, &scalar, &h, &noise);
            assert_eq!(&drbg.k[..len], hex(fields[3]), "K: {line}");
            assert_eq!(&drbg.v[..len], hex(fields[4]), "V: {line}");
            let mut nonce = vec![0; len];
            drbg.next(&mut nonce);
            assert_eq!(nonce, hex(fields[5]), "nonce: {line}");
            let key = SigningKey::from_bytes(id, &scalar).unwrap();
            let signature = key
                .sign(&message, |out: &mut [u8]| {
                    assert_eq!(out.len(), len);
                    out.copy_from_slice(&noise);
                    Ok::<(), ()>(())
                })
                .unwrap();
            assert_eq!(
                r_s(&signature, len),
                (fields[6].to_uppercase(), fields[7].to_uppercase())
            );
            assert_eq!(signature, key.sign_with_extra(&message, &noise).unwrap());
            assert_ne!(
                signature,
                key.sign_with_extra(&message, &[]).unwrap(),
                "fixed noise must not be confused with plain RFC 6979"
            );
            let public = key.public_key().unwrap();
            assert!(verify(id, digest, public.as_bytes(), &message, &signature).is_ok());
        }
    }

    /// The deterministic path repeats; the hedge changes the nonce.
    #[test]
    fn the_hedge_changes_the_nonce_and_nothing_else() {
        let key = SigningKey::from_bytes(CurveId::P256, &[0x11; 32]).unwrap();
        let a = key.sign_with_extra(b"m", &[]).unwrap();
        assert_eq!(a, key.sign_with_extra(b"m", &[]).unwrap());
        let b = key.sign_with_extra(b"m", &[1; 32]).unwrap();
        let c = key.sign_with_extra(b"m", &[2; 32]).unwrap();
        assert_ne!(r_s(&a, 32).0, r_s(&b, 32).0);
        assert_ne!(r_s(&b, 32).0, r_s(&c, 32).0);
    }

    #[test]
    fn der_integers_are_minimal_and_positive() {
        let curve = Curve::p256().unwrap();
        let mut high = E::ZERO;
        high.0[3] = 0x8000_0000_0000_0000; // top bit set: needs a 0x00 pad
        let small = E::from_limbs(&[5]);
        let der = der_signature(&curve, &high, &small).unwrap();
        assert_eq!(&der[..4], &[0x30, 33 + 2 + 1 + 2, 0x02, 33]);
        assert_eq!(der[4], 0x00);
        assert_eq!(&der[der.len() - 3..], &[0x02, 1, 5]);
    }
}
