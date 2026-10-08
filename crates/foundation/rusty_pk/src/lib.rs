//! Public-key primitives: signature verification (RSA, ECDSA, Ed25519) and
//! key agreement (X25519).
//!
//! # Verification handles public data only
//!
//! Everything under [`rsa`], [`ecdsa`] and [`ed25519`] verification works on
//! public keys, messages and signatures, so it uses variable-time algorithms
//! where that is faster. Those routines are named `*_vartime`, and must never
//! be called with a secret. Secret-scalar code (X25519) uses only the
//! branch-free field routines in `mont`.
//!
//! No `unsafe`, no allocation outside RSA (variable-size moduli), no
//! dependencies beyond `rusty_sha2` and `rusty_crypto_key`.

#![no_std]
#![forbid(unsafe_code)]

extern crate alloc;

mod der;
mod field;
mod mont;
mod params;
mod weierstrass;

pub mod ecdsa;
pub mod ed25519;
pub mod rsa;

/// Verification failed. Deliberately carries no detail: a verifier that says
/// *why* a signature was rejected is a verifier that can be probed.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct VerifyError;

impl core::fmt::Display for VerifyError {
    fn fmt(&self, f: &mut core::fmt::Formatter<'_>) -> core::fmt::Result {
        f.write_str("signature verification failed")
    }
}
