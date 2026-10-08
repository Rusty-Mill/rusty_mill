//! The TLS 1.2 record layer — AEAD protection and framing (RFC 5246 §6.2.3.3,
//! RFC 5288, RFC 7905).
//!
//! Stage 4b-i of the hand-rolled engine, and the TLS 1.2 counterpart of
//! [`super::record`]. Like that module it takes an *already derived* key and
//! turns fragments into wire records and back; it knows nothing about
//! handshakes or key derivation.
//!
//! # How this differs from TLS 1.3
//!
//! The AEADs are the same three, but almost everything around them is not, and
//! each difference is a place to be wrong:
//!
//! | | TLS 1.2 (this module) | TLS 1.3 ([`super::record`]) |
//! | --- | --- | --- |
//! | Content type | cleartext in the header | hidden inside the AEAD |
//! | Padding | none | optional |
//! | Additional data | `seq(8) ‖ type ‖ version(2) ‖ plaintext_len(2)` | the 5-byte header |
//! | AES-GCM nonce | 4-byte salt ‖ 8-byte explicit nonce **on the wire** | 12-byte IV ⊕ sequence |
//! | ChaCha20 nonce | 12-byte IV ⊕ sequence (RFC 7905) | the same |
//! | Version bytes | authenticated, must be `0x0303` | ignored |
//!
//! Two of those deserve a sentence each. The additional data uses the
//! *plaintext* length, not the length on the wire, so an opener has to subtract
//! the overhead before it can even build the AAD. And because the content type
//! travels in the clear, it is protected only by being part of the AAD: flipping
//! it makes the tag fail, which is why no separate "outer type" check exists
//! here the way it does for TLS 1.3.
//!
//! # The explicit nonce
//!
//! RFC 5288 §3 leaves the 8-byte explicit part of an AES-GCM nonce to the
//! sender, requiring only that it never repeats under one key, and recommends
//! the record sequence number. That is what [`Sealer`] sends. The opener never
//! assumes it: it reads the explicit part off the wire, because a peer is free
//! to choose differently (rustls, for one, XORs the sequence number into a
//! starting value taken from the key block). A nonce repeated under one GCM key
//! reveals the authentication key, so [`Sealer`] derives it from a counter that
//! refuses to wrap.
//!
//! # Sequence numbers
//!
//! Sequence numbers are implicit, per direction, start at zero when a key is
//! installed (a `ChangeCipherSpec` in TLS 1.2), and must not wrap (RFC 5246
//! §6.1). Both halves own their counter. It only advances on success, so a
//! rejected record does not consume a number — TLS itself treats a decryption
//! failure as fatal (§6.2.3.3), but a caller that chooses to continue is not
//! silently desynchronised.

use core::fmt;

use ring::aead;

use super::record::{
    Aead, ContentType, Opened, RecordError, Sequence, HEADER_LEN, MAX_FRAGMENT_LEN, TAG_LEN,
};

/// The only version a protected TLS 1.2 record may carry.
const RECORD_VERSION: [u8; 2] = [0x03, 0x03];

/// Maximum `TLSCiphertext.fragment` length — 2^14 + 2048 (RFC 5246 §6.2.3).
pub const MAX_CIPHERTEXT_LEN: usize = MAX_FRAGMENT_LEN + 2048;

/// Length of the explicit nonce an AES-GCM record carries.
pub const GCM_EXPLICIT_NONCE_LEN: usize = 8;

/// Length of the implicit salt for AES-GCM (RFC 5288 §3).
pub const GCM_SALT_LEN: usize = 4;

/// Length of the IV for ChaCha20-Poly1305 (RFC 7905 §2).
pub const CHACHA_IV_LEN: usize = 12;

/// Length of the additional data: `seq(8) ‖ type(1) ‖ version(2) ‖ length(2)`.
const AAD_LEN: usize = 13;

/// How long the fixed (key-block) part of the nonce is for `alg`.
///
/// Four for AES-GCM, where it is the implicit salt; twelve for
/// ChaCha20-Poly1305, where it is the whole IV.
pub const fn fixed_iv_len(alg: Aead) -> usize {
    match alg {
        Aead::Aes128Gcm | Aead::Aes256Gcm => GCM_SALT_LEN,
        Aead::ChaCha20Poly1305 => CHACHA_IV_LEN,
    }
}

