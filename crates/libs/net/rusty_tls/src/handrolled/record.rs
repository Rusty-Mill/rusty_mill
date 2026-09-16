//! The TLS 1.3 record layer — AEAD protection and framing (RFC 8446 §5).
//!
//! This is stage 1 of the hand-rolled engine: it takes an *already
//! established* connection's traffic keys and turns fragments into wire
//! records and back. It knows nothing about handshakes, certificates, or key
//! schedules — a caller supplies the key and IV, and this module supplies
//! [`Sealer`] and [`Opener`].
//!
//! # The shape of a protected record
//!
//! ```text
//! outer:  17 03 03 <len:u16> <encrypted_record>       (RFC 8446 §5.2)
//! inner:  <content> <ContentType:u8> <zeros...>       (TLSInnerPlaintext)
//! ```
//!
//! The inner content type is *inside* the AEAD, which is the whole point of
//! the 1.3 record layer: an observer sees `application_data` on every record
//! regardless of what it actually carries. The trailing zero padding is
//! stripped on receipt by scanning backwards for the first non-zero octet,
//! which is that octet's content type.
//!
//! # Nonces and sequence numbers
//!
//! Per §5.3, the per-record nonce is the 64-bit sequence number, left-padded
//! to the IV length, XORed with the static IV. The sequence number is
//! implicit — never on the wire — starts at zero when a key is installed, and
//! [MUST NOT be allowed to wrap][seq]. [`Sealer`] and [`Opener`] own their own
//! counter and refuse to continue past `u64::MAX` rather than reusing a nonce.
//!
//! [seq]: https://www.rfc-editor.org/rfc/rfc8446#section-5.3
//!
//! # Two deliberate divergences from a literal reading of §5.2
//!
//! **The additional data is synthesized, not copied off the wire.** §5.2
//! defines `additional_data` as the record's own
//! `opaque_type || legacy_record_version || length`. This implementation
//! builds `17 03 03 <len>` from scratch instead of reusing the received three
//! header bytes. For any record a conforming TLS 1.3 peer can produce these
//! are identical, because §5.2 requires `opaque_type` to be
//! `application_data` and §5.1 requires `legacy_record_version` to be
//! `0x0303`. rustls does the same thing (`make_tls13_aad`), and matching it
//! is what lets the differential test assert byte-identical output.
//!
//! **The outer `opaque_type` is therefore checked explicitly.** Synthesizing
//! the AAD has a consequence worth being precise about: it leaves the outer
//! type byte *unauthenticated*. An attacker who flips `0x17` to `0x16` in
//! flight does not disturb the tag, because the tag was never computed over
//! the byte they changed. So [`Opener::open`] rejects any outer type other
//! than `application_data(23)` in the framing, before it decrypts. The
//! `legacy_record_version` bytes are *not* checked, because §5.1 says that
//! field "MUST be ignored for all purposes."

use core::fmt;

use ring::aead;

/// Length of the record header: type (1) + legacy version (2) + length (2).
pub const HEADER_LEN: usize = 5;

/// Length of the per-record AEAD nonce, and of the static IV it derives from.
///
/// Twelve for every AEAD TLS 1.3 defines.
pub const NONCE_LEN: usize = 12;

/// Length of the AEAD authentication tag, for every AEAD TLS 1.3 defines.
pub const TAG_LEN: usize = 16;

/// Maximum `TLSPlaintext.fragment` length — 2^14, per RFC 8446 §5.1.
pub const MAX_FRAGMENT_LEN: usize = 1 << 14;

/// Maximum `TLSCiphertext.encrypted_record` length — 2^14 + 256, per §5.2.
pub const MAX_ENCRYPTED_FRAGMENT_LEN: usize = MAX_FRAGMENT_LEN + 256;

/// Maximum `TLSInnerPlaintext` length, derived rather than quoted.
///
/// §5.2 bounds what goes on the wire (`MAX_ENCRYPTED_FRAGMENT_LEN`) and that
/// is inner plaintext plus tag, so the inner plaintext itself is bounded by
/// the difference. Deriving it from the wire limit avoids depending on a
/// reading of §5.4's padding language.
pub const MAX_INNER_PLAINTEXT_LEN: usize = MAX_ENCRYPTED_FRAGMENT_LEN - TAG_LEN;

/// The only outer type a TLS 1.3 protected record may carry (§5.2).
const OUTER_TYPE: u8 = 23;

