//! The TLS 1.2 key derivation — stage 4b-ii, RFC 5246 §5, §6.3, §7.4.9 and
//! RFC 7627.
//!
//! The TLS 1.2 counterpart of [`super::schedule`]. TLS 1.3 chains HKDF
//! extractions; TLS 1.2 runs one pseudorandom function over everything:
//!
//! ```text
//! pre_master_secret   (the ECDHE shared secret)
//!        |
//!        | PRF(., "extended master secret", session_hash)[0..48]
//!        v
//! master_secret
//!        |-- PRF(., "key expansion", server_random + client_random)
//!        |       -> client_write_key, server_write_key,
//!        |          client_write_IV,  server_write_IV
//!        |
//!        `-- PRF(., "client finished" | "server finished", Hash(handshake))[0..12]
//! ```
//!
//! # Only the extended master secret
//!
//! RFC 7627 binds the master secret to the handshake transcript (`session_hash`)
//! instead of only to the two hello randoms. Without it, the triple handshake
//! attack lets an attacker splice sessions together, because two different
//! handshakes can share a master secret. There is deliberately no function here
//! that computes the original RFC 5246 master secret: a peer that does not
//! negotiate the extension is refused by the handshake (stage 4b-iii), and a
//! missing function cannot be called by mistake.
//!
//! # Three places the order is easy to get wrong
//!
//! - The key block's seed is `server_random + client_random`, the **reverse** of
//!   the order the master secret used before RFC 7627. Reading both as "client
//!   then server" derives keys the peer does not have.
//! - The session hash covers every handshake message up to and including
//!   `ClientKeyExchange`, as the *encoded* messages (headers included), and uses
//!   the cipher suite's PRF hash: SHA-256, or SHA-384 for the `*_SHA384` suites.
//! - `Finished` is truncated to 12 octets, unlike TLS 1.3's full-hash MAC.
//!
//! None of these is visible to a round trip between two copies of this code,
//! which is why the tests check against captured OpenSSL handshakes.
//!
//! # What is not here
//!
//! The key exchange (see [`super::kx`]), the handshake messages and the
//! transcript buffer, and anything that talks to a peer. The pre-master secret
//! is supplied by the caller; for ECDHE it is the shared secret exactly as
//! [`super::kx`] reports it, with leading zero octets kept (RFC 8422 §5.10).

use core::fmt;

use ring::hmac;

use super::record::Aead;
use super::record12::fixed_iv_len;
use super::schedule::Hash;

/// Length of a TLS 1.2 master secret, always 48 octets (RFC 5246 §8.1).
pub const MASTER_SECRET_LEN: usize = 48;

/// Length of a `Finished` message's `verify_data` in TLS 1.2 (RFC 5246 §7.4.9).
pub const VERIFY_DATA_LEN: usize = 12;

/// Length of a hello `random` (RFC 5246 §7.4.1.2).
pub const RANDOM_LEN: usize = 32;

/// Why a derivation refused its inputs.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
#[non_exhaustive]
pub enum ScheduleError {
    /// The pre-master secret was empty.
    EmptyPreMasterSecret,
    /// The session or handshake hash was not the length of the suite's hash.
    ///
    /// The caller hashes the transcript; handing over a hash from the wrong
    /// algorithm (SHA-256 for a SHA-384 suite) would otherwise be accepted and
    /// derive a secret the peer does not have.
    HashLength {
        /// The digest length the suite's PRF hash produces.
        expected: usize,
        /// What was supplied.
        actual: usize,
    },
}

impl fmt::Display for ScheduleError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::EmptyPreMasterSecret => f.write_str("the pre-master secret is empty"),
            Self::HashLength { expected, actual } => write!(
                f,
                "transcript hash is {actual} bytes, the suite's hash produces {expected}"
            ),
        }
    }
}

impl std::error::Error for ScheduleError {}

