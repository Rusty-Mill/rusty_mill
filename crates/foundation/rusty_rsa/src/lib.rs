//! Hand-rolled, dependency-free [`BigUint`] arithmetic and [`sha256`],
//! shared by `rusty_oauth` (RS256 JWT verification, ES256 elliptic-curve
//! verification) and `rusty_rdp` (RDP standard security's RSA public-key
//! encryption).
//!
//! Despite the crate name: [`BigUint`] is general-purpose big-integer math,
//! not RSA-specific -- `rusty_oauth`'s own ES256/ECC support builds on it
//! too, via the same `add`/`sub`/`mul`/`bit`/`modpow` operations RSA needs.
//! [`Sha256`]/[`sha256`] is a general hash, likewise not RSA-specific
//! (`rusty_rdp` uses it for CredSSP's public-key binding, unrelated to
//! RSA). The name follows the two callers' actual primary use case for
//! this crate -- RSA public-key math -- rather than trying to invent a
//! more generic name for what is, in practice, an RSA-support crate that
//! happens to expose its building blocks.
//!
//! What's deliberately **not** here: each crate's own `RsaPublicKey`
//! wrapper type. Those diverge for real reasons, not just code shape --
//! `rusty_oauth`'s is verification-only (PKCS#1 v1.5, big-endian
//! JWK-sourced components, explicitly refuses to implement private-key
//! operations: "needs constant-time modular exponentiation... a much
//! larger undertaking than a general OAuth crate should take on") while
//! `rusty_rdp`'s does raw encryption *and* decryption (little-endian,
//! DER-cert-sourced, with a companion `RsaPrivateKey`) for an unrelated
//! wire protocol. Forcing one wrapper shape on both would either drop
//! `rusty_rdp`'s decrypt path or add private-key operations to
//! `rusty_oauth`'s deliberately narrower scope -- so only the shared math
//! moved; each crate's own `RsaPublicKey` stays local, built on
//! [`BigUint`] from here instead of a local copy.

mod bigint;
mod sha256;

pub use bigint::BigUint;
pub use sha256::{sha256, Digest, Sha256};
