//! `RS256` (RSASSA-PKCS1-v1_5 using SHA-256, RFC 7518 §3.3 / RFC 8017 §8.2)
//! JWT signature verification, backed by the hand-rolled big-integer
//! arithmetic in [`crate::crypto::bigint`].
//!
//! This module only *verifies* signatures -- it never signs. RSA key
//! *generation* and private-key operations are out of scope: they need
//! constant-time modular exponentiation and secure prime generation to be
//! safe, which is a much larger undertaking than a general OAuth crate
//! should take on. Verification with a public exponent (invariably small,
//! e.g. 65537) has no such constant-time requirement, which is why it's
//! implemented here.

use crate::crypto::bigint::BigUint;
use crate::crypto::sha256::sha256;
use crate::encoding::base64::decode_url_safe;
use crate::error::{Error, Result};
use crate::json::Value;
use std::cmp::Ordering;

/// The DER encoding of the `DigestInfo` `AlgorithmIdentifier` for SHA-256,
/// as used in EMSA-PKCS1-v1_5 encoding (RFC 8017 §9.2, Note 1).
const SHA256_DIGEST_INFO_PREFIX: [u8; 19] = [
    0x30, 0x31, 0x30, 0x0d, 0x06, 0x09, 0x60, 0x86, 0x48, 0x01, 0x65, 0x03, 0x04, 0x02, 0x01, 0x05,
    0x00, 0x04, 0x20,
];

/// An RSA public key, as used to verify `RS256`-signed tokens.
#[derive(Debug, Clone)]
pub struct RsaPublicKey {
    n: BigUint,
    e: BigUint,
    /// `k`: the modulus size in octets (RFC 8017 §7.2), e.g. 256 for a
    /// 2048-bit key.
    k: usize,
}

impl RsaPublicKey {
    /// Builds a key from raw big-endian modulus/exponent byte strings
    /// (as decoded from a JWK's `n`/`e` members, RFC 7517 §6.3.1).
    pub fn from_components(n_bytes: &[u8], e_bytes: &[u8]) -> Self {
        RsaPublicKey {
            n: BigUint::from_bytes_be(n_bytes),
            e: BigUint::from_bytes_be(e_bytes),
            k: n_bytes.len(),
        }
    }

    /// Builds a key directly from a JWK's base64url-encoded `n` and `e`
    /// members.
    pub fn from_jwk_base64url(n_b64: &str, e_b64: &str) -> Result<Self> {
        let n_bytes = decode_url_safe(n_b64)?;
        let e_bytes = decode_url_safe(e_b64)?;
        Ok(Self::from_components(&n_bytes, &e_bytes))
    }
}

/// Verifies a PKCS#1 v1.5 SHA-256 signature over `message` (RFC 8017
/// §8.2.2, `RSASSA-PKCS1-V1_5-VERIFY`).
pub fn verify_pkcs1v15_sha256(message: &[u8], signature: &[u8], key: &RsaPublicKey) -> Result<()> {
    if signature.len() != key.k {
        return Err(Error::Validation(
            "RSA signature length does not match the key's modulus size".to_string(),
        ));
    }

    let s = BigUint::from_bytes_be(signature);
    if s.compare(&key.n) != Ordering::Less {
        return Err(Error::Validation(
            "RSA signature integer is not smaller than the modulus".to_string(),
        ));
    }

    let m = s.modpow(&key.e, &key.n);
    let em = m.to_bytes_be_padded(key.k).ok_or_else(|| {
        Error::Validation("RSA verification produced an oversized result".to_string())
    })?;

    let hash = sha256(message);
    let mut t = Vec::with_capacity(SHA256_DIGEST_INFO_PREFIX.len() + hash.len());
    t.extend_from_slice(&SHA256_DIGEST_INFO_PREFIX);
    t.extend_from_slice(&hash);

    if key.k < t.len() + 11 {
        return Err(Error::Validation(
            "RSA key is too small for RS256 signatures".to_string(),
        ));
    }
    let ps_len = key.k - 3 - t.len();

    let mut expected = Vec::with_capacity(key.k);
    expected.push(0x00);
    expected.push(0x01);
    expected.extend(std::iter::repeat_n(0xFFu8, ps_len));
    expected.push(0x00);
    expected.extend_from_slice(&t);

    if !crate::crypto::hmac::constant_time_eq(&em, &expected) {
        return Err(Error::Validation(
            "RSA signature verification failed".to_string(),
        ));
    }

    Ok(())
}

