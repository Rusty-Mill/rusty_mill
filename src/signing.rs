//! Agent Card signing and verification (spec Section 8.4): a JSON Web
//! Signature (RFC 7515) over the card's JSON Canonicalization Scheme
//! (JCS, RFC 8785) representation, with the `signatures` field itself
//! excluded from what's signed.
//!
//! Two algorithms are supported: `ES256` (ECDSA P-256 + SHA-256, the
//! spec's own example) and `EdDSA` (Ed25519, RFC 8037). RSA is not
//! supported.
//!
//! ```
//! use rusty_a2a::signing::{AgentCardSigningExt, SigningKey};
//! use rusty_a2a::types::{AgentCard, AgentInterface};
//!
//! let key = SigningKey::generate_ed25519().unwrap();
//! let card = AgentCard::new("Agent", "desc", "1.0.0", AgentInterface::json_rpc("https://a"));
//! let signed = card.signed(&key, Some("key-1")).unwrap();
//!
//! let verifying_key = key.verifying_key();
//! signed
//!     .verify_signature(&signed.signatures[0], &verifying_key)
//!     .expect("signature verifies");
//! ```
//!
//! # Canonicalization scope
//!
//! [`canonical_json`] relies on `serde_json::Value`'s object type (a
//! `BTreeMap` in this crate's default configuration) to get RFC 8785's
//! required lexicographic key ordering "for free", and on
//! `serde_json::to_string`'s output already having no insignificant
//! whitespace. It does **not** implement JCS's specific number-formatting
//! rules for floating-point values, since [`crate::types::AgentCard`] has
//! no float-typed fields - every value on it is a string, bool, integer,
//! array, or object, for which plain compact JSON serialization already
//! coincides with JCS.
//!
//! # Key material
//!
//! [`SigningKey`] and [`VerifyingKey`] only support constructing keys from
//! raw bytes ([`SigningKey::from_es256_bytes`],
//! [`SigningKey::from_ed25519_bytes`], and their `Verifying` counterparts)
//! or generating a fresh random key
//! ([`SigningKey::generate_es256`]/[`SigningKey::generate_ed25519`]).
//! There's no PEM/PKCS8 support - if your key material comes from a file,
//! KMS, or HSM in one of those formats, decode it with a crate suited to
//! that (e.g. `pkcs8`) and hand this module the raw scalar/seed bytes.

use base64::engine::general_purpose::URL_SAFE_NO_PAD;
use base64::Engine as _;
use serde_json::{json, Map, Value};
use signature::{Signer, Verifier};

use crate::types::{AgentCard, AgentCardSignature};

/// Errors from signing or verifying an [`AgentCard`].
#[derive(Debug, thiserror::Error)]
pub enum SigningError {
    #[error("failed to serialize agent card: {0}")]
    Json(#[from] serde_json::Error),
    #[error("signature or protected header is not valid base64url: {0}")]
    InvalidBase64(#[from] base64::DecodeError),
    #[error("protected header is not a valid JSON object")]
    InvalidProtectedHeader,
    #[error("protected header's \"alg\" does not match the verifying key's algorithm")]
    AlgorithmMismatch,
    #[error("key material is the wrong length or otherwise invalid: {0}")]
    InvalidKey(String),
    #[error("signature does not match the given key")]
    VerificationFailed,
    #[error("failed to generate a new key: {0}")]
    KeyGeneration(String),
}

pub type Result<T> = std::result::Result<T, SigningError>;

/// A JWS signature algorithm supported by this module. The JWA-registered
/// name (used in the JWS protected header's `"alg"`) is available via
/// [`Algorithm::jwa_name`].
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Algorithm {
    /// ECDSA using the P-256 curve and SHA-256 (spec Section 8.4.2's
    /// example algorithm).
    Es256,
    /// EdDSA using Ed25519 (RFC 8037).
    EdDsa,
}

impl Algorithm {
    pub fn jwa_name(self) -> &'static str {
        match self {
            Algorithm::Es256 => "ES256",
            Algorithm::EdDsa => "EdDSA",
        }
    }
}

/// A private key for signing an [`AgentCard`].
pub enum SigningKey {
    Es256(p256::ecdsa::SigningKey),
    Ed25519(ed25519_dalek::SigningKey),
}

impl SigningKey {
    /// Builds an ES256 signing key from a 32-byte P-256 private scalar.
    pub fn from_es256_bytes(bytes: &[u8]) -> Result<Self> {
        p256::ecdsa::SigningKey::from_slice(bytes)
            .map(SigningKey::Es256)
            .map_err(|e| SigningError::InvalidKey(e.to_string()))
    }

