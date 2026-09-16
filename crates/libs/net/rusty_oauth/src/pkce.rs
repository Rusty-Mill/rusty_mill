//! Proof Key for Code Exchange (PKCE), RFC 7636.
//!
//! PKCE is mandatory for public clients and strongly recommended for all
//! clients under OAuth 2.1; this crate always generates it for
//! authorization-code requests unless explicitly disabled.

use crate::crypto::sha256::sha256;
use crate::encoding::base64::encode_url_safe_no_pad;
use crate::rand::random_bytes;
use std::fmt;

/// RFC 7636 §4.2: the transformation applied to the `code_verifier` to
/// produce the `code_challenge` sent in the authorization request.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum CodeChallengeMethod {
    /// `S256`: `BASE64URL-ENCODE(SHA256(ASCII(code_verifier)))`. The only
    /// method permitted under OAuth 2.1; always prefer this.
    S256,
    /// `plain`: `code_challenge == code_verifier`. Included only for
    /// interoperability with servers that don't support `S256`; RFC 7636
    /// §7.2 and OAuth 2.1 discourage its use.
    Plain,
}

impl CodeChallengeMethod {
    pub fn as_str(&self) -> &'static str {
        match self {
            CodeChallengeMethod::S256 => "S256",
            CodeChallengeMethod::Plain => "plain",
        }
    }
}

impl fmt::Display for CodeChallengeMethod {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "{}", self.as_str())
    }
}

/// A PKCE `code_verifier` / `code_challenge` pair for a single
/// authorization request.
#[derive(Debug, Clone)]
pub struct Pkce {
    pub code_verifier: String,
    pub code_challenge: String,
    pub code_challenge_method: CodeChallengeMethod,
}

/// Minimum/maximum `code_verifier` length per RFC 7636 §4.1.
const MIN_VERIFIER_LEN: usize = 43;
const MAX_VERIFIER_LEN: usize = 128;

impl Pkce {
    /// Generates a new PKCE pair using the `S256` challenge method, with a
    /// cryptographically random 32-byte verifier (base64url-encoded to 43
    /// characters -- the minimum length RFC 7636 allows, and the value
    /// used by most interoperable implementations).
    pub fn generate() -> crate::error::Result<Self> {
        Self::generate_with(CodeChallengeMethod::S256)
    }

    /// Generates a new PKCE pair using the given challenge method.
    pub fn generate_with(method: CodeChallengeMethod) -> crate::error::Result<Self> {
        let verifier_bytes = random_bytes(32)?;
        let code_verifier = encode_url_safe_no_pad(&verifier_bytes);
        debug_assert!(is_valid_verifier(&code_verifier));

        let code_challenge = match method {
            CodeChallengeMethod::S256 => encode_url_safe_no_pad(&sha256(code_verifier.as_bytes())),
            CodeChallengeMethod::Plain => code_verifier.clone(),
        };

        Ok(Pkce {
            code_verifier,
            code_challenge,
            code_challenge_method: method,
        })
    }

    /// Derives the `code_challenge` for an existing verifier (e.g. one
    /// persisted across a redirect), without generating a new one.
    pub fn from_verifier(
        code_verifier: impl Into<String>,
        method: CodeChallengeMethod,
    ) -> Result<Self, PkceError> {
        let code_verifier = code_verifier.into();
        if !is_valid_verifier(&code_verifier) {
            return Err(PkceError::InvalidVerifier);
        }
        let code_challenge = match method {
            CodeChallengeMethod::S256 => encode_url_safe_no_pad(&sha256(code_verifier.as_bytes())),
            CodeChallengeMethod::Plain => code_verifier.clone(),
        };
        Ok(Pkce {
            code_verifier,
            code_challenge,
            code_challenge_method: method,
        })
    }

    /// Verifies that `code_verifier` (received back at the token endpoint)
    /// matches the `code_challenge` issued at authorization time, per
    /// RFC 7636 §4.6. Intended for authorization-server implementations.
    pub fn verify(code_verifier: &str, code_challenge: &str, method: CodeChallengeMethod) -> bool {
        if !is_valid_verifier(code_verifier) {
            return false;
        }
        let expected = match method {
            CodeChallengeMethod::S256 => encode_url_safe_no_pad(&sha256(code_verifier.as_bytes())),
            CodeChallengeMethod::Plain => code_verifier.to_string(),
        };
        crate::crypto::hmac::constant_time_eq(expected.as_bytes(), code_challenge.as_bytes())
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum PkceError {
    InvalidVerifier,
}

impl fmt::Display for PkceError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            PkceError::InvalidVerifier => write!(
                f,
                "code_verifier must be 43-128 characters from [A-Za-z0-9-._~]"
            ),
        }
    }
}

impl std::error::Error for PkceError {}

/// RFC 7636 §4.1: `code-verifier = 43*128unreserved`, where
/// `unreserved = ALPHA / DIGIT / "-" / "." / "_" / "~"`.
fn is_valid_verifier(v: &str) -> bool {
    let len = v.len();
    (MIN_VERIFIER_LEN..=MAX_VERIFIER_LEN).contains(&len)
        && v.bytes()
            .all(|b| b.is_ascii_alphanumeric() || matches!(b, b'-' | b'.' | b'_' | b'~'))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn generated_verifier_is_valid_and_challenge_matches() {
        let pkce = Pkce::generate().unwrap();
        assert!(is_valid_verifier(&pkce.code_verifier));
        assert_eq!(pkce.code_challenge_method, CodeChallengeMethod::S256);
        assert!(Pkce::verify(
            &pkce.code_verifier,
            &pkce.code_challenge,
            CodeChallengeMethod::S256
        ));
    }

    #[test]
    fn rfc7636_appendix_b_vector() {
        let verifier = "dBjftJeZ4CVP-mB92K27uhbUJU1p1r_wW1gFWFOEjXk";
        let pkce = Pkce::from_verifier(verifier, CodeChallengeMethod::S256).unwrap();
        assert_eq!(
            pkce.code_challenge,
            "E9Melhoa2OwvFrEMTJguCHaoeK1t8URWbuGJSstw-cM"
        );
        assert!(Pkce::verify(
            verifier,
            &pkce.code_challenge,
            CodeChallengeMethod::S256
        ));
    }

    #[test]
    fn plain_method_challenge_equals_verifier() {
        let pkce = Pkce::generate_with(CodeChallengeMethod::Plain).unwrap();
        assert_eq!(pkce.code_verifier, pkce.code_challenge);
    }

    #[test]
    fn rejects_invalid_verifier() {
        assert!(Pkce::from_verifier("too-short", CodeChallengeMethod::S256).is_err());
        assert!(Pkce::from_verifier("x".repeat(200), CodeChallengeMethod::S256).is_err());
        assert!(Pkce::from_verifier("has spaces!!!".repeat(4), CodeChallengeMethod::S256).is_err());
    }

    #[test]
    fn verify_rejects_wrong_challenge() {
        let pkce = Pkce::generate().unwrap();
        assert!(!Pkce::verify(
            &pkce.code_verifier,
            "wrong-challenge",
            CodeChallengeMethod::S256
        ));
    }
}