/// `legacy_record_version`: TLS 1.2's version, even for TLS 1.3 (§5.1).
const LEGACY_RECORD_VERSION: [u8; 2] = [0x03, 0x03];

/// The AEAD algorithms TLS 1.3 defines, and this record layer implements.
///
/// The names are the cipher suites' AEAD halves; the hash half (which suite
/// names also encode) belongs to the key schedule, not here.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
#[non_exhaustive]
pub enum Aead {
    /// AES-128-GCM, as in `TLS_AES_128_GCM_SHA256`.
    Aes128Gcm,
    /// AES-256-GCM, as in `TLS_AES_256_GCM_SHA384`.
    Aes256Gcm,
    /// ChaCha20-Poly1305, as in `TLS_CHACHA20_POLY1305_SHA256`.
    ChaCha20Poly1305,
}

impl Aead {
    /// The key length this algorithm requires, in bytes.
    pub const fn key_len(self) -> usize {
        match self {
            Self::Aes128Gcm => 16,
            Self::Aes256Gcm | Self::ChaCha20Poly1305 => 32,
        }
    }

    /// The authentication tag length, in bytes. Sixteen for all three.
    pub const fn tag_len(self) -> usize {
        TAG_LEN
    }

    fn ring_algorithm(self) -> &'static aead::Algorithm {
        match self {
            Self::Aes128Gcm => &aead::AES_128_GCM,
            Self::Aes256Gcm => &aead::AES_256_GCM,
            Self::ChaCha20Poly1305 => &aead::CHACHA20_POLY1305,
        }
    }
}

/// A record's content type — the inner one, carried inside the AEAD.
///
/// `Unknown` exists because a peer may legitimately send a type this
/// implementation does not know; deciding what to do about that belongs to
/// the layer above, not to record decryption.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub enum ContentType {
    /// `change_cipher_spec(20)` — in TLS 1.3, only ever a no-op "middlebox
    /// compatibility" record.
    ChangeCipherSpec,
    /// `alert(21)`.
    Alert,
    /// `handshake(22)`.
    Handshake,
    /// `application_data(23)`.
    ApplicationData,
    /// Any other value, preserved verbatim.
    Unknown(u8),
}

impl ContentType {
    /// The wire encoding of this content type.
    pub const fn as_u8(self) -> u8 {
        match self {
            Self::ChangeCipherSpec => 20,
            Self::Alert => 21,
            Self::Handshake => 22,
            Self::ApplicationData => 23,
            Self::Unknown(other) => other,
        }
    }

    /// Decode a wire content type. Never fails — unrecognized values become
    /// [`ContentType::Unknown`].
    pub const fn from_u8(value: u8) -> Self {
        match value {
            20 => Self::ChangeCipherSpec,
            21 => Self::Alert,
            22 => Self::Handshake,
            23 => Self::ApplicationData,
            other => Self::Unknown(other),
        }
    }
}

/// Everything the record layer can refuse to do.
///
/// [`RecordError::Decrypt`] is deliberately opaque: it covers a bad tag, a
/// tampered header, the wrong key, and a record arriving out of order, and
/// distinguishing those for the caller would hand an attacker an oracle.
#[derive(Clone, Debug, PartialEq, Eq)]
#[non_exhaustive]
pub enum RecordError {
    /// The supplied key was the wrong length for the algorithm.
    KeyLength {
        /// What the algorithm requires.
        expected: usize,
        /// What was supplied.
        actual: usize,
    },
    /// The supplied IV was not [`NONCE_LEN`] bytes.
    IvLength {
        /// What was supplied.
        actual: usize,
    },
    /// A fragment exceeded [`MAX_FRAGMENT_LEN`], or padding pushed the inner
    /// plaintext past [`MAX_INNER_PLAINTEXT_LEN`].
    FragmentTooLong {
        /// The offending length.
        len: usize,
        /// The limit it broke.
        max: usize,
    },
    /// The buffer was too short to be a record at all.
    Truncated {
        /// Bytes available.
        len: usize,
        /// Bytes needed.
        min: usize,
    },
    /// The header's declared length did not match the bytes supplied.
    ///
    /// [`Opener::open`] takes exactly one whole record; reassembling a stream
    /// into records is the caller's job.
    LengthMismatch {
        /// What the header declared.
        declared: usize,
        /// What was actually available after the header.
        available: usize,
    },
    /// `encrypted_record` exceeded [`MAX_ENCRYPTED_FRAGMENT_LEN`].
    EncryptedFragmentTooLong {
        /// The offending length.
        len: usize,
    },
    /// The outer record type was not `application_data(23)`.
    ///
    /// See this module's header on why this is checked rather than left to
    /// the AEAD.
    UnexpectedOuterType(u8),
    /// The record did not authenticate, or did not decrypt.
    Decrypt,
    /// The AEAD refused to encrypt. Not reachable through this API's own
    /// length checks; present so a `ring` failure is never silently ignored.
    Encrypt,
    /// The inner plaintext was entirely zeros, so it carries no content type.
    ///
    /// RFC 8446 §5.4 makes this an `unexpected_message` alert, not a
    /// zero-length `application_data` record.
    NoContentType,
    /// The sequence number reached `u64::MAX` and this key may not be used
    /// again (§5.3 forbids wrapping).
    SequenceExhausted,
}