    /// Builds an EdDSA (Ed25519) signing key from a 32-byte seed.
    pub fn from_ed25519_bytes(bytes: &[u8; 32]) -> Self {
        SigningKey::Ed25519(ed25519_dalek::SigningKey::from_bytes(bytes))
    }

    /// Generates a fresh random ES256 key using the OS random number
    /// generator.
    pub fn generate_es256() -> Result<Self> {
        Self::from_es256_bytes(&random_bytes::<32>()?)
    }

    /// Generates a fresh random EdDSA (Ed25519) key using the OS random
    /// number generator.
    pub fn generate_ed25519() -> Result<Self> {
        Ok(Self::from_ed25519_bytes(&random_bytes::<32>()?))
    }

    pub fn algorithm(&self) -> Algorithm {
        match self {
            SigningKey::Es256(_) => Algorithm::Es256,
            SigningKey::Ed25519(_) => Algorithm::EdDsa,
        }
    }

    /// The corresponding public key, for distributing to verifiers.
    pub fn verifying_key(&self) -> VerifyingKey {
        match self {
            SigningKey::Es256(k) => VerifyingKey::Es256(*k.verifying_key()),
            SigningKey::Ed25519(k) => VerifyingKey::Ed25519(k.verifying_key()),
        }
    }

    fn sign(&self, data: &[u8]) -> Vec<u8> {
        match self {
            SigningKey::Es256(k) => {
                let sig: p256::ecdsa::Signature = k.sign(data);
                sig.to_bytes().to_vec()
            }
            SigningKey::Ed25519(k) => {
                let sig: ed25519_dalek::Signature = k.sign(data);
                sig.to_bytes().to_vec()
            }
        }
    }
}

fn random_bytes<const N: usize>() -> Result<[u8; N]> {
    let mut buf = [0u8; N];
    getrandom::fill(&mut buf).map_err(|e| SigningError::KeyGeneration(e.to_string()))?;
    Ok(buf)
}

/// A public key for verifying an [`AgentCard`]'s signature.
#[derive(Clone)]
pub enum VerifyingKey {
    Es256(p256::ecdsa::VerifyingKey),
    Ed25519(ed25519_dalek::VerifyingKey),
}

impl VerifyingKey {
    /// Builds an ES256 verifying key from a 33-byte SEC1-compressed (or
    /// 65-byte uncompressed) P-256 point.
    pub fn from_es256_bytes(bytes: &[u8]) -> Result<Self> {
        p256::ecdsa::VerifyingKey::from_sec1_bytes(bytes)
            .map(VerifyingKey::Es256)
            .map_err(|e| SigningError::InvalidKey(e.to_string()))
    }

    /// Builds an EdDSA (Ed25519) verifying key from its 32-byte encoding.
    pub fn from_ed25519_bytes(bytes: &[u8; 32]) -> Result<Self> {
        ed25519_dalek::VerifyingKey::from_bytes(bytes)
            .map(VerifyingKey::Ed25519)
            .map_err(|e| SigningError::InvalidKey(e.to_string()))
    }

    pub fn algorithm(&self) -> Algorithm {
        match self {
            VerifyingKey::Es256(_) => Algorithm::Es256,
            VerifyingKey::Ed25519(_) => Algorithm::EdDsa,
        }
    }

