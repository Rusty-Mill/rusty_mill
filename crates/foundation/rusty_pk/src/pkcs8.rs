//! PKCS#8 private keys (RFC 5208 and RFC 5958) for the signing keys in this crate:
//! ECDSA on P-256 or P-384 ([`ecdsa()`]) and Ed25519 ([`ed25519()`]).
//!
//! # Acceptance
//!
//! Follows `ring` 0.17.14's `EcdsaKeyPair::from_pkcs8` and
//! `Ed25519KeyPair::from_pkcs8_maybe_unchecked` (compared by a differential test over
//! generated keys and every single-byte mutation of them, not proven for all inputs):
//!
//! - **ECDSA:** a version 0 (v1) `PrivateKeyInfo` whose algorithm is exactly
//!   `id-ecPublicKey` with the requested curve's named-curve OID. The private key is an
//!   RFC 5915 `ECPrivateKey` of version 1, whose scalar is exactly the curve's length and
//!   in `[1, n-1]`. Its optional `[0]` parameters must name the same curve. Its `[1]`
//!   public key, optional in the RFC, is **required**, and it must equal `d * G` in
//!   uncompressed form.
//! - **Ed25519:** `id-Ed25519` with no parameters, holding a 32-byte seed. A version 0 (v1)
//!   document carries no public key. A version 1 (v2) document must carry one, as a
//!   primitive `[1]` BIT STRING, or as the constructed `[1]` that some old encoders emit.
//!   It must equal the key derived from the seed.
//! - `[0]` attributes are ignored, as in `ring`. Anything else after the expected fields,
//!   and any non-minimal DER, is rejected.
//!
//! # Secrets
//!
//! Parsing branches only on structure: tags, lengths, versions and OIDs. Secret bytes are
//! sliced, never compared, and go straight to the constant-time constructors
//! ([`ecdsa::SigningKey::from_bytes`], [`ed25519::SigningKey::from_seed`]). The derived
//! public key comes from the secret, so its comparison with the encoded one is
//! fixed-work and its verdict goes through `ecdh::ensure`, like every other
//! secret-derived check. The caller owns the input
//! buffer and should wipe it; this module keeps no copy beyond the returned key, apart
//! from a wiped 32-byte seed buffer.

use rusty_crypto_key::wipe;

use crate::der::Reader;
use crate::ecdh::{ensure, nonzero};
use crate::ecdsa::{self, CurveId};
use crate::ed25519;

/// Why a PKCS#8 document was refused. It carries no key material.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum KeyRejected {
    /// Not well-formed DER of the expected shape.
    Malformed,
    /// A `PrivateKeyInfo` version this key type does not accept.
    UnsupportedVersion,
    /// A different algorithm, or a different curve from the one requested.
    WrongAlgorithm,
    /// The private key is out of range or the wrong length.
    InvalidKey,
    /// A required public key is absent, or it does not match the private key.
    PublicKeyMismatch,
}

impl core::fmt::Display for KeyRejected {
    fn fmt(&self, f: &mut core::fmt::Formatter<'_>) -> core::fmt::Result {
        f.write_str(match self {
            KeyRejected::Malformed => "malformed PKCS#8 document",
            KeyRejected::UnsupportedVersion => "unsupported PKCS#8 version",
            KeyRejected::WrongAlgorithm => "PKCS#8 key is for a different algorithm or curve",
            KeyRejected::InvalidKey => "invalid private key in PKCS#8 document",
            KeyRejected::PublicKeyMismatch => "PKCS#8 public key is missing or does not match",
        })
    }
}

const SEQUENCE: u8 = 0x30;
const OCTET_STRING: u8 = 0x04;
const BIT_STRING: u8 = 0x03;
const CONTEXT_0: u8 = 0xa0;
const CONTEXT_1_PRIMITIVE: u8 = 0x81;
const CONTEXT_1_CONSTRUCTED: u8 = 0xa1;

/// `id-ecPublicKey` (1.2.840.10045.2.1), as an encoded OID.
const ID_EC_PUBLIC_KEY: &[u8] = &[0x06, 0x07, 0x2a, 0x86, 0x48, 0xce, 0x3d, 0x02, 0x01];
/// `prime256v1` (1.2.840.10045.3.1.7).
const P256_OID: &[u8] = &[0x06, 0x08, 0x2a, 0x86, 0x48, 0xce, 0x3d, 0x03, 0x01, 0x07];
/// `secp384r1` (1.3.132.0.34).
const P384_OID: &[u8] = &[0x06, 0x05, 0x2b, 0x81, 0x04, 0x00, 0x22];
/// `id-Ed25519` (1.3.101.112).
const ED25519_OID: &[u8] = &[0x06, 0x03, 0x2b, 0x65, 0x70];

