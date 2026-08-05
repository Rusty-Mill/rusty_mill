//! Session tickets, and the key that seals them — stage 5, `rusty_tls#43`.
//!
//! A TLS 1.3 server that resumes has to remember something across
//! connections. RFC 8446 §4.6.1 gives it two ways to do that: keep a table
//! indexed by an opaque handle, or hand the client an *encrypted copy of the
//! state* and let the client bring it back. This module is the second, which
//! is what "stateless resumption" means and why it needs a key.
//!
//! # This key is as sensitive as the certificate's private key
//!
//! ADR-0003 says so, and it is worth restating where the code is rather than
//! only in the ADR, because the two keys do not *look* alike and the
//! consequence of losing them is the same.
//!
//! A ticket is a sealed copy of a resumption PSK. Anyone who can open a ticket
//! learns that PSK, and a PSK authenticates a handshake as the continuation of
//! a session that was authenticated by the certificate. So: **an attacker with
//! this key can impersonate this server to any client holding a ticket**,
//! without ever touching the certificate's private key. An attacker who can
//! *forge* a ticket — which the same key allows — can impersonate the server to
//! a client that holds no ticket at all, by minting one.
//!
//! Three consequences follow, and none of them is this crate's to enforce:
//!
//! - **Rotate it.** A ticket key that never changes makes every ticket ever
//!   issued decryptable forever, which throws away the forward secrecy the
//!   `psk_dhe_ke`-only decision in ADR-0003 exists to preserve. Rotating means
//!   accepting tickets under the previous key for as long as one may still be
//!   in flight; [`TicketKeys`] is the shape that permits that.
//! - **Do not share it further than the certificate.** Two servers sharing a
//!   ticket key can resume each other's sessions. That is the point in a load
//!   balanced deployment and a hole anywhere else.
//! - **Store it the way a private key is stored.** Not in a config file that
//!   is checked in, not in an environment variable that ends up in a crash
//!   dump.
//!
//! This module can make none of those true. It can refuse to make them hard,
//! which is why the key is a distinct type that says nothing in `Debug` and
//! has no accessor for its bytes.
//!
//! # What is sealed
//!
//! The PSK, the cipher suite it belongs to, when it was issued and for how
//! long, a digest of the certificate chain the issuing server was presenting,
//! the `ticket_age_add` that connection chose, and the chain the *client*
//! presented if it presented one.
//!
//! Two of those are worth a sentence each, because they are what a ticket
//! carrying only a key cannot do:
//!
//! - **The server's chain digest** stops a ticket minted by one identity being
//!   redeemed against another that happens to share a ticket key. The sealing
//!   key alone does not bind a ticket to a certificate, and a deployment that
//!   reuses a key across two configurations would otherwise let a session
//!   authenticated by one chain be continued under the other.
//! - **The client's chain** is what makes resumption and client authentication
//!   able to coexist. A resumed handshake carries no Certificate from the
//!   client, so a server that stored nothing would report an authenticated
//!   client as anonymous — and
//!   [`ClientAuth::required`](super::server::ClientAuth::required) would go
//!   quietly unenforced on exactly the connections a resuming client makes
//!   most of. Storing it lets the server re-validate that chain against the
//!   clock at *resumption* time rather than at issuance time.
//!
//! What is deliberately **not** re-established is proof of possession. The
//! client proved it held that certificate's private key on the connection the
//! ticket came from, and the binder proves it holds the key derived from that
//! connection. A second signature would prove nothing the binder has not.
//!
//! # Primitives
//!
//! AES-256-GCM from `ring`, per ADR-0002 §6: the record layer's own AEADs come
//! from there for the same reason, and a ticket is exactly the kind of thing
//! that wants an authenticated cipher rather than a hand-assembled
//! encrypt-then-MAC.

use ring::aead::{Aad, LessSafeKey, Nonce, UnboundKey, AES_256_GCM, NONCE_LEN};
use ring::rand::{SecureRandom, SystemRandom};

use super::client::CipherSuite;
use super::schedule::Hash;
use super::wire::{Reader, WireError, Writer};

