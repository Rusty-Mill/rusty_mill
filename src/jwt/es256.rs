//! `ES256` (ECDSA using P-256 and SHA-256, RFC 7518 §3.4) JWT signing and
//! verification, backed by the hand-rolled elliptic curve arithmetic in
//! [`crate::crypto::ecc`].
//!
//! Unlike [`crate::jwt::rsa`], which only verifies, this module also
//! *signs* -- something worth being deliberate about. ECDSA signing needs
//! a fresh, unpredictable nonce (`k`) for every signature; reusing one or
//! leaking a few bits of it across signatures directly recovers the
//! private key (this is not a theoretical concern -- it broke the Sony
//! PS3's signing key, and has broken production systems whose RNG was
//! weak or predictable). To sidestep RNG-quality entirely,
//! [`sign_p256_sha256`] uses RFC 6979 deterministic nonce generation: `k` is
//! derived from the private key and message hash via HMAC-SHA256, so the
//! same inputs always produce the same (valid) signature and there is no
//! RNG call to get wrong. See [`crate::crypto::ecc`]'s module docs for
//! the residual (non-constant-time) risk this doesn't address.

use crate::crypto::bigint::BigUint;
use crate::crypto::ecc::{self, Point};
use crate::crypto::hmac::hmac_sha256;
use crate::crypto::sha256::sha256;
use crate::encoding::base64::{decode_url_safe, encode_url_safe_no_pad};
use crate::error::{Error, Result};
use crate::json::Value;
use std::cmp::Ordering;

/// An ECDSA P-256 public key, as used to verify `ES256`-signed tokens.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct EcPublicKey {
    point: Point,
}

impl EcPublicKey {
    /// Builds a key from raw big-endian `x`/`y` affine coordinates (as
    /// decoded from a JWK's `x`/`y` members, RFC 7518 §6.2.1). Rejects
    /// coordinates that don't describe a point actually on the P-256
    /// curve (see [`Point::is_on_curve`] -- this is a required defense
    /// against invalid-curve attacks, not an optional sanity check).
    pub fn from_affine_coordinates(x: &[u8], y: &[u8]) -> Result<Self> {
        let point = Point::from_affine_coordinates(x, y).ok_or_else(|| {
            Error::Validation("EC public key point is not on the P-256 curve".to_string())
        })?;
        Ok(EcPublicKey { point })
    }

    /// Builds a key directly from a JWK's base64url-encoded `x` and `y`
    /// members (`kty: "EC"`, `crv: "P-256"`).
    pub fn from_jwk_base64url(x_b64: &str, y_b64: &str) -> Result<Self> {
        let x = decode_url_safe(x_b64)?;
        let y = decode_url_safe(y_b64)?;
        Self::from_affine_coordinates(&x, &y)
    }
}

/// An ECDSA P-256 private key (a scalar `d` in `[1, n-1]`), used to sign
/// `ES256` JWTs.
#[derive(Clone)]
pub struct EcPrivateKey {
    d: BigUint,
}

impl std::fmt::Debug for EcPrivateKey {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_tuple("EcPrivateKey").field(&"[redacted]").finish()
    }
}

impl EcPrivateKey {
    /// Builds a private key from a raw big-endian scalar (as decoded from
    /// a JWK's `d` member). Rejects `0` and anything `>= n` (the curve
    /// order) -- both invalid as ECDSA private keys.
    pub fn from_bytes(d_bytes: &[u8]) -> Result<Self> {
        let n = ecc::order();
        let d = BigUint::from_bytes_be(d_bytes);
        if d.is_zero() || d.compare(&n) != Ordering::Less {
            return Err(Error::Validation(
                "EC private key scalar is out of range [1, n-1]".to_string(),
            ));
        }
        Ok(EcPrivateKey { d })
    }

    /// Derives this private key's public key (`d * G`).
    pub fn public_key(&self) -> EcPublicKey {
        EcPublicKey {
            point: ecc::base_point().scalar_mul(&self.d),
        }
    }
}

/// RFC 6979 §3.2 deterministic nonce generation, specialized to P-256 +
/// SHA-256 (whose digest and curve-order bit lengths are both exactly
/// 256, which collapses several of the general algorithm's bit-length
/// bookkeeping steps -- `bits2octets`/`bits2int` become direct 32-byte
/// operations with no truncation/padding case to handle).
struct DeterministicNonce {
    k: [u8; 32],
    v: [u8; 32],
}

