//! The TLS 1.3 key schedule — stage 3a, RFC 8446 §7.1.
//!
//! Every key the protocol uses comes from here. The record layer
//! ([`super::record`]) takes a key and an IV and protects bytes; this module
//! is where those keys come from, and where the chain of HKDF extractions and
//! expansions that binds them to the handshake transcript lives.
//!
//! ```text
//!              0
//!              |
//!              v
//!    PSK ->  HKDF-Extract = Early Secret
//!              |
//!              +-> Derive-Secret(., "ext binder" | "res binder", "")
//!              |
//!              v
//!        Derive-Secret(., "derived", "")
//!              |
//!              v
//! (EC)DHE -> HKDF-Extract = Handshake Secret
//!              |
//!              +-> Derive-Secret(., "c hs traffic", ClientHello..ServerHello)
//!              +-> Derive-Secret(., "s hs traffic", ClientHello..ServerHello)
//!              |
//!              v
//!        Derive-Secret(., "derived", "")
//!              |
//!              v
//!     0 -> HKDF-Extract = Master Secret
//!              |
//!              +-> Derive-Secret(., "c ap traffic", ClientHello..server Finished)
//!              +-> Derive-Secret(., "s ap traffic", ClientHello..server Finished)
//!              +-> Derive-Secret(., "exp master",   ClientHello..server Finished)
//!              +-> Derive-Secret(., "res master",   ClientHello..client Finished)
//! ```
//!
//! # Why the transcript is in the middle of it
//!
//! The transcript hashes threaded through the right-hand side are what make
//! this a *key schedule* rather than a key derivation. Every traffic secret
//! is bound to the exact sequence of handshake messages that produced it, so
//! two peers that saw different handshakes cannot arrive at the same keys —
//! which is what stops an attacker rewriting a ClientHello to downgrade a
//! connection and having both sides carry on as if nothing happened.
//!
//! An implementation that derived the right bytes from the wrong transcript
//! would interoperate perfectly with itself and fail to detect exactly the
//! attack this construction exists to detect. That is why the tests check
//! against RFC 8448's published intermediate values rather than against a
//! round trip.
//!
//! # What is here, and what is not
//!
//! Here: `HKDF-Expand-Label`, `Derive-Secret`, the secret schedule above,
//! traffic key and IV derivation, `finished_key`, the Finished MAC, and the
//! key-update step.
//!
//! Not here: the handshake messages themselves, the transcript hash's
//! *accumulation* (a caller supplies a finished hash), the key exchange, and
//! anything that talks to a peer. Those are stages 3b and 3c. This module is
//! pure arithmetic over byte strings — it has no state beyond the secrets it
//! is handed, and no opinion about what a handshake should look like.
//!
//! # Primitives
//!
//! `ring`'s HMAC and digest, per ADR-0002 §6 — HKDF itself is implemented
//! here rather than taken from `ring::hkdf`, because `HKDF-Expand-Label`'s
//! label construction is the part TLS defines and the part worth
//! hand-rolling. The underlying HMAC is not.

use ring::hmac;

/// The hash a cipher suite's key schedule runs on.
///
/// TLS 1.3 pairs each AEAD with a hash, and the hash decides every secret's
/// length as well as the transcript's. It is a separate choice from the AEAD
/// in [`super::record::Aead`] because the record layer does not care which
/// hash produced its key.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
#[non_exhaustive]
pub enum Hash {
    /// SHA-256, as in `TLS_AES_128_GCM_SHA256` and
    /// `TLS_CHACHA20_POLY1305_SHA256`.
    Sha256,
    /// SHA-384, as in `TLS_AES_256_GCM_SHA384`.
    Sha384,
}

impl Hash {
    /// The digest length, which is also every secret's length.
    pub const fn len(self) -> usize {
        match self {
            Self::Sha256 => 32,
            Self::Sha384 => 48,
        }
    }

    /// Never empty; present because `len` without `is_empty` is a lint.
    pub const fn is_empty(self) -> bool {
        false
    }