impl fmt::Display for RecordError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::KeyLength { expected, actual } => {
                write!(f, "key is {actual} bytes, algorithm needs {expected}")
            }
            Self::IvLength { actual } => {
                write!(f, "iv is {actual} bytes, needs {NONCE_LEN}")
            }
            Self::FragmentTooLong { len, max } => {
                write!(f, "fragment of {len} bytes exceeds the {max}-byte limit")
            }
            Self::Truncated { len, min } => {
                write!(f, "record is {len} bytes, needs at least {min}")
            }
            Self::LengthMismatch {
                declared,
                available,
            } => write!(
                f,
                "record header declares {declared} bytes, {available} available"
            ),
            Self::EncryptedFragmentTooLong { len } => write!(
                f,
                "encrypted record of {len} bytes exceeds the \
                 {MAX_ENCRYPTED_FRAGMENT_LEN}-byte limit"
            ),
            Self::UnexpectedOuterType(typ) => {
                write!(f, "outer record type {typ} is not application_data(23)")
            }
            Self::Decrypt => f.write_str("record failed to decrypt or authenticate"),
            Self::Encrypt => f.write_str("record failed to encrypt"),
            Self::NoContentType => {
                f.write_str("inner plaintext is all zeros and carries no content type")
            }
            Self::SequenceExhausted => {
                f.write_str("record sequence number exhausted; this key may not be used again")
            }
        }
    }
}

impl std::error::Error for RecordError {}

/// The AEAD key and static IV for one direction of one connection.
///
/// One of these protects records in a single direction under a single set of
/// traffic secrets. A key update produces a new one, with its own sequence
/// number starting again at zero.
struct DirectionKey {
    key: aead::LessSafeKey,
    iv: [u8; NONCE_LEN],
}

impl DirectionKey {
    fn new(alg: Aead, key: &[u8], iv: &[u8]) -> Result<Self, RecordError> {
        if key.len() != alg.key_len() {
            return Err(RecordError::KeyLength {
                expected: alg.key_len(),
                actual: key.len(),
            });
        }
        if iv.len() != NONCE_LEN {
            return Err(RecordError::IvLength { actual: iv.len() });
        }

        // Both lengths are checked immediately above, which is the only way
        // `UnboundKey::new` can fail.
        let unbound = aead::UnboundKey::new(alg.ring_algorithm(), key).map_err(|_| {
            RecordError::KeyLength {
                expected: alg.key_len(),
                actual: key.len(),
            }
        })?;

        let mut fixed_iv = [0u8; NONCE_LEN];
        fixed_iv.copy_from_slice(iv);

        Ok(Self {
            key: aead::LessSafeKey::new(unbound),
            iv: fixed_iv,
        })
    }

    /// RFC 8446 §5.3: left-pad the sequence number to the IV length, then XOR.
    fn nonce(&self, seq: u64) -> aead::Nonce {
        let mut nonce = self.iv;
        for (out, byte) in nonce[NONCE_LEN - 8..].iter_mut().zip(seq.to_be_bytes()) {
            *out ^= byte;
        }
        // Unique by construction: `seq` never repeats for a given key (the
        // sequence counter is monotonic and refuses to wrap), and the IV is
        // fixed for the key's lifetime.
        aead::Nonce::assume_unique_for_key(nonce)
    }
}