/// Bytes a record grows by beyond its plaintext: explicit nonce plus tag for
/// AES-GCM, tag alone for ChaCha20-Poly1305.
pub const fn overhead(alg: Aead) -> usize {
    match alg {
        Aead::Aes128Gcm | Aead::Aes256Gcm => GCM_EXPLICIT_NONCE_LEN + TAG_LEN,
        Aead::ChaCha20Poly1305 => TAG_LEN,
    }
}

const fn ring_algorithm(alg: Aead) -> &'static aead::Algorithm {
    match alg {
        Aead::Aes128Gcm => &aead::AES_128_GCM,
        Aead::Aes256Gcm => &aead::AES_256_GCM,
        Aead::ChaCha20Poly1305 => &aead::CHACHA20_POLY1305,
    }
}

/// The key and fixed nonce material for one direction of one connection.
struct DirectionKey {
    alg: Aead,
    key: aead::LessSafeKey,
    /// Salt (first four bytes) for AES-GCM, the full IV for ChaCha20-Poly1305.
    fixed: [u8; CHACHA_IV_LEN],
}

impl DirectionKey {
    fn new(alg: Aead, key: &[u8], fixed_iv: &[u8]) -> Result<Self, RecordError> {
        if key.len() != alg.key_len() {
            return Err(RecordError::KeyLength {
                expected: alg.key_len(),
                actual: key.len(),
            });
        }
        if fixed_iv.len() != fixed_iv_len(alg) {
            return Err(RecordError::FixedIvLength {
                expected: fixed_iv_len(alg),
                actual: fixed_iv.len(),
            });
        }

        // The key length is checked above, which is the only way
        // `UnboundKey::new` can fail.
        let unbound = aead::UnboundKey::new(ring_algorithm(alg), key).map_err(|_| {
            RecordError::KeyLength {
                expected: alg.key_len(),
                actual: key.len(),
            }
        })?;

        let mut fixed = [0u8; CHACHA_IV_LEN];
        fixed[..fixed_iv.len()].copy_from_slice(fixed_iv);

        Ok(Self {
            alg,
            key: aead::LessSafeKey::new(unbound),
            fixed,
        })
    }

    /// The nonce for sealing record number `seq`, and the explicit part to put
    /// on the wire (empty for ChaCha20-Poly1305).
    fn seal_nonce(&self, seq: u64) -> (aead::Nonce, Vec<u8>) {
        match self.alg {
            Aead::Aes128Gcm | Aead::Aes256Gcm => {
                let explicit = seq.to_be_bytes();
                (self.gcm_nonce(&explicit), explicit.to_vec())
            }
            Aead::ChaCha20Poly1305 => (self.chacha_nonce(seq), Vec::new()),
        }
    }

    /// `salt ‖ explicit`, RFC 5288 §3.
    fn gcm_nonce(&self, explicit: &[u8]) -> aead::Nonce {
        let mut nonce = [0u8; CHACHA_IV_LEN];
        nonce[..GCM_SALT_LEN].copy_from_slice(&self.fixed[..GCM_SALT_LEN]);
        nonce[GCM_SALT_LEN..].copy_from_slice(explicit);
        // Unique per key when sealing because `explicit` is the sequence
        // number, which never repeats. When opening it is the peer's value and
        // uniqueness is the peer's obligation; opening does not depend on it.
        aead::Nonce::assume_unique_for_key(nonce)
    }

    /// `iv ⊕ (0⁴ ‖ seq)`, RFC 7905 §2.
    fn chacha_nonce(&self, seq: u64) -> aead::Nonce {
        let mut nonce = self.fixed;
        for (out, byte) in nonce[CHACHA_IV_LEN - 8..].iter_mut().zip(seq.to_be_bytes()) {
            *out ^= byte;
        }
        aead::Nonce::assume_unique_for_key(nonce)
    }
}

