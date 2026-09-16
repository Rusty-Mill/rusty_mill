//! Ephemeral key exchange — stage 3c-i.
//!
//! TLS 1.3 is ephemeral-only: every connection agrees a fresh secret, and the
//! long-term key in the certificate signs the handshake rather than encrypting
//! anything. This module is the agreement half of that.
//!
//! # What is hand-rolled here, and what is not
//!
//! Very little, deliberately. The arithmetic is `ring`'s — ADR-0002 §6, and
//! X25519 is exactly the kind of primitive that argument is about. What this
//! module contributes is the *protocol* side: which `NamedGroup` numbers mean
//! which algorithm, and the shape of the API around a key that must be used
//! once.
//!
//! Being clear about that matters, because the temptation is to describe every
//! check here as a defence. The peer's public key is validated by `ring`, not
//! by a length check written here — see [`KeyExchange::agree`].
//!
//! # One use, enforced by the type
//!
//! [`KeyExchange::agree`] takes `self` by value. An ephemeral key that is
//! reused across two connections is no longer ephemeral, and forward secrecy
//! is exactly the property it was there to provide. `ring` makes the same
//! choice for the same reason; mirroring it means a reuse is a compile error
//! rather than a review comment.
//!
//! # The secret never gets a home
//!
//! [`KeyExchange::agree`] hands the shared secret to a closure instead of
//! returning it. That is not ceremony: a returned `Vec<u8>` is a copy this
//! crate would then have to promise to erase, and it cannot — zeroing on
//! `Drop` needs either a dependency or `unsafe`, and this crate forbids
//! `unsafe` outright. A closure scope is a promise the compiler keeps for
//! free. Callers that genuinely need the bytes can still `|s| s.to_vec()`,
//! which puts the copy where a reader can see it.
//!
//! ```text
//! let schedule = kx.agree(&peer_key, |secret| {
//!     KeySchedule::new(Hash::Sha256).into_handshake(secret)
//! })?;
//! ```

use core::fmt;

use ring::agreement;
use ring::rand::SystemRandom;

/// Why a key exchange did not produce a secret.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
#[non_exhaustive]
pub enum KxError {
    /// The `NamedGroup` is not one this module implements.
    UnsupportedGroup(u16),
    /// Generating an ephemeral key failed, which means the system random
    /// source failed. Nothing here can recover from that.
    Generation,
    /// The peer's public key was refused.
    ///
    /// Carries no detail, because every reason looks alike from outside: a
    /// wrong length, a point that is not on the curve, and an X25519 key that
    /// drives the output to all-zeroes are all just "no secret". `ring`
    /// decides all three — see [`KeyExchange::agree`].
    BadPeerKey,
}

impl fmt::Display for KxError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::UnsupportedGroup(group) => write!(f, "unsupported named group 0x{group:04x}"),
            Self::Generation => f.write_str("could not generate an ephemeral key"),
            Self::BadPeerKey => f.write_str("the peer's key share was refused"),
        }
    }
}

impl std::error::Error for KxError {}

/// A TLS `NamedGroup` (RFC 8446 §4.2.7) this module can agree over.
///
/// The three TLS 1.3 clients actually negotiate. The finite-field groups
/// (`ffdhe*`) are absent because `ring` does not implement them and because
/// nothing needs them; an unknown group is [`KxError::UnsupportedGroup`],
/// never a guess.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
#[non_exhaustive]
pub enum NamedGroup {
    /// `secp256r1(0x0017)` — NIST P-256.
    SecP256R1,
    /// `secp384r1(0x0018)` — NIST P-384.
    SecP384R1,
    /// `x25519(0x001d)`, the one a TLS 1.3 client should prefer.
    X25519,
}

impl NamedGroup {
    /// The wire value.
    pub const fn as_u16(self) -> u16 {
        match self {
            Self::SecP256R1 => 0x0017,
            Self::SecP384R1 => 0x0018,
            Self::X25519 => 0x001d,
        }
    }

