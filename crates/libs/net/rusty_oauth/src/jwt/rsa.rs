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
use rusty_json::Value;
use std::cmp::Ordering;

/// The DER encoding of the `DigestInfo` `AlgorithmIdentifier` for SHA-256,
/// as used in EMSA-PKCS1-v1_5 encoding (RFC 8017 §9.2, Note 1).
const SHA256_DIGEST_INFO_PREFIX: [u8; 19] = [
    0x30, 0x31, 0x30, 0x0d, 0x06, 0x09, 0x60, 0x86, 0x48, 0x01, 0x65, 0x03, 0x04, 0x02, 0x01, 0x05,
    0x00, 0x04, 0x20,
];

/// Smallest modulus verification accepts: RFC 7518 §3.3 requires RS256 keys
/// of 2048 bits or more.
pub const MIN_MODULUS_BITS: usize = 2048;

/// Largest modulus verification accepts. Not an RFC rule: it caps the cost of
/// the modular exponentiation, since the key usually comes from a network
/// document (a `jwks_uri`) and the work grows steeply with its size.
pub const MAX_MODULUS_BITS: usize = 4096;

/// Largest public exponent verification accepts (33 bits).
const MAX_EXPONENT: u64 = (1 << 33) - 1;

/// An RSA public key, as used to verify `RS256`-signed tokens.
///
/// Building a key from raw components never fails, and from base64url fails
/// only if the text is not base64url; neither judges the key. [`verify_pkcs1v15_sha256`] and
/// [`verify_rs256`] refuse one whose modulus is outside
/// [`MIN_MODULUS_BITS`]..=[`MAX_MODULUS_BITS`], has a leading zero byte
/// (RFC 7518 §6.3.1.1), is even, or whose exponent is even, below 3 or over
/// 33 bits.
#[derive(Debug, Clone)]
pub struct RsaPublicKey {
    n: BigUint,
    e: BigUint,
    /// `k`: the modulus size in octets (RFC 8017 §7.2), e.g. 256 for a
    /// 2048-bit key.
    k: usize,
    /// Why this key is refused, decided once when it is built.
    unusable: Option<String>,
}

/// Why a key with these big-endian components cannot verify, if it cannot.
fn unusable_reason(n_bytes: &[u8], e_bytes: &[u8]) -> Option<String> {
    if n_bytes.first() == Some(&0) {
        return Some("RSA modulus has a leading zero byte".to_string());
    }
    let bits = n_bytes
        .first()
        .map_or(0, |top| n_bytes.len() * 8 - top.leading_zeros() as usize);
    if !(MIN_MODULUS_BITS..=MAX_MODULUS_BITS).contains(&bits) {
        return Some(format!(
            "RSA modulus is {bits} bits; {MIN_MODULUS_BITS} to {MAX_MODULUS_BITS} are accepted"
        ));
    }
    // A modulus is a product of two odd primes, so it is odd; an even one
    // (a power of two, say) is not an RSA key and is trivially factored.
    if n_bytes.last().is_some_and(|low| low % 2 == 0) {
        return Some("RSA modulus is even".to_string());
    }
    // Five bytes hold 33 bits; anything longer is out of range already.
    let exponent = if e_bytes.len() <= 5 {
        e_bytes
            .iter()
            .fold(0u64, |acc, byte| (acc << 8) | u64::from(*byte))
    } else {
        u64::MAX
    };
    if exponent < 3 || exponent % 2 == 0 || exponent > MAX_EXPONENT {
        return Some("RSA public exponent is out of range".to_string());
    }
    None
}

impl RsaPublicKey {
    /// Builds a key from raw big-endian modulus/exponent byte strings
    /// (as decoded from a JWK's `n`/`e` members, RFC 7517 §6.3.1).
    pub fn from_components(n_bytes: &[u8], e_bytes: &[u8]) -> Self {
        RsaPublicKey {
            n: BigUint::from_bytes_be(n_bytes),
            e: BigUint::from_bytes_be(e_bytes),
            k: n_bytes.len(),
            unusable: unusable_reason(n_bytes, e_bytes),
        }
    }

    /// Why verification refuses this key, or `None` if it can be used: the
    /// same check [`verify_pkcs1v15_sha256`] makes, available up front so a
    /// caller can reject a bad key before it is ever used. The text is fixed
    /// by this crate and holds only sizes, never key material.
    pub fn unusable_reason(&self) -> Option<&str> {
        self.unusable.as_deref()
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
    if let Some(reason) = key.unusable_reason() {
        return Err(Error::Validation(reason.to_string()));
    }
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
            rusty_json::Value::from_iter([("sub".to_string(), rusty_json::Value::from("x"))]);
        let hs256_token = crate::jwt::encode_hs256(&claims, b"whatever", &[]);
        assert!(verify_rs256(&hs256_token, &key()).is_err());
    }

