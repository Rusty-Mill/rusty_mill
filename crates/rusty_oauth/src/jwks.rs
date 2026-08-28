//! JSON Web Key Set parsing (RFC 7517), with `kid`-based key selection.
//!
//! Verifying a JWT signed by a real authorization server almost always
//! means fetching its `jwks_uri` document (surfaced in
//! [`crate::metadata::AuthorizationServerMetadata::jwks_uri`]) and picking
//! out the one key whose `kid` matches the JWT header's `kid` -- signing
//! keys rotate, so a JWKS document commonly holds several. This module is
//! that missing piece: [`jwt::rsa::RsaPublicKey`](crate::jwt::rsa::RsaPublicKey)
//! alone only knows how to use a single already-selected key.

use crate::error::{Error, Result};
use crate::json::{self, Value};
use crate::jwt::rsa::RsaPublicKey;

/// A single JSON Web Key (RFC 7517 §4). Only the fields needed to select
/// and use a key are parsed into typed accessors; the full JSON object is
/// retained in `raw` for anything else (`x5c`, `x5t`, etc).
#[derive(Debug, Clone)]
pub struct Jwk {
    /// `kty` (RFC 7517 §4.1): the key type, e.g. `"RSA"` or `"EC"`.
    pub kty: String,
    /// `kid` (RFC 7517 §4.5): the key ID used to match a JWT header's
    /// `kid` to the right key in a set.
    pub kid: Option<String>,
    /// `use` (RFC 7517 §4.2): `"sig"` or `"enc"`.
    pub use_: Option<String>,
    /// `alg` (RFC 7517 §4.4): the JWA algorithm this key is intended for.
    pub alg: Option<String>,
    /// The complete parsed JSON object for this key.
    pub raw: Value,
}

impl Jwk {
    fn from_value(value: Value) -> Result<Self> {
        let kty = value
            .get("kty")
            .and_then(Value::as_str)
            .ok_or_else(|| Error::Protocol("JWK missing `kty`".to_string()))?
            .to_string();
        let kid = value.get("kid").and_then(Value::as_str).map(str::to_string);
        let use_ = value.get("use").and_then(Value::as_str).map(str::to_string);
        let alg = value.get("alg").and_then(Value::as_str).map(str::to_string);
        Ok(Jwk {
            kty,
            kid,
            use_,
            alg,
            raw: value,
        })
    }

    /// Converts this key to an [`RsaPublicKey`], if it's an RSA key
    /// (`kty: "RSA"`) with the required `n`/`e` members (RFC 7518 §6.3.1).
    pub fn to_rsa_public_key(&self) -> Result<RsaPublicKey> {
        if self.kty != "RSA" {
            return Err(Error::Validation(format!(
                "JWK kty `{}` is not RSA",
                self.kty
            )));
        }
        let n = self
            .raw
            .get("n")
            .and_then(Value::as_str)
            .ok_or_else(|| Error::Protocol("RSA JWK missing `n`".to_string()))?;
        let e = self
            .raw
            .get("e")
            .and_then(Value::as_str)
            .ok_or_else(|| Error::Protocol("RSA JWK missing `e`".to_string()))?;
        RsaPublicKey::from_jwk_base64url(n, e)
    }

    /// Converts this key to an [`EcPublicKey`](crate::jwt::es256::EcPublicKey),
    /// if it's a P-256 EC key (`kty: "EC"`, `crv: "P-256"`) with the
    /// required `x`/`y` members (RFC 7518 §6.2.1). Other curves (e.g.
    /// `P-384`, `P-521`) aren't supported -- this crate's elliptic-curve
    /// arithmetic is specialized to P-256, the curve `ES256` uses.
    pub fn to_ec_public_key(&self) -> Result<crate::jwt::es256::EcPublicKey> {
        let components = self.ec_components()?;
        if components.crv != "P-256" {
            return Err(Error::Validation(format!(
                "JWK crv `{}` is not P-256 (the only curve this crate implements)",
                components.crv
            )));
        }
        let x = crate::encoding::base64::decode_url_safe(&components.x)?;
        let y = crate::encoding::base64::decode_url_safe(&components.y)?;
        crate::jwt::es256::EcPublicKey::from_affine_coordinates(&x, &y)
    }

    /// The EC curve/coordinate components (`crv`, `x`, `y`, RFC 7518
    /// §6.2.1) as raw base64url strings, if this is an EC key (`kty:
    /// "EC"`). Lower-level than [`to_ec_public_key`](Self::to_ec_public_key);
    /// mainly useful for curves that method doesn't support.
    pub fn ec_components(&self) -> Result<EcComponents> {
        if self.kty != "EC" {
            return Err(Error::Validation(format!(
                "JWK kty `{}` is not EC",
                self.kty
            )));
        }
        let field = |key: &str| -> Result<String> {
            self.raw
                .get(key)
                .and_then(Value::as_str)
                .map(str::to_string)
                .ok_or_else(|| Error::Protocol(format!("EC JWK missing `{key}`")))
        };
        Ok(EcComponents {
            crv: field("crv")?,
            x: field("x")?,
            y: field("y")?,
        })
    }
}

