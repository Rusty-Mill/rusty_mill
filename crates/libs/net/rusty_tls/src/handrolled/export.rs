//! Keying material exporters — RFC 5705 for TLS 1.2, RFC 8446 section 7.5 for
//! TLS 1.3.
//!
//! An exporter turns the secrets of a finished handshake into bytes both ends
//! can compute and nobody else can: for channel binding, for protocols that
//! run inside TLS and want keys of their own (RFC 5764's DTLS-SRTP, EAP-TLS),
//! for anything that must be tied to *this* connection. The `label` names the
//! purpose, so two purposes never receive the same bytes, and the `context`
//! mixes in application data.
//!
//! The two versions differ in one visible way. TLS 1.2 distinguishes "no
//! context" from "an empty context" (the second adds a two-octet length to the
//! PRF seed); TLS 1.3 does not, because both hash to the same value. The API
//! takes `Option<&[u8]>` so the difference is expressible, and TLS 1.3 treats
//! `None` and `Some(&[])` alike.
//!
//! Labels are text: IANA's registry of exporter labels is ASCII, and a label
//! that is not valid UTF-8 is refused rather than passed through.

use super::schedule::{derive_secret, expand_label, Hash};
use super::schedule12::{prf, MasterSecret};

/// Why an export was refused.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
#[non_exhaustive]
pub enum ExportError {
    /// The label is not text, or is too long to fit the TLS 1.3 `HkdfLabel`.
    BadLabel,
    /// RFC 5705 section 4: a label may not collide with the ones TLS 1.2 itself
    /// uses (`client finished`, `server finished`, `master secret`,
    /// `key expansion`), or an exporter could be made to reproduce a secret.
    ReservedLabel,
    /// A TLS 1.2 context longer than its two-octet length can carry.
    ContextTooLong,
    /// More output was asked for than the key derivation can produce.
    TooLong,
    /// The connection carries no exporter secret. Every connection this crate
    /// completes does; this is the answer for one that somehow does not, in
    /// place of keying material derived from nothing.
    Unavailable,
}

impl core::fmt::Display for ExportError {
    fn fmt(&self, f: &mut core::fmt::Formatter<'_>) -> core::fmt::Result {
        f.write_str(match self {
            Self::BadLabel => "the exporter label is not text, or too long",
            Self::ReservedLabel => "the exporter label is reserved by TLS 1.2",
            Self::ContextTooLong => "the exporter context is longer than 65535 octets",
            Self::TooLong => "more exported keying material was asked for than can be derived",
            Self::Unavailable => "this connection has no exporter secret",
        })
    }
}

impl std::error::Error for ExportError {}

/// RFC 8446 section 7.5, given the connection's `exporter_master_secret`.
pub(super) fn export13(
    hash: Hash,
    exporter_secret: &[u8],
    label: &[u8],
    context: &[u8],
    out: &mut [u8],
) -> Result<(), ExportError> {
    // An empty secret would still derive *something*, from an all-zero key.
    if exporter_secret.is_empty() {
        return Err(ExportError::Unavailable);
    }
    let label = core::str::from_utf8(label).map_err(|_| ExportError::BadLabel)?;
    // "tls13 " is six octets and the whole label has a one-octet length.
    if label.len() + 6 > 255 {
        return Err(ExportError::BadLabel);
    }
    // HKDF-Expand yields at most 255 blocks, and the HkdfLabel length is a u16.
    if out.len() > 255 * hash.len() || out.len() > usize::from(u16::MAX) {
        return Err(ExportError::TooLong);
    }
    // Derive-Secret(exporter_master_secret, label, "")
    let derived = derive_secret(hash, exporter_secret, label, &hash.empty_hash());
    // HKDF-Expand-Label(derived, "exporter", Hash(context), length)
    let keying = expand_label(hash, &derived, "exporter", &hash.hash(context), out.len());
    out.copy_from_slice(&keying);
    Ok(())
}

/// RFC 5705 section 4, given the connection's master secret and both randoms.
pub(super) fn export12(
    hash: Hash,
    master: &MasterSecret,
    client_random: &[u8],
    server_random: &[u8],
    label: &[u8],
    context: Option<&[u8]>,
    out: &mut [u8],
) -> Result<(), ExportError> {
    const RESERVED: [&[u8]; 4] = [
        b"client finished",
        b"server finished",
        b"master secret",
        b"key expansion",
    ];
    if RESERVED.contains(&label) {
        return Err(ExportError::ReservedLabel);
    }
    let context_length;
    let (length, body): (&[u8], &[u8]) = match context {
        None => (&[], &[]),
        Some(context) => {
            let length = u16::try_from(context.len()).map_err(|_| ExportError::ContextTooLong)?;
            context_length = length.to_be_bytes();
            (&context_length, context)
        }
    };
    prf(
        hash,
        master.as_bytes(),
        label,
        &[client_random, server_random, length, body],
        out,
    );
    Ok(())
}