    #[test]
    fn rejects_wrong_length_signature() {
        let bad_sig = vec![0u8; 10];
        let err = verify_pkcs1v15_sha256(b"message", &bad_sig, &key()).unwrap_err();
        assert!(matches!(err, Error::Validation(_)));
    }

    // A real 4096-bit RSA key generated with `openssl genrsa 4096`, and a JWT
    // signed with `openssl dgst -sha256 -sign`: the largest modulus accepted,
    // verified end to end against a signature made outside this crate.
    const N_4096_B64: &str = "uNPLg2IuAKSTc3zXcF3nH7zpzKUxLUeVx57qFdckzkG8jjCibtPKvkuLkVCFwZlkBpl1JeG_8PkZ8U1EzQIHM1PDFfRwmniMPyiSvzK0Mxj2zEPTjaaed2n_-sOhFOmUjSQezEcLgNXLyo3rjW7IN1j86yQUi6z0myxJOpaZGe5HAYWmkPXNSynmlt9XRXrjfv_kdXuNcSC9oxlcFhrcEf5BJdI9_qwSKyggA7uQwPkzsZrWa4_9O37zXth3X6T-CQ-GdX-kcRfaPwM1QFPqoLpsUeqHP01L5QJfb5VC9ExgY8-h4nHYIUSmQw7KJuGdpvrO2jPKa6rUhiOOLIsPEFNoXv1GT2z957nZw0pxZv7K--iQOPgISvav76Z-xfZr_ld6mYYDL4xzmK2Q9zeYO7vNqpK6DLbcR0-syvJzNRje5a0mdJkb9tls8jkQVKsgrM1YrCQ6KiW0dDVD2G8Mk3M0WUp9Q8Sm7G-f3-YAjPF8QCHxbYGOAnOG7_4bj_yzLtGHILdIXmAK4TwM7kfaqXMo9bcbquYi2Ax4-sYaj1k4btc6elXlD4q3mlJnVQY9u8aWaerCfaXzY4FVSMSxKjE0UssDg1ZP6NoE0gTuW0RwidrdQJb5R3RnkjuCyuOZD5RRwX0p4DnPLYCTWtdYp0ydrftYxOv40YVkG9v8qac";
    const TOKEN_4096: &str = "eyJhbGciOiJSUzI1NiIsInR5cCI6IkpXVCJ9.eyJzdWIiOiJ1c2VyLTQwOTYiLCJpc3MiOiJodHRwczovL2lzc3Vlci5leGFtcGxlLmNvbSIsImV4cCI6NDEwMjQ0NDgwMH0.LhVyxNfjybeES3yMz2RVXubOHl_OfA5cauawy5BkPv-CR6oQEv5g6f6fWKEg6ZsMrI486b4IsgpCqWPZTc-bzDXfp3xekd4RH6AH4OVwRoyrtVqbBUsdCRglgn1RpNCMsIF-roC6yKtH7q0p28qKSlt81nR_0zhT9yHEEq1-0t_ob_ho1wKarHqEagIhduPLWaVCmub3maUbU7m-R8wY5COeS8FnwO19kfAxfl-lmkoHAk47sVUvf2W6t_MEK7a2C0XgmEWsZH7tGOZ0_vlGq_iQN-Bhvx5iX5PmpsjdSJko_fOZeqwjyiWGgzLgpRIJUcBmzfAZLvEaVJzLlPK_wKVkrOKiCy5Z97KTD20dqF9lBvq8AOVcZBd4q-ASGYGaaLXaVz-djB35u2wiy7z76OtOXRopiFV2Se3jvdyyErCDU8xoYS0MyGQJ-ONA8VLxAdWsP6FIroIjlugBnPlvfpf7e565ClXDSgQhegJS62z8bYarx9HOyD1QM7TobAc8zrnnONUc_pVHCe2mI7VTwkaOFD1cBXi8Fti0aPW2NN-YzxnMwiqKdENb0bYQaaYIXZ_-POVapSjRm1gKwINRWf5hIMS7EH0rlKGvt8G3fyQ8XORauAXXRNn0YjbKaghPK4d19bIgQYsHmQrj592DiPd5hSm6yO3KZxcdXO5jZnw";

    #[test]
    fn verifies_real_openssl_signed_4096_bit_token() {
        let key = RsaPublicKey::from_jwk_base64url(N_4096_B64, E_B64).unwrap();
        let claims = verify_rs256(TOKEN_4096, &key).unwrap();
        assert_eq!(claims.get("sub").unwrap().as_str(), Some("user-4096"));
    }

    #[test]
    fn a_4096_bit_key_rejects_a_flipped_signature_bit() {
        let key = RsaPublicKey::from_jwk_base64url(N_4096_B64, E_B64).unwrap();
        let (signing_input, signature) = TOKEN_4096.rsplit_once('.').unwrap();
        let mut bytes = crate::encoding::base64::decode_url_safe(signature).unwrap();
        bytes[0] ^= 1;
        let forged = format!(
            "{signing_input}.{}",
            crate::encoding::base64::encode_url_safe_no_pad(&bytes)
        );
        let why = verify_rs256(&forged, &key)
            .expect_err("flipped bit")
            .to_string();
        assert!(why.contains("verification failed"), "{why}");
    }