/// The additional data, RFC 5246 §6.2.3.3: `seq_num ‖ type ‖ version ‖ length`,
/// where `length` is the length of the *plaintext*.
fn additional_data(
    seq: u64,
    typ: ContentType,
    version: [u8; 2],
    plaintext_len: usize,
) -> [u8; AAD_LEN] {
    let mut aad = [0u8; AAD_LEN];
    aad[..8].copy_from_slice(&seq.to_be_bytes());
    aad[8] = typ.as_u8();
    aad[9..11].copy_from_slice(&version);
    // `plaintext_len` is at most `MAX_FRAGMENT_LEN` at every call site, so it
    // fits in sixteen bits.
    aad[11..13].copy_from_slice(&(plaintext_len as u16).to_be_bytes());
    aad
}

/// Protects outgoing records under one direction's key.
///
/// ```
/// use rusty_tls::handrolled::record::{Aead, ContentType};
/// use rusty_tls::handrolled::record12::Sealer;
///
/// let mut sealer = Sealer::new(Aead::Aes128Gcm, &[0u8; 16], &[0u8; 4])?;
/// let record = sealer.seal(ContentType::ApplicationData, b"hello")?;
/// // The content type is visible on the wire in TLS 1.2.
/// assert_eq!(&record[..3], &[23, 0x03, 0x03]);
/// assert_eq!(sealer.sequence(), Some(1));
/// # Ok::<(), rusty_tls::handrolled::record::RecordError>(())
/// ```
pub struct Sealer {
    key: DirectionKey,
    seq: Sequence,
}

impl Sealer {
    /// Build a sealer from an algorithm, a key of that algorithm's length, and
    /// the fixed part of the nonce ([`fixed_iv_len`] bytes).
    ///
    /// The sequence number starts at zero, which is what installing a fresh
    /// key means. To resume mid-stream use [`Sealer::new_at`].
    pub fn new(alg: Aead, key: &[u8], fixed_iv: &[u8]) -> Result<Self, RecordError> {
        Self::new_at(alg, key, fixed_iv, 0)
    }

    /// Like [`Sealer::new`], but resuming at an arbitrary sequence number.
    ///
    /// A key and its sequence number travel together: resuming a key at a
    /// number it has already used repeats a nonce, which for AES-GCM leaks the
    /// authentication key. Only pass a number taken from this key's own
    /// history.
    pub fn new_at(alg: Aead, key: &[u8], fixed_iv: &[u8], seq: u64) -> Result<Self, RecordError> {
        Ok(Self {
            key: DirectionKey::new(alg, key, fixed_iv)?,
            seq: Sequence::starting_at(seq),
        })
    }

    /// The sequence number the next record will use, or `None` if exhausted.
    pub fn sequence(&self) -> Option<u64> {
        self.seq.0
    }

    /// Protect one fragment, returning the complete record including header.
    ///
    /// The sequence number advances only if this succeeds. An empty fragment is
    /// permitted: RFC 5246 allows zero-length `application_data`.
    pub fn seal(&mut self, typ: ContentType, fragment: &[u8]) -> Result<Vec<u8>, RecordError> {
        if fragment.len() > MAX_FRAGMENT_LEN {
            return Err(RecordError::FragmentTooLong {
                len: fragment.len(),
                max: MAX_FRAGMENT_LEN,
            });
        }

        let seq = self.seq.peek()?;
        let (nonce, explicit) = self.key.seal_nonce(seq);
        let aad = additional_data(seq, typ, RECORD_VERSION, fragment.len());

        let mut body = fragment.to_vec();
        self.key
            .key
            .seal_in_place_append_tag(nonce, aead::Aad::from(aad), &mut body)
            .map_err(|_| RecordError::Encrypt)?;

        let wire_len = explicit.len() + body.len();
        debug_assert_eq!(wire_len, fragment.len() + overhead(self.key.alg));

        let mut record = Vec::with_capacity(HEADER_LEN + wire_len);
        record.push(typ.as_u8());
        record.extend_from_slice(&RECORD_VERSION);
        // `wire_len` is at most `MAX_FRAGMENT_LEN` plus the overhead, so it
        // fits in sixteen bits.
        record.extend_from_slice(&(wire_len as u16).to_be_bytes());
        record.extend_from_slice(&explicit);
        record.extend_from_slice(&body);

        self.seq.advance(seq);
        Ok(record)
    }
}