/// The raw base64url-encoded curve/coordinate components of an EC JWK.
/// See [`Jwk::ec_components`].
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct EcComponents {
    pub crv: String,
    pub x: String,
    pub y: String,
}

/// A JSON Web Key Set (RFC 7517 §5): the document served at an
/// authorization server's `jwks_uri`.
#[derive(Debug, Clone)]
pub struct JwkSet {
    pub keys: Vec<Jwk>,
}

impl JwkSet {
    /// Parses a JWKS document body (`{"keys": [...]}`).
    pub fn parse(body: &str) -> Result<Self> {
        let value = json::parse(body)?;
        let keys_value = value
            .get("keys")
            .and_then(Value::as_array)
            .ok_or_else(|| Error::Protocol("JWK Set missing `keys` array".to_string()))?;
        let keys = keys_value
            .iter()
            .cloned()
            .map(Jwk::from_value)
            .collect::<Result<Vec<_>>>()?;
        Ok(JwkSet { keys })
    }

    /// Finds the key with the given `kid` (RFC 7517 §4.5). Returns `None`
    /// if no key has that `kid` -- or if more than one does, since an
    /// authorization server's JWKS should never have two keys sharing a
    /// `kid` at the same time, and silently picking one of them would be
    /// the wrong failure mode for a security-relevant key lookup.
    pub fn find(&self, kid: &str) -> Option<&Jwk> {
        let mut matches = self.keys.iter().filter(|k| k.kid.as_deref() == Some(kid));
        let first = matches.next()?;
        if matches.next().is_some() {
            return None;
        }
        Some(first)
    }

    /// Finds a key by `kid` and converts it to an [`RsaPublicKey`] in one
    /// step -- the common case for verifying an `RS256` JWT against a
    /// `jwks_uri` document.
    pub fn rsa_key(&self, kid: &str) -> Result<RsaPublicKey> {
        self.find(kid)
            .ok_or_else(|| Error::Validation(format!("no JWK found with kid `{kid}`")))?
            .to_rsa_public_key()
    }