    // A modulus of exactly `bits` bits (top bit set, odd), for the bounds
    // tests: the bounds are checked before any arithmetic, so the value
    // itself does not matter.
    fn modulus(bits: usize) -> Vec<u8> {
        let mut n = vec![0xFF; bits.div_ceil(8)];
        n[0] = 1 << ((bits - 1) % 8);
        n
    }

    /// The outcome of verifying a junk signature against a key: `Ok` is
    /// impossible, so the message says whether the key was refused up front
    /// or got as far as the signature check.
    fn refusal(n: &[u8], e: &[u8]) -> String {
        let key = RsaPublicKey::from_components(n, e);
        let signature = vec![0; n.len()];
        verify_pkcs1v15_sha256(b"message", &signature, &key)
            .expect_err("a zero signature never verifies")
            .to_string()
    }

    const REACHED_THE_SIGNATURE_CHECK: &str = "RSA signature verification failed";

    #[test]
    fn moduli_outside_2048_to_4096_bits_are_refused() {
        for bits in [8, 1024, 2047, 4097, 8192] {
            let why = refusal(&modulus(bits), &[1, 0, 1]);
            assert!(why.contains(&format!("{bits} bits")), "{bits}: {why}");
        }
        assert!(refusal(&[], &[1, 0, 1]).contains("0 bits"));
    }

    #[test]
    fn moduli_of_2048_and_4096_bits_reach_the_signature_check() {
        for bits in [2048, 4096] {
            let why = refusal(&modulus(bits), &[1, 0, 1]);
            assert!(why.contains(REACHED_THE_SIGNATURE_CHECK), "{bits}: {why}");
        }
    }

    #[test]
    fn a_modulus_with_a_leading_zero_byte_is_refused() {
        let mut n = modulus(2048);
        n.insert(0, 0);
        assert!(refusal(&n, &[1, 0, 1]).contains("leading zero byte"));
    }

    #[test]
    fn even_moduli_are_refused_including_a_power_of_two() {
        let mut even = modulus(2048);
        *even.last_mut().expect("non-empty") = 0xFE;
        let mut power_of_two = vec![0; 256];
        power_of_two[0] = 0x80; // 2^2047 is exactly 2048 bits and even
        for n in [even, power_of_two] {
            assert!(refusal(&n, &[1, 0, 1]).contains("modulus is even"));
        }
    }

    #[test]
    fn exponents_that_are_even_small_or_over_33_bits_are_refused() {
        let n = modulus(2048);
        let out_of_range: [&[u8]; 9] = [
            &[],
            &[0],
            &[1],
            &[2],
            &[1, 0, 0],                   // 65536, even
            &[2, 0, 0, 0, 1],             // 2^33 + 1, 34 bits
            &[1, 0, 0, 0, 0, 1],          // six bytes
            &[1, 0xFF, 0xFF, 0xFF, 0xFE], // 2^33 - 2, even
            // 2^64 + 3: wraps to 3 if the length is not checked before folding.
            &[1, 0, 0, 0, 0, 0, 0, 0, 3],
        ];
        for e in out_of_range {
            let why = refusal(&n, e);
            assert!(why.contains("exponent is out of range"), "{e:?}: {why}");
        }
    }

    #[test]
    fn odd_exponents_from_3_to_33_bits_reach_the_signature_check() {
        let n = modulus(2048);
        let accepted: [&[u8]; 4] = [&[3], &[1, 0, 1], &[0, 0, 3], &[1, 0xFF, 0xFF, 0xFF, 0xFF]];
        for e in accepted {
            let why = refusal(&n, e);
            assert!(why.contains(REACHED_THE_SIGNATURE_CHECK), "{e:?}: {why}");
        }
    }

    #[test]
    fn unusable_reason_names_why_a_key_is_refused_and_is_none_for_a_usable_one() {
        let usable = RsaPublicKey::from_components(&modulus(2048), &[1, 0, 1]);
        assert_eq!(usable.unusable_reason(), None);
        let small = RsaPublicKey::from_components(&modulus(1024), &[1, 0, 1]);
        let reason = small.unusable_reason().expect("1024 bits is refused");
        assert!(reason.contains("1024 bits"), "{reason}");
        // The same reason is what verification reports.
        assert_eq!(
            refusal(&modulus(1024), &[1, 0, 1]),
            format!("validation error: {reason}")
        );
    }

    #[test]
    fn verify_rs256_refuses_a_small_key_before_checking_the_signature() {
        // The real token, but a key whose modulus is only 1024 bits.
        let key = RsaPublicKey::from_components(&modulus(1024), &[1, 0, 1]);
        let why = verify_rs256(TOKEN, &key)
            .expect_err("too small")
            .to_string();
        assert!(why.contains("1024 bits"), "{why}");
    }
}