/// Everything ticket sealing and opening can refuse.
#[derive(Clone, Debug, PartialEq, Eq)]
#[non_exhaustive]
pub enum TicketError {
    /// A key was not [`TicketKey::LEN`] octets.
    BadKeyLength(usize),
    /// The system random source failed.
    Random,
    /// Sealing failed, which in practice means the plaintext was absurd.
    Seal,
    /// A ticket did not open: it was truncated, tampered with, or sealed under
    /// a different key.
    ///
    /// Deliberately one variant. A server that told a client which of those it
    /// was would be answering questions about its own key schedule for anyone
    /// who asked.
    Open,
    /// A ticket opened and its contents were not a ticket.
    Malformed(&'static str),
}

impl From<WireError> for TicketError {
    fn from(_: WireError) -> Self {
        Self::Malformed("the sealed contents are not a ticket")
    }
}

impl core::fmt::Display for TicketError {
    fn fmt(&self, f: &mut core::fmt::Formatter<'_>) -> core::fmt::Result {
        match self {
            Self::BadKeyLength(len) => {
                write!(f, "a ticket key is {} octets, got {len}", TicketKey::LEN)
            }
            Self::Random => f.write_str("the system random source failed"),
            Self::Seal => f.write_str("sealing a ticket failed"),
            Self::Open => f.write_str("a ticket did not open"),
            Self::Malformed(why) => write!(f, "a ticket opened but {why}"),
        }
    }
}

impl std::error::Error for TicketError {}

/// The associated data every ticket is sealed with.
///
/// It is not secret and it is not there to hide anything. It is domain
/// separation: a blob sealed by something else under the same key will not
/// open as a ticket, so a deployment that reuses a key by mistake gets a
/// refusal rather than a confusion.
const TICKET_AAD: &[u8] = b"rusty_tls resumption ticket v1";

/// The version of the sealed layout this code writes.
///
/// Bumped from 1 when tickets grew `ticket_age_add` and the client's chain. A
/// ticket at any other version is *ignored* rather than refused — see
/// [`TicketContents::open`] — because a version bump is a rotation of a
/// different kind, and a server that aborted on its own predecessor's tickets
/// would turn every upgrade into an outage for whoever was mid-session.
const TICKET_VERSION: u8 = 2;

/// A key a server seals resumption tickets with.
///
/// Read the module docs before deploying one. The short version: this is as
/// sensitive as the certificate's private key, and it needs rotating.
pub struct TicketKey {
    key: LessSafeKey,
}

impl TicketKey {
    /// The key length, in octets. AES-256-GCM.
    pub const LEN: usize = 32;

    /// A key from caller-supplied bytes.
    ///
    /// The bytes must come from a cryptographic random source, and they must
    /// be [`Self::LEN`] octets — not "at least", because a shorter key padded
    /// or a longer one truncated would be a key nobody chose.
    pub fn new(secret: &[u8]) -> Result<Self, TicketError> {
        if secret.len() != Self::LEN {
            return Err(TicketError::BadKeyLength(secret.len()));
        }
        let key = UnboundKey::new(&AES_256_GCM, secret).map_err(|_| TicketError::Seal)?;
        Ok(Self {
            key: LessSafeKey::new(key),
        })
    }

    /// A fresh key from the system random source.
    ///
    /// Convenient and *not* what a real deployment wants on its own: a key
    /// generated at start-up is a key every restart invalidates, so every
    /// ticket outstanding at that moment stops working. Fine for a test, fine
    /// for a single process that does not care, and not a substitute for
    /// storing one.
    pub fn generate() -> Result<Self, TicketError> {
        let mut secret = [0u8; Self::LEN];
        SystemRandom::new()
            .fill(&mut secret)
            .map_err(|_| TicketError::Random)?;
        Self::new(&secret)
    }

    /// Seal a ticket's contents, returning `nonce || ciphertext || tag`.
    ///
    /// The nonce is fresh per ticket and carried in the clear, because the
    /// receiver has no other way to learn it. A ticket key that sealed two
    /// tickets under one nonce would leak the relationship between the two
    /// PSKs, which is the failure mode GCM is least forgiving about.
    fn seal(&self, plaintext: &[u8]) -> Result<Vec<u8>, TicketError> {
        let mut nonce = [0u8; NONCE_LEN];
        SystemRandom::new()
            .fill(&mut nonce)
            .map_err(|_| TicketError::Random)?;

        let mut out = plaintext.to_vec();
        self.key
            .seal_in_place_append_tag(
                Nonce::assume_unique_for_key(nonce),
                Aad::from(TICKET_AAD),
                &mut out,
            )
            .map_err(|_| TicketError::Seal)?;

        let mut ticket = Vec::with_capacity(NONCE_LEN + out.len());
        ticket.extend_from_slice(&nonce);
        ticket.extend_from_slice(&out);
        Ok(ticket)
    }