    /// Finds a key by `kid` and converts it to an
    /// [`EcPublicKey`](crate::jwt::es256::EcPublicKey) in one step -- the
    /// common case for verifying an `ES256` JWT against a `jwks_uri`
    /// document.
    pub fn ec_key(&self, kid: &str) -> Result<crate::jwt::es256::EcPublicKey> {
        self.find(kid)
            .ok_or_else(|| Error::Validation(format!("no JWK found with kid `{kid}`")))?
            .to_ec_public_key()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    // The same RSA key used in `jwt::rsa`'s tests (generated with `openssl
    // genrsa` / cross-checked against an `openssl dgst -sign` signature
    // there) -- reused here so this is a real key, not made-up numbers.
    const N_B64: &str = "oQ5vxaCnk7fBF8_wMi-RV26_9Bri9J7I6T76PBq-eQ_oD7xtca3DD7WyhB846mGRURRiQj5G8ORWT_UDSKvJIc0EsoXjDmac3JUm6fQiLnm1107lw4rIavf4isUZVi18SfVAO8ZiWSioLOf2Bh4t-d0wCK92evedt7QvrivcO2GvurwP2jmyh_Ev2xqBIKn8oC8iKm2FBYhyu_LYMHzbqEXWOz4l3uxYUnpXZXVnP5u0IQPET2Hskxj10YpV-KrZ2iZNo6A5QZxxFYLXY4FOOS91onur89z_tTyxEJzfYsIIzyU_qlxs1-Or_erZIKeHo7YHkpEWAg2o-nekcb-7Zw";
    const E_B64: &str = "AQAB";

    fn sample_jwks() -> String {
        format!(
            r#"{{"keys": [
                {{"kty": "RSA", "kid": "key-1", "use": "sig", "alg": "RS256", "n": "{N_B64}", "e": "{E_B64}"}},
                {{"kty": "EC", "kid": "key-2", "use": "sig", "alg": "ES256", "crv": "P-256", "x": "mVjjDRscopQ_sIwZFAC-qxcnKQhehDzxMEItaGv4Gns", "y": "p2E6hrrGZpPdat6tOD6eHwQHQk3HKBBJvOBsP--pHm8"}},
                {{"kty": "oct", "kid": "key-3", "k": "GawgguFyGrWKav7AX4VKUg"}}
            ]}}"#
        )
    }

    #[test]
    fn parses_multi_key_set() {
        let set = JwkSet::parse(&sample_jwks()).unwrap();
        assert_eq!(set.keys.len(), 3);
        assert_eq!(set.keys[0].kty, "RSA");
        assert_eq!(set.keys[1].kty, "EC");
        assert_eq!(set.keys[2].kty, "oct");
    }

    #[test]
    fn finds_key_by_kid() {
        let set = JwkSet::parse(&sample_jwks()).unwrap();
        let key = set.find("key-2").unwrap();
        assert_eq!(key.kty, "EC");
        assert_eq!(key.alg.as_deref(), Some("ES256"));
    }

    #[test]
    fn missing_kid_returns_none() {
        let set = JwkSet::parse(&sample_jwks()).unwrap();
        assert!(set.find("nonexistent").is_none());
    }

    #[test]
    fn duplicate_kid_is_treated_as_not_found() {
        let json = format!(
            r#"{{"keys": [
                {{"kty": "RSA", "kid": "dup", "n": "{N_B64}", "e": "{E_B64}"}},
                {{"kty": "RSA", "kid": "dup", "n": "{N_B64}", "e": "{E_B64}"}}
            ]}}"#
        );
        let set = JwkSet::parse(&json).unwrap();
        assert!(set.find("dup").is_none());
    }

    #[test]
    fn rsa_key_converts_selected_key() {
        let set = JwkSet::parse(&sample_jwks()).unwrap();
        let key = set.rsa_key("key-1").unwrap();
        // Cross-check: verifies the same real-world signature used in
        // `jwt::rsa`'s own test, but reached via kid lookup this time.
        let token = "eyJhbGciOiJSUzI1NiIsInR5cCI6IkpXVCJ9.eyJzdWIiOiJ1c2VyLTEyMyIsImlzcyI6Imh0dHBzOi8vaXNzdWVyLmV4YW1wbGUuY29tIiwiZXhwIjo0MTAyNDQ0ODAwfQ.JZKl0Sc1FmSap7qg8hxyREr6FDIcFrqlaUrLdMsBVWc_V6q5AIS4G5V7hiAXumX_tHbO5jWudnLHFuUK-nZ1XpTFZm656cHAmU_tdk5kIajtBu56OX8GGtjiOubXsC4xoK0nM-P7IfAagjp2F8CL_vt724ZnjbZd-d8MAXK6JgU-BoRt6vJT2DvW6iGqlJTdiFfCIBuCZnhaMfWZ5R6sGC3d5l1PZOkVWSjmBx1oNkLqcZUwMeY3Ww4OgqIvk_DpWqsYzpGGYU90_X_hAl63qnzBPnmGuQili_VT81ws9OZCCGdYMU6m_UA-ltSsLim1NwyQT4pVg4Ziad6E7gdSNA";
        let claims = crate::jwt::rsa::verify_rs256(token, &key).unwrap();
        assert_eq!(claims.get("sub").unwrap().as_str(), Some("user-123"));
    }

    #[test]
    fn ec_key_exposes_raw_components() {
        let set = JwkSet::parse(&sample_jwks()).unwrap();
        let key = set.find("key-2").unwrap();
        let components = key.ec_components().unwrap();
        assert_eq!(components.crv, "P-256");
        assert!(key.to_rsa_public_key().is_err());
    }

    #[test]
    fn ec_key_converts_selected_key() {
        // Same real P-256 key pair used throughout crypto::ecc's and
        // jwt::es256's tests (openssl ecparam -genkey), reached here via
        // kid lookup instead of constructing EcPublicKey directly.
        let set = JwkSet::parse(&sample_jwks()).unwrap();
        let key = set.ec_key("key-2").unwrap();
        let expected = crate::jwt::es256::EcPublicKey::from_jwk_base64url(
            "mVjjDRscopQ_sIwZFAC-qxcnKQhehDzxMEItaGv4Gns",
            "p2E6hrrGZpPdat6tOD6eHwQHQk3HKBBJvOBsP--pHm8",
        )
        .unwrap();
        assert_eq!(key, expected);
    }

    #[test]
    fn ec_key_rejects_non_p256_curve() {
        let json = r#"{"keys": [
            {"kty": "EC", "kid": "p384-key", "crv": "P-384", "x": "AAAA", "y": "AAAA"}
        ]}"#;
        let set = JwkSet::parse(json).unwrap();
        assert!(set.ec_key("p384-key").is_err());
    }

    #[test]
    fn oct_key_rejects_both_conversions() {
        let set = JwkSet::parse(&sample_jwks()).unwrap();
        let key = set.find("key-3").unwrap();
        assert!(key.to_rsa_public_key().is_err());
        assert!(key.ec_components().is_err());
    }

    #[test]
    fn missing_keys_array_errors() {
        assert!(JwkSet::parse(r#"{"not_keys": []}"#).is_err());
    }
}