impl fmt::Debug for Sealer {
    /// Prints the sequence number and nothing else — never the key material.
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("Sealer")
            .field("sequence", &self.seq.0)
            .finish_non_exhaustive()
    }
}

/// Unprotects incoming records under one direction's key.
pub struct Opener {
    key: DirectionKey,
    seq: Sequence,
}

impl Opener {
    /// Build an opener from an algorithm, a key of that algorithm's length, and
    /// the fixed part of the nonce ([`fixed_iv_len`] bytes).
    ///
    /// The sequence number starts at zero. To resume mid-stream use
    /// [`Opener::new_at`].
    pub fn new(alg: Aead, key: &[u8], fixed_iv: &[u8]) -> Result<Self, RecordError> {
        Self::new_at(alg, key, fixed_iv, 0)
    }

    /// Like [`Opener::new`], but resuming at an arbitrary sequence number.
    pub fn new_at(alg: Aead, key: &[u8], fixed_iv: &[u8], seq: u64) -> Result<Self, RecordError> {
        Ok(Self {
            key: DirectionKey::new(alg, key, fixed_iv)?,
            seq: Sequence::starting_at(seq),
        })
    }

    /// The sequence number the next record will use, or `None` if exhausted.
    pub fn sequence(&self) -> Option<u64> {
        self.seq.0
    }

    /// Unprotect exactly one whole record, header included.
    ///
    /// `record` must be one complete record and nothing more; splitting a
    /// stream into records happens above this layer. The content type is
    /// returned as written in the header and is authenticated by the tag. A
    /// type this layer does not know comes back as [`ContentType::Unknown`] for
    /// the caller to refuse.
    ///
    /// The sequence number advances only if this succeeds.
    pub fn open(&mut self, record: &[u8]) -> Result<Opened, RecordError> {
        if record.len() < HEADER_LEN {
            return Err(RecordError::Truncated {
                len: record.len(),
                min: HEADER_LEN,
            });
        }

        let typ = ContentType::from_u8(record[0]);
        let version = [record[1], record[2]];
        if version != RECORD_VERSION {
            return Err(RecordError::UnexpectedVersion(version));
        }

        let declared = usize::from(u16::from_be_bytes([record[3], record[4]]));
        let body = &record[HEADER_LEN..];
        if body.len() != declared {
            return Err(RecordError::LengthMismatch {
                declared,
                available: body.len(),
            });
        }
        if declared > MAX_CIPHERTEXT_LEN {
            return Err(RecordError::EncryptedFragmentTooLong { len: declared });
        }

        let overhead = overhead(self.key.alg);
        let Some(plaintext_len) = declared.checked_sub(overhead) else {
            return Err(RecordError::Truncated {
                len: declared,
                min: overhead,
            });
        };
        if plaintext_len > MAX_FRAGMENT_LEN {
            return Err(RecordError::FragmentTooLong {
                len: plaintext_len,
                max: MAX_FRAGMENT_LEN,
            });
        }

        let seq = self.seq.peek()?;
        let (nonce, sealed) = match self.key.alg {
            Aead::Aes128Gcm | Aead::Aes256Gcm => {
                let (explicit, sealed) = body.split_at(GCM_EXPLICIT_NONCE_LEN);
                (self.key.gcm_nonce(explicit), sealed)
            }
            Aead::ChaCha20Poly1305 => (self.key.chacha_nonce(seq), body),
        };
        let aad = additional_data(seq, typ, version, plaintext_len);

        let mut buf = sealed.to_vec();
        let opened_len = self
            .key
            .key
            .open_in_place(nonce, aead::Aad::from(aad), &mut buf)
            .map_err(|_| RecordError::Decrypt)?
            .len();
        buf.truncate(opened_len);
        debug_assert_eq!(buf.len(), plaintext_len);

        self.seq.advance(seq);
        Ok(Opened { typ, fragment: buf })
    }
}

impl fmt::Debug for Opener {
    /// Prints the sequence number and nothing else — never the key material.
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("Opener")
            .field("sequence", &self.seq.0)
            .finish_non_exhaustive()
    }
}