/// The `PrivateKeyInfo` fields this module uses.
struct PrivateKeyInfo<'a> {
    version: u8,
    algorithm: &'a [u8],
    private_key: &'a [u8],
    public_key: Option<&'a [u8]>,
}

/// Splits a `PrivateKeyInfo`, skipping `[0]` attributes. A public key is read only from
/// a version 1 (v2) document; a version 0 document with one is malformed.
fn private_key_info(der: &[u8]) -> Result<PrivateKeyInfo<'_>, KeyRejected> {
    let mut outer = Reader::new(der);
    let mut info = Reader::new(outer.tlv(SEQUENCE).ok_or(KeyRejected::Malformed)?);
    if !outer.at_end() {
        return Err(KeyRejected::Malformed);
    }
    let version = info
        .small_version()
        .ok_or(KeyRejected::UnsupportedVersion)?;
    let algorithm = info.tlv(SEQUENCE).ok_or(KeyRejected::Malformed)?;
    let private_key = info.tlv(OCTET_STRING).ok_or(KeyRejected::Malformed)?;
    if info.peek(CONTEXT_0) {
        info.tlv(CONTEXT_0).ok_or(KeyRejected::Malformed)?;
    }
    let public_key = match version {
        0 => None,
        _ => Some(v2_public_key(&mut info)?),
    };
    if !info.at_end() {
        return Err(KeyRejected::Malformed);
    }
    Ok(PrivateKeyInfo {
        version,
        algorithm,
        private_key,
        public_key,
    })
}

/// The v2 `[1]` public key: a primitive BIT STRING, or the constructed legacy form that
/// wraps a universal BIT STRING.
fn v2_public_key<'a>(info: &mut Reader<'a>) -> Result<&'a [u8], KeyRejected> {
    if info.at_end() {
        return Err(KeyRejected::PublicKeyMismatch);
    }
    if !info.peek(CONTEXT_1_CONSTRUCTED) {
        return info
            .bit_string(CONTEXT_1_PRIMITIVE)
            .ok_or(KeyRejected::Malformed);
    }
    let mut legacy = Reader::new(
        info.tlv(CONTEXT_1_CONSTRUCTED)
            .ok_or(KeyRejected::Malformed)?,
    );
    let bits = legacy
        .bit_string(BIT_STRING)
        .ok_or(KeyRejected::Malformed)?;
    if !legacy.at_end() {
        return Err(KeyRejected::Malformed);
    }
    Ok(bits)
}

/// `1` iff the public key derived from the private key equals the encoded one, else `0`.
/// The lengths are public; the bytes are compared with the same work whatever they hold.
fn public_matches(derived: &[u8], encoded: &[u8]) -> u64 {
    if derived.len() != encoded.len() {
        return 0;
    }
    let diff = derived
        .iter()
        .zip(encoded)
        .fold(0u8, |acc, (a, b)| acc | (a ^ b));
    1 ^ nonzero(u64::from(diff))
}

/// An ECDSA key on `curve` from a PKCS#8 v1 document (see the module docs for what is
/// accepted). The curve must match the document's.
pub fn ecdsa(curve: CurveId, der: &[u8]) -> Result<ecdsa::SigningKey, KeyRejected> {
    let curve_oid = match curve {
        CurveId::P256 => P256_OID,
        CurveId::P384 => P384_OID,
    };
    let info = private_key_info(der)?;
    let mut expected = [0u8; 9 + 10];
    let algorithm_len = ID_EC_PUBLIC_KEY.len() + curve_oid.len();
    expected[..ID_EC_PUBLIC_KEY.len()].copy_from_slice(ID_EC_PUBLIC_KEY);
    expected[ID_EC_PUBLIC_KEY.len()..algorithm_len].copy_from_slice(curve_oid);
    if info.algorithm != &expected[..algorithm_len] {
        return Err(KeyRejected::WrongAlgorithm);
    }
    if info.version != 0 {
        return Err(KeyRejected::UnsupportedVersion);
    }
    let (scalar, public) = ec_private_key(info.private_key, curve_oid)?;
    let key = ecdsa::SigningKey::from_bytes(curve, scalar).map_err(|_| KeyRejected::InvalidKey)?;
    let derived = key.public_key().map_err(|_| KeyRejected::InvalidKey)?;
    ensure(public_matches(derived.as_bytes(), public))
        .map_err(|_| KeyRejected::PublicKeyMismatch)?;
    Ok(key)
}