fn bits2octets(hash: &[u8; 32], n: &BigUint) -> [u8; 32] {
    let z = BigUint::from_bytes_be(hash).rem(n);
    let mut out = [0u8; 32];
    out.copy_from_slice(
        &z.to_bytes_be_padded(32)
            .expect("reduced mod n fits in 32 bytes"),
    );
    out
}

impl DeterministicNonce {
    fn new(private_key: &BigUint, hash: &[u8; 32]) -> Self {
        let n = ecc::order();
        let x_bytes = private_key
            .to_bytes_be_padded(32)
            .expect("private key scalar fits in 32 bytes");
        let h1 = bits2octets(hash, &n);

        let mut v = [0x01u8; 32];
        let mut k = [0x00u8; 32];

        let mut buf = Vec::with_capacity(32 + 1 + 32 + 32);
        buf.extend_from_slice(&v);
        buf.push(0x00);
        buf.extend_from_slice(&x_bytes);
        buf.extend_from_slice(&h1);
        k = hmac_sha256(&k, &buf);
        v = hmac_sha256(&k, &v);

        buf.clear();
        buf.extend_from_slice(&v);
        buf.push(0x01);
        buf.extend_from_slice(&x_bytes);
        buf.extend_from_slice(&h1);
        k = hmac_sha256(&k, &buf);
        v = hmac_sha256(&k, &v);

        DeterministicNonce { k, v }
    }

    /// Produces the next deterministic candidate `k` in `[1, n-1]`. Only
    /// ever called more than once if the (astronomically unlikely, ~2^-128)
    /// event of a candidate producing `r = 0` or `s = 0` occurs, per RFC
    /// 6979 §3.2 step h.3's "try again" instruction -- each call advances
    /// the underlying HMAC state, so successive candidates always differ.
    fn next_candidate(&mut self) -> BigUint {
        let n = ecc::order();
        loop {
            self.v = hmac_sha256(&self.k, &self.v);
            let candidate = BigUint::from_bytes_be(&self.v);
            if !candidate.is_zero() && candidate.compare(&n) == Ordering::Less {
                return candidate;
            }
            let mut buf = self.v.to_vec();
            buf.push(0x00);
            self.k = hmac_sha256(&self.k, &buf);
            self.v = hmac_sha256(&self.k, &self.v);
        }
    }
}

/// Computes a raw `r || s` ECDSA signature (RFC 7518 §3.4's fixed
/// 64-byte format -- not DER) over `message`, using RFC 6979
/// deterministic nonce generation.
pub fn sign_p256_sha256(message: &[u8], key: &EcPrivateKey) -> Vec<u8> {
    let n = ecc::order();
    let hash = sha256(message);
    let z = BigUint::from_bytes_be(&hash).rem(&n);

    let mut nonce_gen = DeterministicNonce::new(&key.d, &hash);
    loop {
        let k = nonce_gen.next_candidate();
        let point = ecc::base_point().scalar_mul(&k);
        let Point::Affine { x: x1, .. } = point else {
            continue; // k*G = O only if k ≡ 0 (mod n), already excluded above
        };
        let r = x1.rem(&n);
        if r.is_zero() {
            continue;
        }

        let k_inv = ecc::mod_inverse(&k, &n);
        let r_d = ecc::mod_mul(&r, &key.d, &n);
        let z_plus_rd = ecc::mod_add(&z, &r_d, &n);
        let s = ecc::mod_mul(&k_inv, &z_plus_rd, &n);
        if s.is_zero() {
            continue;
        }

        let mut signature = Vec::with_capacity(64);
        signature.extend(r.to_bytes_be_padded(32).expect("r < n fits in 32 bytes"));
        signature.extend(s.to_bytes_be_padded(32).expect("s < n fits in 32 bytes"));
        return signature;
    }
}