    fn verify(&self, data: &[u8], sig_bytes: &[u8]) -> Result<()> {
        match self {
            VerifyingKey::Es256(k) => {
                let sig = p256::ecdsa::Signature::from_slice(sig_bytes)
                    .map_err(|e| SigningError::InvalidKey(e.to_string()))?;
                k.verify(data, &sig).map_err(|_| SigningError::VerificationFailed)
            }
            VerifyingKey::Ed25519(k) => {
                let sig = ed25519_dalek::Signature::from_slice(sig_bytes)
                    .map_err(|e| SigningError::InvalidKey(e.to_string()))?;
                k.verify(data, &sig).map_err(|_| SigningError::VerificationFailed)
            }
        }
    }
}

/// Produces the exact bytes an [`AgentCard`] signature is computed over:
/// the card's JSON Canonicalization Scheme (RFC 8785) representation,
/// with the `signatures` field excluded (spec Section 8.4.1). See the
/// module docs for the scope of JCS this implements.
pub fn canonical_json(card: &AgentCard) -> std::result::Result<String, serde_json::Error> {
    let mut unsigned = card.clone();
    unsigned.signatures.clear();
    // Round-tripping through `Value` (whose `Map` is a `BTreeMap` in this
    // crate's default serde_json configuration) is what gets keys into
    // RFC 8785's required lexicographic order - a plain
    // `serde_json::to_string(&unsigned)` would serialize fields in struct
    // declaration order instead.
    let value = serde_json::to_value(&unsigned)?;
    serde_json::to_string(&value)
}

/// Computes a JWS signature over `card` (spec Section 8.4), returning an
/// [`AgentCardSignature`] ready to push onto `card.signatures`. `kid`, if
/// given, is included in the protected header so verifiers can pick the
/// right key when a card has multiple signatures; if omitted, a `kid` is
/// still always included (spec Section 8.4.2 lists it, alongside `alg` and
/// `typ`, as a header member the protected header "MUST include"),
/// deterministically derived from the key's own public material so it
/// stays consistent across every signature this key ever makes rather than
/// e.g. a fresh random value each call, which would defeat a verifier's
/// `kid`-based key lookup.
pub fn sign_agent_card(card: &AgentCard, key: &SigningKey, kid: Option<&str>) -> Result<AgentCardSignature> {
    let mut header = Map::new();
    header.insert("alg".to_string(), json!(key.algorithm().jwa_name()));
    header.insert("typ".to_string(), json!("JOSE"));
    let kid = kid.map(str::to_string).unwrap_or_else(|| default_kid(key));
    header.insert("kid".to_string(), json!(kid));
    let protected_b64 = URL_SAFE_NO_PAD.encode(serde_json::to_vec(&Value::Object(header))?);

    let payload_b64 = URL_SAFE_NO_PAD.encode(canonical_json(card)?);
    let signing_input = format!("{protected_b64}.{payload_b64}");

    let signature_b64 = URL_SAFE_NO_PAD.encode(key.sign(signing_input.as_bytes()));

    Ok(AgentCardSignature {
        protected: protected_b64,
        signature: signature_b64,
        header: None,
    })
}

/// A stable `kid` derived from `key`'s own public material (base64url of
/// its public key bytes - the SEC1-compressed point for ES256, the raw
/// 32-byte public key for EdDSA) - see [`sign_agent_card`].
fn default_kid(key: &SigningKey) -> String {
    let public_bytes: Vec<u8> = match key.verifying_key() {
        VerifyingKey::Es256(vk) => vk.to_sec1_bytes().to_vec(),
        VerifyingKey::Ed25519(vk) => vk.to_bytes().to_vec(),
    };
    URL_SAFE_NO_PAD.encode(public_bytes)
}

/// Verifies that `signature` is a valid JWS over `card`'s canonical form,
/// produced by the private key matching `key` (spec Section 8.4.3).
/// Recomputes the canonical payload from `card` itself - the signature
/// object never carries the payload (it's "detached": implied to be the
/// signed document, per how [`AgentCardSignature`] has no `payload`
/// field).
pub fn verify_agent_card_signature(
    card: &AgentCard,
    signature: &AgentCardSignature,
    key: &VerifyingKey,
) -> Result<()> {
    let protected_json = URL_SAFE_NO_PAD.decode(&signature.protected)?;
    let header: Value =
        serde_json::from_slice(&protected_json).map_err(|_| SigningError::InvalidProtectedHeader)?;
    let alg = header
        .get("alg")
        .and_then(Value::as_str)
        .ok_or(SigningError::InvalidProtectedHeader)?;
    if alg != key.algorithm().jwa_name() {
        return Err(SigningError::AlgorithmMismatch);
    }

    let payload_b64 = URL_SAFE_NO_PAD.encode(canonical_json(card)?);
    let signing_input = format!("{}.{}", signature.protected, payload_b64);

    let signature_bytes = URL_SAFE_NO_PAD.decode(&signature.signature)?;
    key.verify(signing_input.as_bytes(), &signature_bytes)
}

/// Ergonomic `card.signed(&key, kid)` / `card.verify_signature(&sig, &key)`
/// wrappers around [`sign_agent_card`] / [`verify_agent_card_signature`].
pub trait AgentCardSigningExt {
    /// Returns a clone of this card with a new signature over it appended
    /// to `signatures`.
    fn signed(&self, key: &SigningKey, kid: Option<&str>) -> Result<AgentCard>;

