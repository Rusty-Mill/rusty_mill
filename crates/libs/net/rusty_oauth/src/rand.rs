//! Cryptographically secure random byte generation. Used for PKCE code
//! verifiers (RFC 7636), `state` and `nonce` values (RFC 6749 §10.12 /
//! OIDC), and any other value that must be unguessable.
//!
//! The OS CSPRNG plumbing itself (`/dev/urandom` on Unix, `BCryptGenRandom`
//! on Windows) lives in [`rusty_rand`], a workspace sibling with no
//! external dependencies that was extracted from this module and
//! `rusty_uuid`'s identical copy of it. This module keeps the crate's own
//! error type and function names so callers are unchanged.

use std::fmt;

/// The OS could not supply secure random bytes.
#[derive(Debug)]
pub struct RandError(rusty_rand::Error);

impl fmt::Display for RandError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        self.0.fmt(f)
    }
}

impl std::error::Error for RandError {
    fn source(&self) -> Option<&(dyn std::error::Error + 'static)> {
        Some(&self.0)
    }
}

/// Fills `buf` with cryptographically secure random bytes from the OS CSPRNG.
pub fn fill_random(buf: &mut [u8]) -> Result<(), RandError> {
    rusty_rand::fill(buf).map_err(RandError)
}

/// Returns `len` cryptographically secure random bytes.
pub fn random_bytes(len: usize) -> Result<Vec<u8>, RandError> {
    rusty_rand::bytes(len).map_err(RandError)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn fills_requested_length() {
        let bytes = random_bytes(32).expect("random bytes");
        assert_eq!(bytes.len(), 32);
    }

    #[test]
    fn not_all_zero_and_differs_between_calls() {
        let a = random_bytes(32).unwrap();
        let b = random_bytes(32).unwrap();
        assert_ne!(a, [0u8; 32]);
        assert_ne!(a, b);
    }
}