/// Verifies a raw `r || s` ECDSA signature (RFC 7518 §3.4 format) over
/// `message` against `key`.
pub fn verify_p256_sha256(message: &[u8], signature: &[u8], key: &EcPublicKey) -> Result<()> {
    if signature.len() != 64 {
        return Err(Error::Validation(
            "ECDSA signature must be exactly 64 bytes (32-byte r || 32-byte s)".to_string(),
        ));
    }
    let n = ecc::order();
    let r = BigUint::from_bytes_be(&signature[..32]);
    let s = BigUint::from_bytes_be(&signature[32..]);
    if r.is_zero() || r.compare(&n) != Ordering::Less {
        return Err(Error::Validation(
            "ECDSA signature `r` is out of range".to_string(),
        ));
    }
    if s.is_zero() || s.compare(&n) != Ordering::Less {
        return Err(Error::Validation(
            "ECDSA signature `s` is out of range".to_string(),
        ));
    }

    let hash = sha256(message);
    let z = BigUint::from_bytes_be(&hash).rem(&n);

    let w = ecc::mod_inverse(&s, &n);
    let u1 = ecc::mod_mul(&z, &w, &n);
    let u2 = ecc::mod_mul(&r, &w, &n);

    match ecc::double_scalar_mul(&u1, &ecc::base_point(), &u2, &key.point) {
        Point::Infinity => Err(Error::Validation(
            "ECDSA signature verification failed".to_string(),
        )),
        Point::Affine { x, .. } => {
            let v = x.rem(&n);
            if v == r {
                Ok(())
            } else {
                Err(Error::Validation(
                    "ECDSA signature verification failed".to_string(),
                ))
            }
        }
    }
}

/// Encodes and signs a JWT using `ES256`.
pub fn encode_es256(claims: &Value, key: &EcPrivateKey, extra_header: &[(&str, &str)]) -> String {
    let mut header_fields = vec![
        ("alg".to_string(), Value::from("ES256")),
        ("typ".to_string(), Value::from("JWT")),
    ];
    for (k, v) in extra_header {
        header_fields.push((k.to_string(), Value::from(*v)));
    }
    let header = Value::Object(header_fields);

    let header_b64 = encode_url_safe_no_pad(header.to_json().as_bytes());
    let payload_b64 = encode_url_safe_no_pad(claims.to_json().as_bytes());
    let signing_input = format!("{header_b64}.{payload_b64}");

    let signature = sign_p256_sha256(signing_input.as_bytes(), key);
    let signature_b64 = encode_url_safe_no_pad(&signature);

    format!("{signing_input}.{signature_b64}")
}