    /// Decode a wire value, or `None` for a group this module cannot use.
    pub const fn from_u16(value: u16) -> Option<Self> {
        match value {
            0x0017 => Some(Self::SecP256R1),
            0x0018 => Some(Self::SecP384R1),
            0x001d => Some(Self::X25519),
            _ => None,
        }
    }

    fn ring_algorithm(self) -> &'static agreement::Algorithm {
        match self {
            Self::SecP256R1 => &agreement::ECDH_P256,
            Self::SecP384R1 => &agreement::ECDH_P384,
            Self::X25519 => &agreement::X25519,
        }
    }
}

/// One connection's ephemeral private key, with its public key already
/// computed for the `key_share` extension.
///
/// Not `Clone`, and [`KeyExchange::agree`] consumes it — see the module docs
/// on why one use is a type-level property here rather than a convention.
pub struct KeyExchange {
    group: NamedGroup,
    private: agreement::EphemeralPrivateKey,
    public: Vec<u8>,
}

impl KeyExchange {
    /// Generate a fresh ephemeral key for `group`.
    pub fn generate(group: NamedGroup) -> Result<Self, KxError> {
        let rng = SystemRandom::new();
        let private = agreement::EphemeralPrivateKey::generate(group.ring_algorithm(), &rng)
            .map_err(|_| KxError::Generation)?;
        let public = private
            .compute_public_key()
            .map_err(|_| KxError::Generation)?
            .as_ref()
            .to_vec();

        Ok(Self {
            group,
            private,
            public,
        })
    }

    /// Which group this key is for.
    pub const fn group(&self) -> NamedGroup {
        self.group
    }

    /// The public key, in the encoding a `key_share` entry carries: 32 raw
    /// octets for X25519 (RFC 7748 §5), an uncompressed point for the NIST
    /// curves (RFC 8446 §4.2.8.2, SEC1 §2.3.3).
    ///
    /// Both encodings are `ring`'s output, unmodified. There is no re-encoding
    /// step here to get wrong.
    pub fn public_key(&self) -> &[u8] {
        &self.public
    }

    /// Agree a shared secret with the peer's `key_share`, and hand it to `f`.
    ///
    /// # What validates the peer's key
    ///
    /// `ring` does, not this function. It checks the length, checks that a
    /// NIST point is on the curve, and — the one that is easy to forget —
    /// refuses an X25519 exchange whose output is all zeroes, which is what a
    /// small-order peer key produces (RFC 7748 §6.1, required by RFC 8446
    /// §7.4.2). There is no additional check here, and claiming one would be
    /// worse than having none: a reader would trust a defence that was not
    /// doing the work.
    ///
    /// What this function does own is that a failure is
    /// [`KxError::BadPeerKey`] with nothing attached, so no distinction leaks
    /// back to whoever sent the key.
    pub fn agree<T>(
        self,
        peer_public_key: &[u8],
        f: impl FnOnce(&[u8]) -> T,
    ) -> Result<T, KxError> {
        let peer = agreement::UnparsedPublicKey::new(self.group.ring_algorithm(), peer_public_key);
        agreement::agree_ephemeral(self.private, &peer, f).map_err(|_| KxError::BadPeerKey)
    }
}

/// Deliberately says nothing about the private key.
///
/// `ring`'s own `Debug` for an `EphemeralPrivateKey` already prints only the
/// algorithm, so this is not fixing a leak — it is refusing to *inherit* the
/// absence of one. The guarantee that key material never reaches a log is
/// worth owning here rather than depending on a dependency's rendering
/// choices, which is the same reasoning the record layer applies to its own
/// types.
impl fmt::Debug for KeyExchange {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("KeyExchange")
            .field("group", &self.group)
            .field("public_key_bytes", &self.public.len())
            .field("private", &"<redacted>")
            .finish()
    }
}