fn hmac_algorithm(hash: Hash) -> hmac::Algorithm {
    match hash {
        Hash::Sha256 => hmac::HMAC_SHA256,
        Hash::Sha384 => hmac::HMAC_SHA384,
    }
}

/// `PRF(secret, label, seed)`, RFC 5246 §5, writing `out.len()` octets.
///
/// ```text
/// PRF(secret, label, seed) = P_hash(secret, label + seed)
/// P_hash(secret, seed) = HMAC(secret, A(1) + seed) + HMAC(secret, A(2) + seed) + ...
/// A(0) = seed,  A(i) = HMAC(secret, A(i-1))
/// ```
///
/// `seed` is given as pieces and concatenated, so a caller never builds the
/// joined buffer (and cannot join it in the wrong order without writing the
/// order down at the call site). It is exposed so the tests can hold it to
/// OpenSSL's and rustls' implementations directly.
pub fn prf(hash: Hash, secret: &[u8], label: &[u8], seed: &[&[u8]], out: &mut [u8]) {
    let key = hmac::Key::new(hmac_algorithm(hash), secret);

    let mut a_ctx = hmac::Context::with_key(&key);
    a_ctx.update(label);
    seed.iter().for_each(|piece| a_ctx.update(piece));
    let mut a = a_ctx.sign(); // A(1)

    for chunk in out.chunks_mut(hash.len()) {
        let mut block = hmac::Context::with_key(&key);
        block.update(a.as_ref());
        block.update(label);
        seed.iter().for_each(|piece| block.update(piece));
        let block = block.sign();
        chunk.copy_from_slice(&block.as_ref()[..chunk.len()]);
        a = hmac::sign(&key, a.as_ref()); // A(i+1)
    }
}

fn check_hash_len(hash: Hash, value: &[u8]) -> Result<(), ScheduleError> {
    if value.len() == hash.len() {
        Ok(())
    } else {
        Err(ScheduleError::HashLength {
            expected: hash.len(),
            actual: value.len(),
        })
    }
}

/// A TLS 1.2 master secret.
///
/// Opaque on purpose: the only things to do with one are derive keys and verify
/// `Finished`, both of which take it by reference. `Debug` prints nothing of it.
pub struct MasterSecret([u8; MASTER_SECRET_LEN]);

impl MasterSecret {
    /// Wrap an existing master secret, for resuming a session from a ticket.
    pub fn from_bytes(bytes: [u8; MASTER_SECRET_LEN]) -> Self {
        Self(bytes)
    }

    /// The secret itself, for storing in a session ticket.
    pub fn as_bytes(&self) -> &[u8; MASTER_SECRET_LEN] {
        &self.0
    }
}

impl fmt::Debug for MasterSecret {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str("MasterSecret(<redacted>)")
    }
}

/// `master_secret = PRF(pre_master_secret, "extended master secret",
/// session_hash)[0..48]`, RFC 7627 §4.
///
/// `session_hash` is the suite's PRF hash of every encoded handshake message from
/// `ClientHello` up to and including `ClientKeyExchange`.
pub fn extended_master_secret(
    hash: Hash,
    pre_master_secret: &[u8],
    session_hash: &[u8],
) -> Result<MasterSecret, ScheduleError> {
    if pre_master_secret.is_empty() {
        return Err(ScheduleError::EmptyPreMasterSecret);
    }
    check_hash_len(hash, session_hash)?;

    let mut out = [0u8; MASTER_SECRET_LEN];
    prf(
        hash,
        pre_master_secret,
        b"extended master secret",
        &[session_hash],
        &mut out,
    );
    Ok(MasterSecret(out))
}

/// The keys and fixed IVs for both directions of a connection.
///
/// AEAD suites have no MAC keys, so the key block (RFC 5246 §6.3) is exactly
/// these four values in this order.
pub struct KeyBlock {
    /// Key the client encrypts with and the server decrypts with.
    pub client_write_key: Vec<u8>,
    /// Key the server encrypts with and the client decrypts with.
    pub server_write_key: Vec<u8>,
    /// Fixed IV for the client's direction: the 4-byte salt for AES-GCM, the
    /// 12-byte IV for ChaCha20-Poly1305 (see [`fixed_iv_len`]).
    pub client_write_iv: Vec<u8>,
    /// Fixed IV for the server's direction.
    pub server_write_iv: Vec<u8>,
}