/// Verifies an `ES256`-signed JWT and returns its claims.
pub fn verify_es256(token: &str, key: &EcPublicKey) -> Result<Value> {
    let decoded = super::decode_unverified(token)?;

    let alg = decoded
        .header
        .get("alg")
        .and_then(Value::as_str)
        .ok_or_else(|| Error::Validation("malformed JWT: header missing `alg`".to_string()))?;
    if alg != "ES256" {
        return Err(Error::Validation(format!("expected alg ES256, got {alg}")));
    }

    verify_p256_sha256(decoded.signing_input.as_bytes(), &decoded.signature, key)?;

    Ok(decoded.claims)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn hex(s: &str) -> Vec<u8> {
        (0..s.len())
            .step_by(2)
            .map(|i| u8::from_str_radix(&s[i..i + 2], 16).unwrap())
            .collect()
    }

    // The same real key pair used in crypto::ecc's tests
    // (`openssl ecparam -genkey`), reused here for the JWT-level tests.
    const D_HEX: &str = "67718fec6a6b21b412a5c5306286f1ee30e32498fd6c61b66f57d0ad1d7c0738";
    const X_HEX: &str = "9958e30d1b1ca2943fb08c191400beab172729085e843cf130422d686bf81a7b";
    const Y_HEX: &str = "a7613a86bac66693dd6adead383e9e1f0407424dc7281049bce06c3fefa91e6f";

    fn private_key() -> EcPrivateKey {
        EcPrivateKey::from_bytes(&hex(D_HEX)).unwrap()
    }

    fn public_key() -> EcPublicKey {
        EcPublicKey::from_affine_coordinates(&hex(X_HEX), &hex(Y_HEX)).unwrap()
    }

    #[test]
    fn private_key_derives_matching_public_key() {
        assert_eq!(private_key().public_key(), public_key());
    }

    #[test]
    fn round_trip_sign_and_verify() {
        let claims = Value::object([
            ("sub".to_string(), Value::from("user-123")),
            ("iss".to_string(), Value::from("https://issuer.example.com")),
        ]);
        let token = encode_es256(&claims, &private_key(), &[]);
        let verified = verify_es256(&token, &public_key()).unwrap();
        assert_eq!(verified.get("sub").unwrap().as_str(), Some("user-123"));
    }

    /// Verifies a token signed entirely outside this crate: `openssl dgst
    /// -sha256 -sign` (which produces a DER signature, converted to the
    /// JWS raw `r || s` format for this test) over the exact same
    /// `openssl ecparam -genkey` key pair used throughout this module's
    /// tests. This is the ES256 analogue of `jwt::rsa`'s
    /// `verifies_real_openssl_signed_token` test -- it validates
    /// `verify_p256_sha256` against an independent implementation, not
    /// just against this crate's own `sign_p256_sha256`.
    #[test]
    fn verifies_real_openssl_signed_token() {
        const TOKEN: &str = "eyJhbGciOiJFUzI1NiIsInR5cCI6IkpXVCJ9.eyJzdWIiOiJ1c2VyLTEyMyIsImlzcyI6Imh0dHBzOi8vaXNzdWVyLmV4YW1wbGUuY29tIiwiZXhwIjo0MTAyNDQ0ODAwfQ.QTCqGlpXHN4fUgZ95-a16krovlzAAnBK4g3M6cihp46whGLIBLhCcCM-e4zQTtCTx4KDmKUrLfm80YIlnmtYfg";
        let claims = verify_es256(TOKEN, &public_key()).unwrap();
        assert_eq!(claims.get("sub").unwrap().as_str(), Some("user-123"));
        assert_eq!(
            claims.get("iss").unwrap().as_str(),
            Some("https://issuer.example.com")
        );
    }

    /// Pins `sign_p256_sha256`'s output for a fixed (key, message) pair to
    /// the exact signature this test suite generated and independently
    /// confirmed with `openssl dgst -sha256 -verify` (`Verified OK`)
    /// outside this crate. RFC 6979 nonce derivation is deterministic, so
    /// this exact value should never change; if it does, something in the
    /// signing path regressed even if `round_trip_sign_and_verify` (which
    /// only checks self-consistency) still passes.
    #[test]
    fn signature_matches_openssl_verified_value() {
        let signing_input = "eyJhbGciOiJFUzI1NiIsInR5cCI6IkpXVCJ9.eyJzdWIiOiJ1c2VyLTEyMyIsImlzcyI6Imh0dHBzOi8vaXNzdWVyLmV4YW1wbGUuY29tIiwiZXhwIjo0MTAyNDQ0ODAwfQ";
        let sig = sign_p256_sha256(signing_input.as_bytes(), &private_key());
        let expected = hex(
            "2389da59b22e48ec443c35d49024ab5d16d788ad02b324c254c1e005324782df\
             8515c8751f0de979a9c2f0b0f27da1558b84d5d5bef86e7fcc44bddb1878e5ff",
        );
        assert_eq!(sig, expected);
    }

    #[test]
    fn signing_is_deterministic() {
        let claims = Value::object([("sub".to_string(), Value::from("user"))]);
        let a = encode_es256(&claims, &private_key(), &[]);
        let b = encode_es256(&claims, &private_key(), &[]);
        assert_eq!(a, b, "RFC 6979 nonce derivation must be deterministic");
    }

    #[test]
    fn verify_rejects_wrong_key() {
        let claims = Value::object([("sub".to_string(), Value::from("user"))]);
        let token = encode_es256(&claims, &private_key(), &[]);
        let other_key = EcPrivateKey::from_bytes(&[0x02; 32]).unwrap().public_key();
        assert!(verify_es256(&token, &other_key).is_err());
    }

    #[test]
    fn verify_rejects_tampered_claims() {
        let claims = Value::object([
            ("sub".to_string(), Value::from("user")),
            ("admin".to_string(), Value::from(false)),
        ]);
        let token = encode_es256(&claims, &private_key(), &[]);
        let mut parts: Vec<&str> = token.split('.').collect();
        let forged = Value::object([
            ("sub".to_string(), Value::from("user")),
            ("admin".to_string(), Value::from(true)),
        ]);
        let forged_payload = encode_url_safe_no_pad(forged.to_json().as_bytes());
        parts[1] = &forged_payload;
        let forged_token = parts.join(".");
        assert!(verify_es256(&forged_token, &public_key()).is_err());
    }

    #[test]
    fn rejects_alg_confusion() {
        let claims = Value::object([("sub".to_string(), Value::from("x"))]);
        let hs256_token = crate::jwt::encode_hs256(&claims, b"whatever", &[]);
        assert!(verify_es256(&hs256_token, &public_key()).is_err());
    }

    #[test]
    fn rejects_wrong_length_signature() {
        let err = verify_p256_sha256(b"message", &[0u8; 10], &public_key()).unwrap_err();
        assert!(matches!(err, Error::Validation(_)));
    }

    #[test]
    fn from_affine_coordinates_rejects_off_curve_point() {
        let mut tampered_y = hex(Y_HEX);
        tampered_y[31] ^= 0x01;
        assert!(EcPublicKey::from_affine_coordinates(&hex(X_HEX), &tampered_y).is_err());
    }
}