/// Verifies an `RS256`-signed JWT against `key` and returns its claims.
/// Rejects any token whose header does not declare `alg: "RS256"` --
/// callers must already know which algorithm/key they expect (never
/// trust the token to tell you, which is how "alg confusion" forgeries
/// work).
pub fn verify_rs256(token: &str, key: &RsaPublicKey) -> Result<Value> {
    let decoded = super::decode_unverified(token)?;

    let alg = decoded
        .header
        .get("alg")
        .and_then(Value::as_str)
        .ok_or_else(|| Error::Validation("malformed JWT: header missing `alg`".to_string()))?;
    if alg != "RS256" {
        return Err(Error::Validation(format!("expected alg RS256, got {alg}")));
    }

    verify_pkcs1v15_sha256(decoded.signing_input.as_bytes(), &decoded.signature, key)?;

    Ok(decoded.claims)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::jwt::{validate_claims, Validation};

    // Test vector: a real 2048-bit RSA key generated with `openssl genrsa`,
    // and a JWT signed with `openssl dgst -sha256 -sign` (PKCS#1 v1.5) --
    // i.e. produced entirely outside this crate, to cross-check the
    // hand-rolled bignum/RSA implementation against a trusted reference.
    const N_B64: &str = "oQ5vxaCnk7fBF8_wMi-RV26_9Bri9J7I6T76PBq-eQ_oD7xtca3DD7WyhB846mGRURRiQj5G8ORWT_UDSKvJIc0EsoXjDmac3JUm6fQiLnm1107lw4rIavf4isUZVi18SfVAO8ZiWSioLOf2Bh4t-d0wCK92evedt7QvrivcO2GvurwP2jmyh_Ev2xqBIKn8oC8iKm2FBYhyu_LYMHzbqEXWOz4l3uxYUnpXZXVnP5u0IQPET2Hskxj10YpV-KrZ2iZNo6A5QZxxFYLXY4FOOS91onur89z_tTyxEJzfYsIIzyU_qlxs1-Or_erZIKeHo7YHkpEWAg2o-nekcb-7Zw";
    const E_B64: &str = "AQAB";
    const TOKEN: &str = "eyJhbGciOiJSUzI1NiIsInR5cCI6IkpXVCJ9.eyJzdWIiOiJ1c2VyLTEyMyIsImlzcyI6Imh0dHBzOi8vaXNzdWVyLmV4YW1wbGUuY29tIiwiZXhwIjo0MTAyNDQ0ODAwfQ.JZKl0Sc1FmSap7qg8hxyREr6FDIcFrqlaUrLdMsBVWc_V6q5AIS4G5V7hiAXumX_tHbO5jWudnLHFuUK-nZ1XpTFZm656cHAmU_tdk5kIajtBu56OX8GGtjiOubXsC4xoK0nM-P7IfAagjp2F8CL_vt724ZnjbZd-d8MAXK6JgU-BoRt6vJT2DvW6iGqlJTdiFfCIBuCZnhaMfWZ5R6sGC3d5l1PZOkVWSjmBx1oNkLqcZUwMeY3Ww4OgqIvk_DpWqsYzpGGYU90_X_hAl63qnzBPnmGuQili_VT81ws9OZCCGdYMU6m_UA-ltSsLim1NwyQT4pVg4Ziad6E7gdSNA";

    fn key() -> RsaPublicKey {
        RsaPublicKey::from_jwk_base64url(N_B64, E_B64).unwrap()
    }

    #[test]
    fn verifies_real_openssl_signed_token() {
        let claims = verify_rs256(TOKEN, &key()).unwrap();
        assert_eq!(claims.get("sub").unwrap().as_str(), Some("user-123"));

        let opts = Validation {
            expected_issuer: Some("https://issuer.example.com".to_string()),
            ..Default::default()
        };
        assert!(validate_claims(&claims, &opts).is_ok());
    }

    #[test]
    fn rejects_tampered_claims() {
        let mut parts: Vec<&str> = TOKEN.split('.').collect();
        // A claims segment that decodes to different (but still valid) JSON.
        let forged = crate::encoding::base64::encode_url_safe_no_pad(br#"{"sub":"attacker"}"#);
        parts[1] = &forged;
        let forged_token = parts.join(".");
        assert!(verify_rs256(&forged_token, &key()).is_err());
    }

    #[test]
    fn rejects_wrong_key() {
        // A different, unrelated 2048-bit modulus/exponent (structurally
        // valid, just not the key that signed TOKEN).
        let other_key = RsaPublicKey::from_components(&[0x01; 256], &[0x01, 0x00, 0x01]);
        assert!(verify_rs256(TOKEN, &other_key).is_err());
    }

    #[test]
    fn rejects_alg_confusion() {
        let claims =
            crate::json::Value::object([("sub".to_string(), crate::json::Value::from("x"))]);
        let hs256_token = crate::jwt::encode_hs256(&claims, b"whatever", &[]);
        assert!(verify_rs256(&hs256_token, &key()).is_err());
    }

    #[test]
    fn rejects_wrong_length_signature() {
        let bad_sig = vec![0u8; 10];
        let err = verify_pkcs1v15_sha256(b"message", &bad_sig, &key()).unwrap_err();
        assert!(matches!(err, Error::Validation(_)));
    }
}