/// The scalar and the required public key of an RFC 5915 `ECPrivateKey`.
fn ec_private_key<'a>(
    der: &'a [u8],
    curve_oid: &[u8],
) -> Result<(&'a [u8], &'a [u8]), KeyRejected> {
    let mut outer = Reader::new(der);
    let mut key = Reader::new(outer.tlv(SEQUENCE).ok_or(KeyRejected::Malformed)?);
    if !outer.at_end() || key.small_version() != Some(1) {
        return Err(KeyRejected::Malformed);
    }
    let scalar = key.tlv(OCTET_STRING).ok_or(KeyRejected::Malformed)?;
    if key.peek(CONTEXT_0) && key.tlv(CONTEXT_0) != Some(curve_oid) {
        return Err(KeyRejected::WrongAlgorithm);
    }
    if !key.peek(CONTEXT_1_CONSTRUCTED) {
        return Err(KeyRejected::PublicKeyMismatch);
    }
    let mut wrapped = Reader::new(
        key.tlv(CONTEXT_1_CONSTRUCTED)
            .ok_or(KeyRejected::Malformed)?,
    );
    let public = wrapped
        .bit_string(BIT_STRING)
        .ok_or(KeyRejected::Malformed)?;
    if !wrapped.at_end() || !key.at_end() {
        return Err(KeyRejected::Malformed);
    }
    Ok((scalar, public))
}

/// An Ed25519 key from a PKCS#8 v1 or v2 document (see the module docs).
pub fn ed25519(der: &[u8]) -> Result<ed25519::SigningKey, KeyRejected> {
    let info = private_key_info(der)?;
    if info.algorithm != ED25519_OID {
        return Err(KeyRejected::WrongAlgorithm);
    }
    let mut outer = Reader::new(info.private_key);
    let seed_bytes = outer.tlv(OCTET_STRING).ok_or(KeyRejected::Malformed)?;
    if !outer.at_end() {
        return Err(KeyRejected::Malformed);
    }
    let mut seed = [0u8; 32];
    if seed_bytes.len() != seed.len() {
        return Err(KeyRejected::InvalidKey);
    }
    seed.copy_from_slice(seed_bytes);
    let key = ed25519::SigningKey::from_seed(&seed).map_err(|_| KeyRejected::InvalidKey);
    wipe(&mut seed);
    let key = key?;
    if let Some(public) = info.public_key {
        ensure(public_matches(key.public_key(), public))
            .map_err(|_| KeyRejected::PublicKeyMismatch)?;
    }
    Ok(key)
}

#[cfg(test)]
mod tests {
    use super::*;

    /// RFC 8410 section 10.3: a v1 Ed25519 key with no public key.
    const RFC8410_V1: [u8; 48] = [
        0x30, 0x2e, 0x02, 0x01, 0x00, 0x30, 0x05, 0x06, 0x03, 0x2b, 0x65, 0x70, 0x04, 0x22, 0x04,
        0x20, 0xd4, 0xee, 0x72, 0xdb, 0xf9, 0x13, 0x58, 0x4a, 0xd5, 0xb6, 0xd8, 0xf1, 0xf7, 0x69,
        0xf8, 0xad, 0x3a, 0xfe, 0x7c, 0x28, 0xcb, 0xf1, 0xd4, 0xfb, 0xe0, 0x97, 0xa8, 0x8f, 0x44,
        0x75, 0x58, 0x42,
    ];

    #[test]
    fn rfc_8410_v1_ed25519_key_parses() {
        let key = ed25519(&RFC8410_V1).unwrap();
        let mut seed = [0u8; 32];
        seed.copy_from_slice(&RFC8410_V1[16..]);
        assert_eq!(
            key.public_key(),
            ed25519::SigningKey::from_seed(&seed).unwrap().public_key()
        );
    }

    #[test]
    fn public_matches_needs_equal_length_and_bytes() {
        assert_eq!(public_matches(&[1, 2, 3], &[1, 2, 3]), 1);
        assert_eq!(public_matches(&[1, 2, 3], &[1, 2, 2]), 0);
        assert_eq!(public_matches(&[1, 2, 3], &[1, 2]), 0);
        assert_eq!(public_matches(&[], &[]), 1);
    }

    #[test]
    fn trailing_bytes_and_wrong_algorithm_are_refused() {
        let mut long = RFC8410_V1.to_vec();
        long.push(0);
        assert_eq!(ed25519(&long).unwrap_err(), KeyRejected::Malformed);
        assert_eq!(
            ecdsa(CurveId::P256, &RFC8410_V1).unwrap_err(),
            KeyRejected::WrongAlgorithm
        );
    }
}