    /// Open a ticket sealed by [`Self::seal`].
    fn open(&self, ticket: &[u8]) -> Result<Vec<u8>, TicketError> {
        let (nonce, sealed) = ticket
            .split_at_checked(NONCE_LEN)
            .ok_or(TicketError::Open)?;
        let nonce: [u8; NONCE_LEN] = nonce.try_into().expect("split at exactly NONCE_LEN");

        let mut buffer = sealed.to_vec();
        let opened = self
            .key
            .open_in_place(
                Nonce::assume_unique_for_key(nonce),
                Aad::from(TICKET_AAD),
                &mut buffer,
            )
            .map_err(|_| TicketError::Open)?;
        Ok(opened.to_vec())
    }
}

/// Says nothing about the key.
impl core::fmt::Debug for TicketKey {
    fn fmt(&self, f: &mut core::fmt::Formatter<'_>) -> core::fmt::Result {
        f.debug_struct("TicketKey")
            .field("key", &"<redacted>")
            .finish()
    }
}

/// The key a server seals new tickets with, plus keys it will still open.
///
/// Rotation needs both halves. A server that swapped its key would refuse
/// every ticket outstanding at that moment, so the old key has to stay
/// openable for at least as long as a ticket's lifetime — and it must stop
/// being used for *sealing* immediately, or rotation has not happened.
/// Separating the two is what makes that expressible rather than a comment
/// somebody has to honour.
pub struct TicketKeys<'a> {
    /// The key new tickets are sealed with.
    pub current: &'a TicketKey,
    /// Keys no longer used for sealing whose tickets are still accepted,
    /// most recently retired first.
    ///
    /// Empty is the un-rotated case and is fine to start with. Leaving a key
    /// here forever is not rotation; it is two current keys.
    pub previous: &'a [&'a TicketKey],
}

impl TicketKeys<'_> {
    /// Try `current`, then each of `previous`, and return the first that
    /// opens.
    fn open(&self, ticket: &[u8]) -> Option<Vec<u8>> {
        core::iter::once(self.current)
            .chain(self.previous.iter().copied())
            .find_map(|key| key.open(ticket).ok())
    }
}

/// What a ticket carries, once opened.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct TicketContents {
    /// The cipher suite the session ran under. A PSK is bound to that suite's
    /// hash and means nothing under another.
    pub suite: CipherSuite,
    /// When the ticket was issued, in seconds since the Unix epoch, as the
    /// issuing server reckoned it.
    pub issued_at: i64,
    /// How long after `issued_at` the ticket may be redeemed, in seconds.
    pub lifetime: u32,
    /// The `ticket_age_add` sent in this ticket's NewSessionTicket.
    ///
    /// Kept because it is the *only* way to read the age a client reports:
    /// §4.2.11's `obfuscated_ticket_age` is the real age plus this, and a
    /// server that did not remember the addend could not subtract it. Without
    /// it the field is unreadable, which is how it went unchecked until
    /// `rusty_tls#43` was finished.
    pub age_add: u32,
    /// SHA-256 over the issuing server's certificate chain — see the module
    /// docs on why a ticket is bound to an identity and not only to a key.
    pub identity: Vec<u8>,
    /// The chain the client presented, DER-encoded, empty if it presented
    /// none.
    ///
    /// See the module docs: this is what lets a resumed connection report the
    /// client it was resuming rather than reporting none.
    pub client_certificates: Vec<Vec<u8>>,
    /// The resumption PSK.
    pub psk: Vec<u8>,
}

impl TicketContents {
    /// The digest a chain is bound by.
    ///
    /// SHA-256 whatever the cipher suite's hash is: this is a binding, not a
    /// key derivation, and pinning one algorithm means a ticket issued under a
    /// SHA-384 suite is still comparable with one issued under SHA-256.
    pub fn identity_of(certificates: &[Vec<u8>]) -> Vec<u8> {
        let mut all = Vec::new();
        for certificate in certificates {
            all.extend_from_slice(&(certificate.len() as u32).to_be_bytes());
            all.extend_from_slice(certificate);
        }
        Hash::Sha256.hash(&all)
    }

    fn encode(&self) -> Vec<u8> {
        let mut writer = Writer::new();
        writer.u8(TICKET_VERSION);
        writer.u16(self.suite.0);
        writer.u32((self.issued_at >> 32) as u32);
        writer.u32(self.issued_at as u32);
        writer.u32(self.lifetime);
        writer.u32(self.age_add);
        writer.vector_u8(|w| w.bytes(&self.identity));
        writer.vector_u16(|w| {
            for certificate in &self.client_certificates {
                w.vector_u16(|w| w.bytes(certificate));
            }
        });
        writer.vector_u8(|w| w.bytes(&self.psk));
        writer.into_vec()
    }