/// The record header used as AEAD additional data.
///
/// Synthesized rather than copied off the wire — see this module's header.
fn additional_data(encrypted_len: usize) -> [u8; HEADER_LEN] {
    [
        OUTER_TYPE,
        LEGACY_RECORD_VERSION[0],
        LEGACY_RECORD_VERSION[1],
        (encrypted_len >> 8) as u8,
        encrypted_len as u8,
    ]
}

/// Hands out sequence numbers, and stops at `u64::MAX` instead of wrapping.
///
/// `None` means exhausted. The counter only advances after an operation
/// succeeds, so a rejected record does not consume a sequence number.
#[derive(Debug)]
struct Sequence(Option<u64>);

impl Sequence {
    const fn starting_at(seq: u64) -> Self {
        Self(Some(seq))
    }

    fn peek(&self) -> Result<u64, RecordError> {
        self.0.ok_or(RecordError::SequenceExhausted)
    }

    fn advance(&mut self, used: u64) {
        self.0 = used.checked_add(1);
    }
}

/// Protects outgoing records under one direction's traffic key.
///
/// ```
/// use rusty_tls::handrolled::record::{Aead, ContentType, Sealer};
///
/// let mut sealer = Sealer::new(Aead::Aes128Gcm, &[0u8; 16], &[0u8; 12])?;
/// let record = sealer.seal(ContentType::ApplicationData, b"hello", 0)?;
/// assert_eq!(&record[..3], &[0x17, 0x03, 0x03]);
/// assert_eq!(sealer.sequence(), Some(1));
/// # Ok::<(), rusty_tls::handrolled::record::RecordError>(())
/// ```
pub struct Sealer {
    key: DirectionKey,
    seq: Sequence,
}

impl Sealer {
    /// Build a sealer from an algorithm, a key of that algorithm's length,
    /// and a [`NONCE_LEN`]-byte static IV.
    ///
    /// The sequence number starts at zero, which is what installing a fresh
    /// traffic key means. To pick up mid-stream, use [`Sealer::new_at`].
    pub fn new(alg: Aead, key: &[u8], iv: &[u8]) -> Result<Self, RecordError> {
        Self::new_at(alg, key, iv, 0)
    }

    /// Like [`Sealer::new`], but resuming at an arbitrary sequence number.
    ///
    /// This exists because a key and a sequence number travel together: to
    /// hand an established connection's protection state to something else
    /// — the way rustls' own `ExtractedSecrets` reports `(u64,
    /// ConnectionTrafficSecrets)` per direction for kTLS offload — you need
    /// both halves, and starting over at zero would reuse nonces.
    pub fn new_at(alg: Aead, key: &[u8], iv: &[u8], seq: u64) -> Result<Self, RecordError> {
        Ok(Self {
            key: DirectionKey::new(alg, key, iv)?,
            seq: Sequence::starting_at(seq),
        })
    }

    /// The sequence number the next record will use, or `None` if exhausted.
    pub fn sequence(&self) -> Option<u64> {
        self.seq.0
    }

    /// Protect one fragment, returning the complete record including header.
    ///
    /// `padding` is how many zero octets to append after the content type
    /// (RFC 8446 §5.4). Zero is the ordinary choice; non-zero hides the true
    /// length of the fragment from an observer, at the cost of bandwidth.
    ///
    /// The sequence number advances only if this succeeds.
    pub fn seal(
        &mut self,
        typ: ContentType,
        fragment: &[u8],
        padding: usize,
    ) -> Result<Vec<u8>, RecordError> {
        if fragment.len() > MAX_FRAGMENT_LEN {
            return Err(RecordError::FragmentTooLong {
                len: fragment.len(),
                max: MAX_FRAGMENT_LEN,
            });
        }

        // content || type || zeros
        let inner_len = fragment.len().saturating_add(1).saturating_add(padding);
        if inner_len > MAX_INNER_PLAINTEXT_LEN {
            return Err(RecordError::FragmentTooLong {
                len: inner_len,
                max: MAX_INNER_PLAINTEXT_LEN,
            });
        }

        let seq = self.seq.peek()?;
        let encrypted_len = inner_len + TAG_LEN;

        let mut record = Vec::with_capacity(HEADER_LEN + encrypted_len);
        record.extend_from_slice(&additional_data(encrypted_len));

        let mut inner = Vec::with_capacity(inner_len);
        inner.extend_from_slice(fragment);
        inner.push(typ.as_u8());
        inner.resize(inner_len, 0);

        self.key
            .key
            .seal_in_place_append_tag(
                self.key.nonce(seq),
                aead::Aad::from(additional_data(encrypted_len)),
                &mut inner,
            )
            .map_err(|_| RecordError::Encrypt)?;

        debug_assert_eq!(inner.len(), encrypted_len);
        record.extend_from_slice(&inner);

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

/// One successfully opened record.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Opened {
    /// The inner content type, recovered from inside the AEAD.
    pub typ: ContentType,
    /// The fragment, with the content type and any padding removed.
    pub fragment: Vec<u8>,
}

/// Unprotects incoming records under one direction's traffic key.
pub struct Opener {
    key: DirectionKey,
    seq: Sequence,
}

impl Opener {
    /// Build an opener from an algorithm, a key of that algorithm's length,
    /// and a [`NONCE_LEN`]-byte static IV.
    ///
    /// The sequence number starts at zero, which is what installing a fresh
    /// traffic key means. To pick up mid-stream, use [`Opener::new_at`].
    pub fn new(alg: Aead, key: &[u8], iv: &[u8]) -> Result<Self, RecordError> {
        Self::new_at(alg, key, iv, 0)
    }