    /// Verifies one of this card's signatures against `key`.
    fn verify_signature(&self, signature: &AgentCardSignature, key: &VerifyingKey) -> Result<()>;
}

impl AgentCardSigningExt for AgentCard {
    fn signed(&self, key: &SigningKey, kid: Option<&str>) -> Result<AgentCard> {
        let signature = sign_agent_card(self, key, kid)?;
        let mut signed = self.clone();
        signed.signatures.push(signature);
        Ok(signed)
    }

    fn verify_signature(&self, signature: &AgentCardSignature, key: &VerifyingKey) -> Result<()> {
        verify_agent_card_signature(self, signature, key)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::types::{AgentCard, AgentInterface, AgentSkill};

    fn sample_card() -> AgentCard {
        AgentCard::new(
            "Test Agent",
            "An agent used to test JWS signing.",
            "1.0.0",
            AgentInterface::json_rpc("https://agent.example.com"),
        )
        .with_streaming(true)
        .with_skill(AgentSkill::new("echo", "Echo", "Echoes input.").with_tags(["demo"]))
    }

    // Fixed (not random) test key material, so these tests are
    // deterministic and don't depend on `getrandom` at all.
    const ES256_SEED: [u8; 32] = [7u8; 32];
    const ED25519_SEED: [u8; 32] = [11u8; 32];

    #[test]
    fn canonical_json_excludes_signatures_and_sorts_keys() {
        let mut card = sample_card();
        card.signatures.push(AgentCardSignature {
            protected: "x".to_string(),
            signature: "y".to_string(),
            header: None,
        });

        let json = canonical_json(&card).unwrap();
        assert!(
            !json.contains("signatures"),
            "signatures field must be excluded: {json}"
        );

        // Lexicographic key order: "capabilities" < "defaultInputModes" < "name" ...
        let capabilities_pos = json.find("\"capabilities\"").unwrap();
        let name_pos = json.find("\"name\"").unwrap();
        let skills_pos = json.find("\"skills\"").unwrap();
        assert!(
            capabilities_pos < name_pos,
            "keys must be sorted lexicographically: {json}"
        );
        assert!(
            name_pos < skills_pos,
            "keys must be sorted lexicographically: {json}"
        );

        // No insignificant (structural) whitespace - unpadded separators,
        // no newlines. String *content* may of course contain spaces.
        assert!(
            !json.contains(": ") && !json.contains(", ") && !json.contains('\n'),
            "canonical JSON must be compact: {json}"
        );
    }

    /// Spec Section 8.4.2: the protected header "MUST include: `alg` ...
    /// `typ`: SHOULD be set to `"JOSE"` ... `kid`".
    #[test]
    fn protected_header_always_includes_typ_and_kid() {
        fn decode_protected_header(signature: &AgentCardSignature) -> Value {
            let bytes = URL_SAFE_NO_PAD.decode(&signature.protected).unwrap();
            serde_json::from_slice(&bytes).unwrap()
        }

        let key = SigningKey::from_ed25519_bytes(&ED25519_SEED);
        let card = sample_card();

        let with_explicit_kid = card.signed(&key, Some("key-1")).unwrap();
        let header = decode_protected_header(&with_explicit_kid.signatures[0]);
        assert_eq!(header["typ"], "JOSE");
        assert_eq!(header["kid"], "key-1");

        // No `kid` supplied: still present, and stable across repeated
        // calls with the same key (not e.g. a fresh random value each
        // time, which would defeat a verifier's `kid`-based key lookup).
        let without_kid_first = card.signed(&key, None).unwrap();
        let without_kid_second = card.signed(&key, None).unwrap();
        let header_first = decode_protected_header(&without_kid_first.signatures[0]);
        let header_second = decode_protected_header(&without_kid_second.signatures[0]);
        assert_eq!(header_first["typ"], "JOSE");
        assert!(header_first["kid"].is_string() && !header_first["kid"].as_str().unwrap().is_empty());
        assert_eq!(header_first["kid"], header_second["kid"]);
    }

    #[test]
    fn es256_round_trip() {
        let key = SigningKey::from_es256_bytes(&ES256_SEED).unwrap();
        let card = sample_card();
        let signed = card.signed(&key, Some("key-1")).unwrap();
        assert_eq!(signed.signatures.len(), 1);

        let verifying_key = key.verifying_key();
        signed
            .verify_signature(&signed.signatures[0], &verifying_key)
            .expect("ES256 signature must verify");
    }

    #[test]
    fn ed25519_round_trip() {
        let key = SigningKey::from_ed25519_bytes(&ED25519_SEED);
        let card = sample_card();
        let signed = card.signed(&key, None).unwrap();

        let verifying_key = key.verifying_key();
        signed
            .verify_signature(&signed.signatures[0], &verifying_key)
            .expect("EdDSA signature must verify");
    }

    #[test]
    fn tampering_with_the_card_invalidates_the_signature() {
        let key = SigningKey::from_ed25519_bytes(&ED25519_SEED);
        let card = sample_card();
        let signed = card.signed(&key, None).unwrap();

        let mut tampered = signed.clone();
        tampered.name = "A Different Agent".to_string();

        let verifying_key = key.verifying_key();
        let err = tampered
            .verify_signature(&signed.signatures[0], &verifying_key)
            .unwrap_err();
        assert!(matches!(err, SigningError::VerificationFailed));
    }

    #[test]
    fn verifying_with_the_wrong_key_fails() {
        let key = SigningKey::from_ed25519_bytes(&ED25519_SEED);
        let other_key = SigningKey::from_ed25519_bytes(&[99u8; 32]);
        let card = sample_card();
        let signed = card.signed(&key, None).unwrap();

        let err = signed
            .verify_signature(&signed.signatures[0], &other_key.verifying_key())
            .unwrap_err();
        assert!(matches!(err, SigningError::VerificationFailed));
    }

    #[test]
    fn verifying_with_the_wrong_algorithm_is_rejected_before_checking_bytes() {
        let es256_key = SigningKey::from_es256_bytes(&ES256_SEED).unwrap();
        let ed25519_key = SigningKey::from_ed25519_bytes(&ED25519_SEED);
        let card = sample_card();
        let signed = card.signed(&es256_key, None).unwrap();

        let err = signed
            .verify_signature(&signed.signatures[0], &ed25519_key.verifying_key())
            .unwrap_err();
        assert!(matches!(err, SigningError::AlgorithmMismatch));
    }

    #[test]
    fn multiple_signatures_can_coexist() {
        let es256_key = SigningKey::from_es256_bytes(&ES256_SEED).unwrap();
        let ed25519_key = SigningKey::from_ed25519_bytes(&ED25519_SEED);
        let card = sample_card();

        let signed_once = card.signed(&es256_key, Some("es256-key")).unwrap();
        let signed_twice = signed_once.signed(&ed25519_key, Some("ed25519-key")).unwrap();
        assert_eq!(signed_twice.signatures.len(), 2);

        signed_twice
            .verify_signature(&signed_twice.signatures[0], &es256_key.verifying_key())
            .expect("first signature must verify");
        signed_twice
            .verify_signature(&signed_twice.signatures[1], &ed25519_key.verifying_key())
            .expect("second signature must verify");
    }

    #[test]
    fn generated_keys_round_trip_too() {
        let key = SigningKey::generate_es256().expect("key generation");
        let card = sample_card();
        let signed = card.signed(&key, None).unwrap();
        signed
            .verify_signature(&signed.signatures[0], &key.verifying_key())
            .expect("generated ES256 key must round-trip");

        let key = SigningKey::generate_ed25519().expect("key generation");
        let signed = card.signed(&key, None).unwrap();
        signed
            .verify_signature(&signed.signatures[0], &key.verifying_key())
            .expect("generated Ed25519 key must round-trip");
    }
}