impl fmt::Debug for KeyBlock {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str("KeyBlock(<redacted>)")
    }
}

/// `key_block = PRF(master_secret, "key expansion", server_random +
/// client_random)`, partitioned for `alg`, RFC 5246 §6.3.
///
/// Note the seed order: server random first.
pub fn key_block(
    hash: Hash,
    alg: Aead,
    master_secret: &MasterSecret,
    client_random: &[u8; RANDOM_LEN],
    server_random: &[u8; RANDOM_LEN],
) -> KeyBlock {
    let key_len = alg.key_len();
    let iv_len = fixed_iv_len(alg);

    let mut block = vec![0u8; 2 * key_len + 2 * iv_len];
    prf(
        hash,
        &master_secret.0,
        b"key expansion",
        &[server_random, client_random],
        &mut block,
    );

    let (client_write_key, rest) = block.split_at(key_len);
    let (server_write_key, rest) = rest.split_at(key_len);
    let (client_write_iv, server_write_iv) = rest.split_at(iv_len);
    KeyBlock {
        client_write_key: client_write_key.to_vec(),
        server_write_key: server_write_key.to_vec(),
        client_write_iv: client_write_iv.to_vec(),
        server_write_iv: server_write_iv.to_vec(),
    }
}

/// Which side's `Finished` is being computed.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Side {
    /// The client's `Finished`, `"client finished"`.
    Client,
    /// The server's `Finished`, `"server finished"`.
    Server,
}

impl Side {
    const fn label(self) -> &'static [u8] {
        match self {
            Self::Client => b"client finished",
            Self::Server => b"server finished",
        }
    }
}

/// `verify_data = PRF(master_secret, finished_label, Hash(handshake_messages))
/// [0..12]`, RFC 5246 §7.4.9.
///
/// `handshake_hash` is the suite's PRF hash of every encoded handshake message
/// so far. The client's covers up to and including `ClientKeyExchange` (and any
/// `CertificateVerify`); the server's also covers the client's `Finished`.
pub fn finished_verify_data(
    hash: Hash,
    master_secret: &MasterSecret,
    side: Side,
    handshake_hash: &[u8],
) -> Result<[u8; VERIFY_DATA_LEN], ScheduleError> {
    check_hash_len(hash, handshake_hash)?;
    let mut out = [0u8; VERIFY_DATA_LEN];
    prf(
        hash,
        &master_secret.0,
        side.label(),
        &[handshake_hash],
        &mut out,
    );
    Ok(out)
}

/// Check a received `Finished`'s `verify_data` in constant time.
///
/// Any wrong length, including a wrong hash length on the caller's side, is
/// simply `false`: there is nothing useful for a caller to do with the reason.
pub fn verify_finished(
    hash: Hash,
    master_secret: &MasterSecret,
    side: Side,
    handshake_hash: &[u8],
    received: &[u8],
) -> bool {
    let Ok(expected) = finished_verify_data(hash, master_secret, side, handshake_hash) else {
        return false;
    };
    // `hmac::verify` compares in constant time but wants a full-length tag, and
    // `verify_data` is truncated; `ring`'s own comparison helper is deprecated.
    // So compare MACs of the two values under a key only the verifier knows (the
    // expected value). They are equal exactly when the values are, and the final
    // comparison is ring's constant-time one. Only the length of `received`,
    // which is public, affects the time taken.
    let key = hmac::Key::new(hmac::HMAC_SHA256, &expected);
    let expected_tag = hmac::sign(&key, &expected);
    hmac::verify(&key, received, expected_tag.as_ref()).is_ok()
}