    /// Like [`Opener::new`], but resuming at an arbitrary sequence number.
    ///
    /// See [`Sealer::new_at`] for why a key and a sequence number travel
    /// together.
    pub fn new_at(alg: Aead, key: &[u8], iv: &[u8], seq: u64) -> Result<Self, RecordError> {
        Ok(Self {
            key: DirectionKey::new(alg, key, iv)?,
            seq: Sequence::starting_at(seq),
        })
    }

    /// The sequence number the next record will use, or `None` if exhausted.
    pub fn sequence(&self) -> Option<u64> {
        self.seq.0
    }

    /// Unprotect exactly one whole record, header included.
    ///
    /// `record` must be one complete record and nothing more — splitting a
    /// byte stream into records happens above this layer, where the header's
    /// length field is read to find each boundary.
    ///
    /// The sequence number advances only if this succeeds, so a rejected
    /// record can be retried or reported without desynchronizing a caller
    /// that chooses to continue. (TLS itself does not: §5.2 makes a
    /// decryption failure fatal to the connection.)
    pub fn open(&mut self, record: &[u8]) -> Result<Opened, RecordError> {
        if record.len() < HEADER_LEN {
            return Err(RecordError::Truncated {
                len: record.len(),
                min: HEADER_LEN,
            });
        }

        if record[0] != OUTER_TYPE {
            return Err(RecordError::UnexpectedOuterType(record[0]));
        }
        // record[1..3] is `legacy_record_version`, which §5.1 says MUST be
        // ignored for all purposes. So it is.

        let declared = usize::from(u16::from_be_bytes([record[3], record[4]]));
        let body = &record[HEADER_LEN..];
        if body.len() != declared {
            return Err(RecordError::LengthMismatch {
                declared,
                available: body.len(),
            });
        }

        if declared > MAX_ENCRYPTED_FRAGMENT_LEN {
            return Err(RecordError::EncryptedFragmentTooLong { len: declared });
        }
        // An inner plaintext needs at least the content type octet, so the
        // shortest legal encrypted_record is one byte plus a tag.
        if declared < TAG_LEN + 1 {
            return Err(RecordError::Truncated {
                len: declared,
                min: TAG_LEN + 1,
            });
        }

        let seq = self.seq.peek()?;

        let mut buf = body.to_vec();
        let plain_len = self
            .key
            .key
            .open_in_place(
                self.key.nonce(seq),
                aead::Aad::from(additional_data(declared)),
                &mut buf,
            )
            .map_err(|_| RecordError::Decrypt)?
            .len();
        buf.truncate(plain_len);

        // §5.2: scan back past the zero padding; the first non-zero octet
        // from the end is the content type. Its index is therefore also the
        // length of the content that precedes it.
        let type_pos = match buf.iter().rposition(|&b| b != 0) {
            Some(pos) => pos,
            None => return Err(RecordError::NoContentType),
        };
        let typ = ContentType::from_u8(buf[type_pos]);
        buf.truncate(type_pos);

        if buf.len() > MAX_FRAGMENT_LEN {
            return Err(RecordError::FragmentTooLong {
                len: buf.len(),
                max: MAX_FRAGMENT_LEN,
            });
        }

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