    fn digest_algorithm(self) -> &'static ring::digest::Algorithm {
        match self {
            Self::Sha256 => &ring::digest::SHA256,
            Self::Sha384 => &ring::digest::SHA384,
        }
    }

    fn hmac_algorithm(self) -> hmac::Algorithm {
        match self {
            Self::Sha256 => hmac::HMAC_SHA256,
            Self::Sha384 => hmac::HMAC_SHA384,
        }
    }

    /// The hash of the empty string, which `Derive-Secret` uses whenever its
    /// `Messages` argument is `""`.
    pub fn empty_hash(self) -> Vec<u8> {
        ring::digest::digest(self.digest_algorithm(), b"")
            .as_ref()
            .to_vec()
    }

    /// Hash a message. Provided so a caller can build transcript hashes
    /// without reaching for a second digest implementation and risking a
    /// mismatch with the one the schedule uses.
    pub fn hash(self, message: &[u8]) -> Vec<u8> {
        ring::digest::digest(self.digest_algorithm(), message)
            .as_ref()
            .to_vec()
    }
}

/// `HKDF-Extract(salt, ikm)`, RFC 5869 §2.2.
///
/// HMAC with the salt as the key and the input keying material as the
/// message, which reads backwards until you remember that extract's job is to
/// concentrate entropy from the IKM rather than to authenticate it.
fn hkdf_extract(hash: Hash, salt: &[u8], ikm: &[u8]) -> Vec<u8> {
    let key = hmac::Key::new(hash.hmac_algorithm(), salt);
    hmac::sign(&key, ikm).as_ref().to_vec()
}

/// `HKDF-Expand(prk, info, length)`, RFC 5869 §2.3.
fn hkdf_expand(hash: Hash, prk: &[u8], info: &[u8], length: usize) -> Vec<u8> {
    let key = hmac::Key::new(hash.hmac_algorithm(), prk);
    let mut output = Vec::with_capacity(length);
    let mut previous: Vec<u8> = Vec::new();
    let mut counter: u8 = 1;

    while output.len() < length {
        let mut context = hmac::Context::with_key(&key);
        context.update(&previous);
        context.update(info);
        context.update(&[counter]);
        previous = context.sign().as_ref().to_vec();
        output.extend_from_slice(&previous);
        // RFC 5869 caps expansion at 255 blocks. Nothing TLS 1.3 derives
        // comes close, so this is a bound rather than a case to handle.
        counter = counter
            .checked_add(1)
            .expect("no TLS 1.3 derivation needs 255 HKDF blocks");
    }

    output.truncate(length);
    output
}

/// Build the `HkdfLabel` structure RFC 8446 §7.1 defines:
///
/// ```text
/// struct {
///     uint16 length;
///     opaque label<7..255> = "tls13 " + Label;
///     opaque context<0..255> = Context;
/// } HkdfLabel;
/// ```
///
/// The `"tls13 "` prefix is domain separation: it keeps a TLS 1.3 secret from
/// colliding with one derived by any other protocol that happens to use HKDF
/// with the same PRK and a similar label.
fn hkdf_label(length: usize, label: &str, context: &[u8]) -> Vec<u8> {
    let mut full_label = Vec::with_capacity(6 + label.len());
    full_label.extend_from_slice(b"tls13 ");
    full_label.extend_from_slice(label.as_bytes());

    debug_assert!(full_label.len() <= 255, "label fits its length prefix");
    debug_assert!(context.len() <= 255, "context fits its length prefix");

    let mut out = Vec::with_capacity(4 + full_label.len() + context.len());
    out.extend_from_slice(&(length as u16).to_be_bytes());
    out.push(full_label.len() as u8);
    out.extend_from_slice(&full_label);
    out.push(context.len() as u8);
    out.extend_from_slice(context);
    out
}

/// `HKDF-Expand-Label(secret, label, context, length)`, RFC 8446 §7.1.
///
/// Public because stage 3c needs it directly for things outside the secret
/// schedule — the record layer's per-key `key` and `iv` derivations are
/// `HKDF-Expand-Label` calls, and so is a key update.
pub fn expand_label(
    hash: Hash,
    secret: &[u8],
    label: &str,
    context: &[u8],
    length: usize,
) -> Vec<u8> {
    hkdf_expand(hash, secret, &hkdf_label(length, label, context), length)
}

/// `Derive-Secret(secret, label, messages)`, RFC 8446 §7.1.
///
/// Takes an already-computed transcript hash rather than the messages
/// themselves: a caller accumulating a transcript has the running hash
/// anyway, and re-hashing the whole transcript at every derivation would be
/// both slower and an opportunity to hash a different set of messages than
/// the one that was accumulated.
pub fn derive_secret(hash: Hash, secret: &[u8], label: &str, transcript_hash: &[u8]) -> Vec<u8> {
    expand_label(hash, secret, label, transcript_hash, hash.len())
}