    /// `Ok(None)` for a layout this code does not write — see
    /// [`TICKET_VERSION`].
    fn decode(plaintext: &[u8]) -> Result<Option<Self>, TicketError> {
        let mut reader = Reader::new(plaintext);
        if reader.u8()? != TICKET_VERSION {
            return Ok(None);
        }
        let suite = CipherSuite(reader.u16()?);
        let high = i64::from(reader.u32()?);
        let low = i64::from(reader.u32()?);
        let issued_at = (high << 32) | low;
        let lifetime = reader.u32()?;
        let age_add = reader.u32()?;
        let identity = reader.vector_u8()?.to_vec();
        let mut chain = Reader::new(reader.vector_u16()?);
        let mut client_certificates = Vec::new();
        while !chain.is_empty() {
            client_certificates.push(chain.vector_u16()?.to_vec());
        }
        let psk = reader.vector_u8()?.to_vec();
        reader.finish()?;

        if psk.is_empty() {
            return Err(TicketError::Malformed("it carries no key"));
        }
        Ok(Some(Self {
            suite,
            issued_at,
            lifetime,
            age_add,
            identity,
            client_certificates,
            psk,
        }))
    }

    /// Seal these contents under `key`.
    pub fn seal(&self, key: &TicketKey) -> Result<Vec<u8>, TicketError> {
        key.seal(&self.encode())
    }

    /// Open a ticket under any of `keys`, returning `None` if none of them
    /// opens it *or* if it opens and carries a layout this code does not
    /// write.
    ///
    /// Neither is an error at the protocol level: RFC 8446 §4.2.11 lets a
    /// server ignore an identity it does not recognise and fall back to a full
    /// handshake, and a server that aborted instead would break every client
    /// whose ticket outlived a key rotation — or a deployment's own upgrade.
    /// A ticket that opens *at this version* and then turns out not to be a
    /// ticket is a different matter, and comes back as `Some(Err(_))`: this
    /// server sealed something it cannot read, which is its own bug rather
    /// than a client's doing.
    pub fn open(ticket: &[u8], keys: &TicketKeys<'_>) -> Option<Result<Self, TicketError>> {
        match keys.open(ticket).map(|plaintext| Self::decode(&plaintext)) {
            Some(Ok(Some(contents))) => Some(Ok(contents)),
            Some(Ok(None)) | None => None,
            Some(Err(err)) => Some(Err(err)),
        }
    }

    /// The client's real ticket age, in milliseconds, from what it reported.
    ///
    /// §4.2.11: `obfuscated_ticket_age` is the age plus the `ticket_age_add`
    /// this ticket was issued with, modulo 2³². Wrapping subtraction is the
    /// inverse, and it is exact — the obfuscation loses nothing.
    pub const fn reported_age_ms(&self, obfuscated_ticket_age: u32) -> u32 {
        obfuscated_ticket_age.wrapping_sub(self.age_add)
    }

    /// Whether the age a client reports is consistent with when this server
    /// issued the ticket, to within `skew_ms`.
    ///
    /// # This is a sanity bound, not anti-replay
    ///
    /// Worth being blunt, because the field's *purpose* invites the stronger
    /// reading. `obfuscated_ticket_age` exists so a server doing 0-RTT can
    /// bound how long a replayed early-data flight stays useful. ADR-0003 puts
    /// 0-RTT out of scope, and no window checked here makes a 1-RTT
    /// resumption replay-proof — a resumed handshake completes a fresh key
    /// exchange, so replaying one gets an attacker a connection it cannot
    /// read.
    ///
    /// What it does buy is smaller and real: a client whose reported age
    /// disagrees with this server's own record is not the client this ticket
    /// was issued to, or is not measuring what it claims to be measuring.
    /// Declining to resume for it costs one full handshake.
    ///
    /// Anti-replay proper needs the strike-register design in `rusty_tls#58`,
    /// and needs it before any early data exists to replay.
    pub fn age_is_plausible(&self, obfuscated_ticket_age: u32, now: i64, skew_ms: u32) -> bool {
        let reported = self.reported_age_ms(obfuscated_ticket_age);
        // Saturating throughout: a ticket from a server whose clock is wildly
        // ahead should fail this check, not wrap around into passing it.
        let elapsed = now.saturating_sub(self.issued_at).max(0);
        let actual = u32::try_from(elapsed)
            .unwrap_or(u32::MAX)
            .saturating_mul(1000);
        actual.abs_diff(reported) <= skew_ms
    }

    /// True if `now` is within `lifetime` seconds of when this was issued.
    ///
    /// Both directions are checked. A ticket from the future is as suspect as
    /// one from too far in the past — it means the issuing server's clock and
    /// this one's disagree, and accepting it would extend the ticket's real
    /// lifetime by however much they differ.
    pub fn is_current(&self, now: i64) -> bool {
        let age = now - self.issued_at;
        age >= 0 && age <= i64::from(self.lifetime)
    }
}

/// Says nothing about key material.
impl core::fmt::Display for TicketContents {
    fn fmt(&self, f: &mut core::fmt::Formatter<'_>) -> core::fmt::Result {
        write!(
            f,
            "a ticket for suite 0x{:04x}, issued at {}, good for {}s",
            self.suite.0, self.issued_at, self.lifetime
        )
    }
}