/// The secret schedule, walked one extraction at a time.
///
/// Each method consumes the stage before it, so the sequence RFC 8446 §7.1
/// lays out is the only sequence this type permits: there is no way to reach
/// the master secret without going through the handshake secret, and no way
/// to derive handshake traffic secrets from the master secret by mistake.
#[derive(Clone, Debug)]
pub struct KeySchedule {
    hash: Hash,
    secret: Vec<u8>,
}

impl KeySchedule {
    /// Begin at the Early Secret, with no pre-shared key.
    ///
    /// `HKDF-Extract(0, 0)`: both the salt and the IKM are `Hash.length`
    /// zeroes. The result is a fixed value per hash — not a secret at all —
    /// which is exactly why a handshake without a PSK gains no security until
    /// the (EC)DHE input arrives.
    pub fn new(hash: Hash) -> Self {
        let zeroes = vec![0u8; hash.len()];
        Self {
            hash,
            secret: hkdf_extract(hash, &zeroes, &zeroes),
        }
    }

    /// Begin at the Early Secret with a pre-shared key as the IKM.
    pub fn new_with_psk(hash: Hash, psk: &[u8]) -> Self {
        let zeroes = vec![0u8; hash.len()];
        Self {
            hash,
            secret: hkdf_extract(hash, &zeroes, psk),
        }
    }

    /// The hash this schedule runs on.
    pub const fn hash(&self) -> Hash {
        self.hash
    }

    /// The current stage's secret.
    ///
    /// Exposed for the tests that check it against RFC 8448's published
    /// values, which is the only way to know the schedule is right rather
    /// than merely self-consistent.
    pub fn secret(&self) -> &[u8] {
        &self.secret
    }

    /// Advance to the Handshake Secret with the (EC)DHE shared secret.
    ///
    /// The `Derive-Secret(., "derived", "")` step in between is not optional
    /// decoration: extracting directly would let the new input be combined
    /// with a secret that has already been used for other derivations.
    pub fn into_handshake(self, shared_secret: &[u8]) -> Self {
        let derived = derive_secret(self.hash, &self.secret, "derived", &self.hash.empty_hash());
        Self {
            hash: self.hash,
            secret: hkdf_extract(self.hash, &derived, shared_secret),
        }
    }

    /// Advance to the Master Secret.
    ///
    /// The IKM is `Hash.length` zeroes: there is no new keying material at
    /// this point, and the step exists to separate the master secret from the
    /// handshake secret rather than to add entropy.
    pub fn into_master(self) -> Self {
        let derived = derive_secret(self.hash, &self.secret, "derived", &self.hash.empty_hash());
        let zeroes = vec![0u8; self.hash.len()];
        Self {
            hash: self.hash,
            secret: hkdf_extract(self.hash, &derived, &zeroes),
        }
    }

    /// `Derive-Secret(., label, transcript_hash)` from the current stage.
    ///
    /// The labels are RFC 8446's: `"c hs traffic"`, `"s hs traffic"`,
    /// `"c ap traffic"`, `"s ap traffic"`, `"exp master"`, `"res master"`.
    /// They are not an enum because the set is fixed by a spec that also
    /// defines extension points, and a caller getting a label wrong produces
    /// a secret that will not interoperate — which is loud, not silent.
    pub fn derive(&self, label: &str, transcript_hash: &[u8]) -> Vec<u8> {
        derive_secret(self.hash, &self.secret, label, transcript_hash)
    }
}

/// The binder key for a resumption PSK.
///
/// `Derive-Secret(Early-Secret(psk), "res binder", "")`. The early secret here
/// is extracted from the PSK rather than from zeroes, which is the one place
/// the key schedule's first step differs between a fresh handshake and a
/// resumed one.
///
/// `"res binder"` and not `"ext binder"`: the two labels separate a *resumption*
/// PSK from an externally provisioned one, so a key established one way cannot
/// be presented as if it had been established the other.
///
/// # What the tests here do and do not prove
///
/// The suite covers the *shape* of this function — deterministic, dependent on
/// both the PSK and the transcript, the right length for the hash, and not
/// simply the PSK passed through. It does **not** pin the value: swapping this
/// label for `"ext binder"` passes every one of them.
///
/// Nothing in this repo can check the value yet. That needs either RFC 8448's
/// resumption vectors or a handshake that actually resumes, and neither exists
/// here — see `rusty_tls#43`. Said plainly because five green tests beside a
/// derivation invite the reader to conclude the derivation is right.
pub fn binder_key(hash: Hash, psk: &[u8]) -> Vec<u8> {
    let early = KeySchedule::new_with_psk(hash, psk);
    derive_secret(hash, early.secret(), "res binder", &hash.empty_hash())
}

/// The binder for a `pre_shared_key` offer.
///
/// The same HMAC construction as a Finished, over the ClientHello **truncated
/// to just before the binders themselves** — a binder cannot cover itself. That
/// is why `pre_shared_key` must be the last extension in the hello: anything
/// after it would fall outside what the binder proves.
///
/// `truncated_transcript_hash` is the hash of everything up to that point,
/// which for a first-connection resumption is just the truncated hello and for
/// one after a HelloRetryRequest includes the earlier messages too.
pub fn psk_binder(hash: Hash, psk: &[u8], truncated_transcript_hash: &[u8]) -> Vec<u8> {
    let key = binder_key(hash, psk);
    finished_verify_data(hash, &key, truncated_transcript_hash)
}

/// The record-protection key and IV derived from one traffic secret.
///
/// RFC 8446 §7.3. Both are `HKDF-Expand-Label` from the same secret with
/// different labels, which is why they are produced together — deriving one
/// without the other is never useful.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct TrafficKeys {
    /// The AEAD key, of the length the AEAD requires.
    pub key: Vec<u8>,
    /// The static IV, [`super::record::NONCE_LEN`] bytes.
    pub iv: Vec<u8>,
}

/// Derive the record-protection key and IV from a traffic secret.
///
/// `key_len` comes from the AEAD rather than the hash — the two are chosen
/// together by a cipher suite but sized independently, and
/// `TLS_AES_128_GCM_SHA256` is exactly the case where they differ.
pub fn traffic_keys(hash: Hash, traffic_secret: &[u8], key_len: usize) -> TrafficKeys {
    TrafficKeys {
        key: expand_label(hash, traffic_secret, "key", b"", key_len),
        iv: expand_label(hash, traffic_secret, "iv", b"", super::record::NONCE_LEN),
    }
}

/// The key a Finished message's MAC is computed under, RFC 8446 §4.4.4.
pub fn finished_key(hash: Hash, traffic_secret: &[u8]) -> Vec<u8> {
    expand_label(hash, traffic_secret, "finished", b"", hash.len())
}

/// The `verify_data` for a Finished message: `HMAC(finished_key, transcript)`.
///
/// This is the handshake's own integrity check. It proves the peer holds the
/// handshake traffic secret *and* saw the same transcript, which is what makes
/// a modified ClientHello detectable rather than merely unlikely to work.
pub fn finished_verify_data(hash: Hash, traffic_secret: &[u8], transcript_hash: &[u8]) -> Vec<u8> {
    let key = hmac::Key::new(hash.hmac_algorithm(), &finished_key(hash, traffic_secret));
    hmac::sign(&key, transcript_hash).as_ref().to_vec()
}

/// Check a received Finished message's `verify_data`.
///
/// Uses `ring`'s constant-time verification rather than comparing slices. A
/// byte-by-byte comparison that returns early leaks how much of the MAC an
/// attacker got right, which turns forging one from impossible into a few
/// thousand guesses.
pub fn verify_finished(
    hash: Hash,
    traffic_secret: &[u8],
    transcript_hash: &[u8],
    verify_data: &[u8],
) -> bool {
    let key = hmac::Key::new(hash.hmac_algorithm(), &finished_key(hash, traffic_secret));
    hmac::verify(&key, transcript_hash, verify_data).is_ok()
}

/// The next application traffic secret after a KeyUpdate, RFC 8446 §7.2.
///
/// `application_traffic_secret_N+1 =
///  HKDF-Expand-Label(application_traffic_secret_N, "traffic upd", "",
///  Hash.length)`
///
/// One-way by construction: the new secret cannot be run backwards to recover
/// the old one, so a compromise after a key update does not expose what came
/// before it.
pub fn update_traffic_secret(hash: Hash, traffic_secret: &[u8]) -> Vec<u8> {
    expand_label(hash, traffic_secret, "traffic upd", b"", hash.len())
}
