//! The TLS 1.3 server handshake — stage 5.
//!
//! The mirror of [`super::client`], and the last stage in ADR-0002's table.
//! The ADR calls it "last, or never", and the reason is worth restating here
//! rather than only there, because it changes how this module should be read.
//!
//! # A server is exposed differently from a client
//!
//! A client talks to peers it chose; a server answers whoever connects. Three
//! consequences shape what is here:
//!
//! - **Every byte parsed is unsolicited.** A client's peer had to be dialled
//!   first. A server's did not, so every parser this reaches is reachable by
//!   anyone who can open a socket.
//! - **It holds a private key and signs on demand.** Everything before stage 5
//!   only *checked* signatures. A server produces one per connection, over a
//!   transcript the peer partly controls — which is precisely why the
//!   CertificateVerify content is built by
//!   [`super::handshake::certificate_verify_content`], with its padding and
//!   context string, and never from a bare hash.
//! - **State is per-connection and attacker-triggered.** A client's handshake
//!   state machine runs once against one peer; a server's runs concurrently
//!   against everyone.
//!
//! Nothing here changes the fact that this whole module is behind two gates
//! and is not the engine this crate ships. But "not shipped" is a reason to be
//! careful about claims, not a reason to be careless about code.
//!
//! # What is deliberately not supported
//!
//! Refused rather than half-implemented, on the same principle the client
//! applies:
//!
//! - **0-RTT.** Not accepted, and an `early_data` extension in a ClientHello
//!   is *refused* rather than dropped — ADR-0003 records that as a decision.
//!   A server that ignored it would look, to the client, exactly like one that
//!   had considered the early data and declined it, and those two answers mean
//!   different things to a client deciding what it may replay.
//! - **`psk_ke`.** A client that offers no `psk_dhe_ke` is answered with a full
//!   handshake. Resuming without fresh key material trades the session's
//!   forward secrecy for one saved key exchange.
//! - **Resumption together with client authentication.** When
//!   [`ServerConfig::client_auth`] is configured, a `pre_shared_key` offer is
//!   answered with a full handshake. That is a gap and not a preference: these
//!   tickets carry no client identity, so a resumed connection would report no
//!   client certificate and [`ClientAuth::required`] would go quietly
//!   unenforced. Falling back keeps the posture the configuration asked for.
//!   See `rusty_tls#43`.
//! - **TLS 1.2 and below.** A ClientHello that does not offer `0x0304` in
//!   `supported_versions` gets a `protocol_version` alert, which is what stage
//!   4a taught this code to send and to read.
//!
//! # Session resumption
//!
//! Implemented, as of `rusty_tls#43`, and off unless [`ServerConfig`] carries a
//! [`Tickets`]. When it does, this server issues NewSessionTickets after each
//! handshake and accepts them back as a `pre_shared_key` on a later one.
//!
//! **Stateless.** A ticket is a sealed copy of the resumption PSK rather than a
//! handle into a table, so this server keeps no per-session storage — and in
//! exchange it keeps a key that is as sensitive as [`ServerConfig::key`].
//! [`super::ticket`]'s module docs say why, and why rotating it is a deployment
//! concern this crate documents rather than hides.
//!
//! Three rules decide whether an offer is taken, and the third is the one that
//! is not optional:
//!
//! - **A ticket that does not open, has expired, belongs to another cipher
//!   suite, or was issued under a different certificate chain is ignored**, and
//!   the handshake proceeds in full. §4.2.11 permits that, and it has to:
//!   every ticket becomes unopenable eventually, and a server that refused
//!   would turn its own key rotation into an outage.
//! - **An identity that *is* recognised has its binder verified**, over the
//!   ClientHello truncated to just before the binders.
//! - **A recognised identity whose binder does not verify aborts the
//!   handshake.** Not a fallback. Once the server has decided which key the
//!   client claims to hold, the binder is the only proof it holds it, and
//!   falling back would make that proof optional — the handshake would still
//!   complete, and nothing would have been checked.
//!
//! # Client certificates
//!
//! Implemented, as of `rusty_tls#42`, and off unless [`ServerConfig`] carries a
//! [`ClientAuth`]. When it does, this server sends a CertificateRequest naming
//! the schemes it accepts, then checks two separate things about the answer:
//! that the chain validates to the configured anchors, and that the
//! CertificateVerify was made by the key in the leaf. **Both are required.** A
//! certificate proves nothing on its own — anyone can present anyone's — and
//! the signature is the only thing tying it to the peer that sent it.
//!
//! No name is matched. A client certificate identifies a client and there is
//! no hostname for it to be checked against; deciding whether *this* client is
//! allowed is the application's job, and
//! [`Connection::peer_certificates`] is where it reads what arrived.
//!
//! [`ClientAuth::required`] is spelled out rather than inferred from whether
//! anchors were configured, because "ask, and accept whoever turns up
//! empty-handed" and "ask, and refuse them" are different security postures.
//!
//! # HelloRetryRequest
//!
//! Implemented, as of `rusty_tls#44`. A client that supports a group this
//! server does but sent no `key_share` for it is asked to try again, rather
//! than refused with `handshake_failure` — refusing it turned away a client
//! that would have completed after one extra round trip, for a reason that was
//! not its fault.
//!
//! Two limits are worth knowing before relying on it:
//!
//! - **§4.1.2 is checked in part, not in full.** [`ServerHandshake`] verifies
//!   that the second hello carries the same `random` and `legacy_session_id`,
//!   still offers the negotiated cipher suite and TLS 1.3, and now has a share
//!   for the group it was asked for. It does not diff the two hellos field by
//!   field, because doing so means retaining ClientHello1 across a round trip —
//!   attacker-supplied bytes held per connection — and §4.4.1's `message_hash`
//!   substitution exists so a server does not have to.
//! - **One retry, ever.** §4.1.4 forbids a second, so a client that comes back
//!   without the share it was asked for is refused rather than asked again.

use super::handshake::{
    certificate_verify_content, complete_prefix, extension, find, messages, parse_finished,
    CertificateMessage, CertificateVerify, ClientHello, Extension, HandshakeError, HandshakeType,
    Message, PresharedKeyOffer, ServerHello, Transcript, CLIENT_CERTIFICATE_VERIFY_CONTEXT,
    HELLO_RETRY_REQUEST_RANDOM, SERVER_CERTIFICATE_VERIFY_CONTEXT,
};
use super::kx::{KeyExchange, KxError, NamedGroup};
use super::path::{validate_path, PathError, PathOptions, TrustAnchor};
use super::record::{
    Aead, ContentType, Opener, RecordError, Sealer, HEADER_LEN, MAX_ENCRYPTED_FRAGMENT_LEN,
};
use super::schedule::{
    expand_label, finished_verify_data, traffic_keys, verify_finished, verify_psk_binder, Hash,
    KeySchedule,
};
use super::sign::{SignError, SigningKey};
use super::ticket::{TicketContents, TicketError, TicketKeys};
use super::verify::{verify_tls13_signature, SignatureScheme, VerifyError};
use super::wire::{Reader, Writer};
use super::x509::{Certificate, X509Error};

use super::client::{Alert, AlertDescription, AlertLevel, CipherSuite, Connection};

/// Everything the server handshake can refuse.
///
/// Most variants correspond to an alert this server sends before giving up —
/// see [`ServerError::alert`]. A server that failed silently would leave every
/// client guessing, which is the situation stage 4a fixed on the client side.
#[derive(Clone, Debug, PartialEq, Eq)]
#[non_exhaustive]
pub enum ServerError {
    /// A handshake message was malformed.
    Handshake(HandshakeError),
    /// A record was malformed or did not decrypt.
    Record(RecordError),
    /// The key exchange failed.
    Kx(KxError),
    /// Signing the handshake failed.
    Sign(SignError),
    /// A message arrived where a different one was required.
    UnexpectedMessage {
        /// What the state machine required.
        expected: &'static str,
        /// What arrived.
        got: HandshakeType,
    },
    /// A record carried content the handshake has no use for.
    UnexpectedContentType(ContentType),
    /// The client did not offer TLS 1.3.
    NotTls13,
    /// The client offered no cipher suite this server implements.
    NoSharedCipherSuite,
    /// The client's `supported_groups` named no group this server implements.
    ///
    /// Distinct from "sent no share for one": a client that supports a group
    /// this server does can be sent a HelloRetryRequest and will succeed on the
    /// second try. This is the case where there is nothing to retry *with*, so
    /// the handshake genuinely cannot continue.
    NoSharedGroup,
    /// The retried ClientHello still carried no share for the requested group.
    ///
    /// RFC 8446 §4.1.4 forbids a second HelloRetryRequest, so a client that
    /// ignores the first has nowhere left to go.
    RetriedHelloStillHasNoShare(NamedGroup),
    /// The retried ClientHello was not the same client.
    ///
    /// §4.1.2 permits a narrow set of changes between the two hellos, and
    /// `random` and `legacy_session_id` are not among them. A client free to
    /// rewrite its own offer mid-handshake could present one set of parameters
    /// to be judged and a different set to be used.
    RetriedHelloChangedIdentity,
    /// The client offered no signature scheme this server's key can produce.
    NoSharedSignatureScheme,
    /// The client's Finished did not verify.
    BadFinished,
    /// A client certificate was required and none was presented.
    ///
    /// An empty Certificate is a conforming answer, not a malformed one — this
    /// is the server declining it, which is the server's decision to make.
    ClientCertificateRequired,
    /// A certificate the client presented could not be parsed.
    MalformedClientCertificate(X509Error),
    /// The client's certificate chain did not validate.
    ///
    /// No name is checked: a client certificate identifies a client, and there
    /// is no hostname for it to match. What the application does with the
    /// identity is above this layer, and
    /// [`Connection::peer_certificates`] is where it reads it.
    ClientCertificate(PathError),
    /// The client's CertificateVerify did not verify.
    ///
    /// The certificate proves nothing on its own; anyone can present anyone's.
    /// This is the check that the peer holds the matching private key.
    ClientCertificateVerify(VerifyError),
    /// The peer sent an alert.
    PeerAlert(Alert),
    /// A record arrived after the connection was already broken.
    Failed,
    /// The system random source failed.
    Random,
    /// A `pre_shared_key` offer was accepted and its binder did not verify.
    ///
    /// Fatal, and RFC 8446 §4.2.11 says so: once the server has recognised an
    /// identity it MUST validate that identity's binder and abort if it does
    /// not check out. Falling back to a full handshake instead would turn the
    /// one proof that the client holds the PSK into an optional extra.
    BadBinder,
    /// A ClientHello carried `pre_shared_key` somewhere other than last.
    ///
    /// §4.2.11 requires it to be the final extension, because its binders cover
    /// the hello truncated to just before them. Anything after it is outside
    /// what the binders prove.
    PskOfferNotLast,
    /// A ClientHello carried an `early_data` extension.
    ///
    /// ADR-0003 puts 0-RTT out of scope. Refused rather than ignored: a server
    /// that dropped the extension silently would look to a client exactly like
    /// one that considered and rejected the early data, and the difference
    /// matters to a client deciding what it may replay.
    EarlyDataOffered,
    /// A ticket opened and its contents were not a ticket.
    Ticket(TicketError),
}

impl core::fmt::Display for ServerError {
    fn fmt(&self, f: &mut core::fmt::Formatter<'_>) -> core::fmt::Result {
        match self {
            Self::Handshake(err) => write!(f, "malformed handshake message: {err}"),
            Self::Record(err) => write!(f, "record layer: {err}"),
            Self::Kx(err) => write!(f, "key exchange: {err}"),
            Self::Sign(err) => write!(f, "signing: {err}"),
            Self::UnexpectedMessage { expected, got } => {
                write!(f, "expected {expected}, got {got:?}")
            }
            Self::UnexpectedContentType(typ) => write!(f, "unexpected content type {typ:?}"),
            Self::NotTls13 => f.write_str("the client did not offer TLS 1.3"),
            Self::NoSharedCipherSuite => f.write_str("no cipher suite in common"),
            Self::NoSharedGroup => f.write_str("no key exchange group in common"),
            Self::RetriedHelloStillHasNoShare(group) => write!(
                f,
                "the retried ClientHello still carried no share for {group:?}"
            ),
            Self::RetriedHelloChangedIdentity => {
                f.write_str("the retried ClientHello was not the same hello")
            }
            Self::NoSharedSignatureScheme => f.write_str("no signature scheme in common"),
            Self::BadFinished => f.write_str("the client's Finished did not verify"),
            Self::ClientCertificateRequired => {
                f.write_str("a client certificate was required and none was sent")
            }
            Self::MalformedClientCertificate(err) => {
                write!(f, "a client certificate did not parse: {err}")
            }
            Self::ClientCertificate(err) => write!(f, "the client's certificate: {err}"),
            Self::ClientCertificateVerify(err) => {
                write!(f, "the client's CertificateVerify: {err}")
            }
            Self::PeerAlert(alert) => {
                write!(
                    f,
                    "the peer sent a {:?} alert: {}",
                    alert.level, alert.description
                )
            }
            Self::Failed => f.write_str("the connection already failed"),
            Self::Random => f.write_str("the system random source failed"),
            Self::BadBinder => f.write_str("a recognised PSK identity's binder did not verify"),
            Self::PskOfferNotLast => {
                f.write_str("pre_shared_key was not the last extension in the ClientHello")
            }
            Self::EarlyDataOffered => {
                f.write_str("the client offered early_data, which this server does not accept")
            }
            Self::Ticket(err) => write!(f, "a session ticket: {err}"),
        }
    }
}

impl std::error::Error for ServerError {}

impl From<HandshakeError> for ServerError {
    fn from(err: HandshakeError) -> Self {
        Self::Handshake(err)
    }
}
impl From<RecordError> for ServerError {
    fn from(err: RecordError) -> Self {
        Self::Record(err)
    }
}
impl From<KxError> for ServerError {
    fn from(err: KxError) -> Self {
        Self::Kx(err)
    }
}
impl From<SignError> for ServerError {
    fn from(err: SignError) -> Self {
        Self::Sign(err)
    }
}

impl ServerError {
    /// The alert a client should be told about this, if any.
    ///
    /// Deliberately coarse. A server that mapped every internal distinction to
    /// its own alert would be describing its own parser to whoever asked, and
    /// the RFC's alert set is not that fine-grained anyway.
    pub const fn alert(&self) -> Option<AlertDescription> {
        Some(match self {
            Self::NotTls13 => AlertDescription::PROTOCOL_VERSION,
            Self::Handshake(_) | Self::UnexpectedMessage { .. } => AlertDescription::DECODE_ERROR,
            Self::UnexpectedContentType(_) => AlertDescription::ILLEGAL_PARAMETER,
            Self::BadFinished | Self::ClientCertificateVerify(_) => AlertDescription::DECRYPT_ERROR,
            Self::ClientCertificateRequired => AlertDescription::CERTIFICATE_REQUIRED,
            Self::ClientCertificate(_) | Self::MalformedClientCertificate(_) => {
                AlertDescription::BAD_CERTIFICATE
            }
            Self::Record(_) => AlertDescription::BAD_RECORD_MAC,
            Self::NoSharedCipherSuite
            | Self::NoSharedGroup
            | Self::RetriedHelloStillHasNoShare(_)
            | Self::Kx(_)
            | Self::NoSharedSignatureScheme
            | Self::Sign(_)
            | Self::Random => AlertDescription::HANDSHAKE_FAILURE,
            // §4.1.2 calls a hello that changed where it may not an illegal
            // parameter, which is more specific than "handshake failure" and
            // tells the client which of the two hellos to look at.
            Self::RetriedHelloChangedIdentity => AlertDescription::ILLEGAL_PARAMETER,
            Self::BadBinder => AlertDescription::DECRYPT_ERROR,
            Self::PskOfferNotLast | Self::EarlyDataOffered => AlertDescription::ILLEGAL_PARAMETER,
            Self::Ticket(_) => AlertDescription::HANDSHAKE_FAILURE,
            // The peer already knows; telling it again is noise.
            Self::PeerAlert(_) | Self::Failed => return None,
        })
    }
}

type Result<T> = core::result::Result<T, ServerError>;

// ---------------------------------------------------------------------------
// Configuration
// ---------------------------------------------------------------------------

/// What a server needs before it can answer anything.
pub struct ServerConfig<'a> {
    /// The certificate chain, DER-encoded, end-entity first — exactly what
    /// goes into a Certificate message.
    pub certificates: &'a [Vec<u8>],
    /// The private key for the end-entity certificate.
    pub key: &'a SigningKey,
    /// The cipher suites this server will select, most preferred first.
    pub cipher_suites: &'a [CipherSuite],
    /// The key exchange groups this server will use, most preferred first.
    pub groups: &'a [NamedGroup],
    /// Whether, and how, to ask the client to authenticate.
    ///
    /// `None` — the default posture — sends no CertificateRequest, so no
    /// client is ever authenticated and none is ever asked to be.
    pub client_auth: Option<&'a ClientAuth<'a>>,
    /// Whether, and how, to issue and accept resumption tickets.
    ///
    /// `None` is the posture this server had before `rusty_tls#43`: no
    /// NewSessionTicket is issued, and a `pre_shared_key` offer is ignored and
    /// answered with a full handshake. A server that stores nothing cannot be
    /// confused about what it stored, and that remains a legitimate thing to
    /// choose.
    pub tickets: Option<&'a Tickets<'a>>,
}

/// How a server handles resumption.
///
/// Read [`super::ticket`]'s module docs before configuring one. The sealing
/// key in here is as sensitive as [`ServerConfig::key`], and ADR-0003 records
/// that rotating it is a deployment concern this crate must document rather
/// than hide — which is what [`super::ticket::TicketKeys`] is shaped for.
pub struct Tickets<'a> {
    /// The key new tickets are sealed with, plus any retired keys whose
    /// tickets are still accepted.
    pub keys: TicketKeys<'a>,
    /// The moment to issue and expire tickets against, in seconds since the
    /// Unix epoch.
    ///
    /// Supplied rather than read from a clock, for the reason
    /// [`PathOptions::time`] is — and, like it, it is read per handshake, so a
    /// long-lived configuration wants a fresh value per connection rather than
    /// one captured at start-up.
    pub now: i64,
    /// How long a ticket may be redeemed for, in seconds.
    ///
    /// This bounds how long a compromise of the sealing key reaches forward,
    /// so it is a security parameter and not only a convenience one. RFC 8446
    /// §4.6.1 caps it at seven days and says nothing about what is sensible
    /// below that.
    pub lifetime: u32,
    /// How many tickets to issue after a handshake.
    ///
    /// More than one lets a client open several connections later without
    /// linking them to each other, since a ticket is only meant to be used
    /// once. Zero issues none, which is how to accept resumption without
    /// offering any more of it.
    pub count: usize,
}

/// How a server treats client certificates.
///
/// `required` is not a default anywhere, and is spelled out rather than
/// inferred from whether anchors were supplied. "Ask, and accept whoever turns
/// up empty-handed" and "ask, and refuse them" are different security
/// postures, and a server should have to say which one it is running.
pub struct ClientAuth<'a> {
    /// The anchors a client's chain must reach.
    pub anchors: &'a [TrustAnchor<'a>],
    /// Path validation options, including the current time.
    pub path: PathOptions,
    /// Whether a client that presents no certificate is refused.
    ///
    /// `false` means the handshake continues with an unauthenticated client —
    /// which is only useful if the application above can tell the difference,
    /// so [`Connection::peer_certificates`] reports what arrived.
    pub required: bool,
}

// ---------------------------------------------------------------------------
// Framing
// ---------------------------------------------------------------------------

fn plaintext_record(typ: ContentType, fragment: &[u8]) -> Vec<u8> {
    let mut out = Vec::with_capacity(HEADER_LEN + fragment.len());
    out.push(typ.as_u8());
    out.extend_from_slice(&[0x03, 0x03]);
    out.extend_from_slice(&(fragment.len() as u16).to_be_bytes());
    out.extend_from_slice(fragment);
    out
}

const CHANGE_CIPHER_SPEC: u8 = 20;

// ---------------------------------------------------------------------------
// State
// ---------------------------------------------------------------------------

struct Negotiated {
    aead: Aead,
    hash: Hash,
    suite: CipherSuite,
    transcript: Transcript,
    client_handshake_secret: Vec<u8>,
    opener: Opener,
    client_application_secret: Vec<u8>,
    server_application_secret: Vec<u8>,
    /// The Master Secret, kept so `res master` can be derived once the
    /// client's Finished arrives.
    ///
    /// It cannot be derived earlier: `res master` covers the transcript
    /// *through* that message, and every other secret in the handshake is
    /// bound to a point before it.
    master: KeySchedule,
    /// What this server is still waiting for from the client.
    ///
    /// A field rather than a `match` on whatever arrived, for the same reason
    /// the client's `Expect` is one: with client authentication in play, "the
    /// CertificateVerify was quietly missing" has to be the *default*
    /// rejection rather than a case someone remembered to write. A server that
    /// accepted a Certificate and then a Finished, skipping the proof of key
    /// possession in between, would authenticate anyone who could copy a
    /// certificate off the wire.
    expect: ExpectFromClient,
    /// The chain the client presented, empty if it presented none.
    client_certificates: Vec<Vec<u8>>,
    /// The transcript hash as of the end of the client's Certificate, which is
    /// what its CertificateVerify signs. Captured when that message is added
    /// rather than recomputed later.
    client_certificate_transcript: Vec<u8>,
}

/// The message a server will accept next from the client.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum ExpectFromClient {
    /// No CertificateRequest was sent, so the Finished comes straight away.
    Finished,
    /// A CertificateRequest was sent; §4.4.2's answer comes first.
    Certificate,
    /// A non-empty Certificate arrived, so its proof of possession is due.
    CertificateVerify,
    /// A Certificate arrived and was empty, so there is nothing to prove.
    FinishedAfterEmptyCertificate,
}

impl ExpectFromClient {
    const fn name(self) -> &'static str {
        match self {
            Self::Finished | Self::FinishedAfterEmptyCertificate => "Finished",
            Self::Certificate => "Certificate",
            Self::CertificateVerify => "CertificateVerify",
        }
    }
}

/// What a HelloRetryRequest has to remember until the second hello arrives.
///
/// Deliberately small, and it does **not** include the first ClientHello. RFC
/// 8446 §4.4.1's synthetic `message_hash` substitution exists precisely so a
/// server need not retain it, and a server that kept it anyway would be
/// carrying attacker-supplied bytes across a round trip for every connection
/// that retries — the opposite of what the substitution is for.
///
/// `random` and `session_id` are kept because they are 32 and at most 32 bytes,
/// and because §4.1.2 makes them the cheapest evidence that the second hello
/// came from the same client. See [`ServerHandshake::retried_hello`] for what
/// that does and does not prove.
struct Retrying {
    group: NamedGroup,
    suite: CipherSuite,
    aead: Aead,
    hash: Hash,
    /// `message_hash(Hash(ClientHello1)) || HelloRetryRequest`, which is what
    /// the post-retry transcript begins with.
    transcript_head: Vec<u8>,
    random: Vec<u8>,
    session_id: Vec<u8>,
}

enum State {
    AwaitClientHello,
    AwaitRetriedClientHello(Box<Retrying>),
    AwaitFinished(Box<Negotiated>),
    Done(Box<Connection>),
    Failed,
}

/// A TLS 1.3 server handshake in progress.
///
/// Sans-IO, exactly as [`super::client::ClientHandshake`] is: feed it one
/// record, send whatever comes back.
pub struct ServerHandshake<'a> {
    config: &'a ServerConfig<'a>,
    state: State,
    buffer: Vec<u8>,
}

impl<'a> ServerHandshake<'a> {
    /// A server waiting for a ClientHello.
    ///
    /// Unlike the client, there is nothing to send first — a server speaks
    /// only when spoken to, which is the whole difference in exposure.
    pub const fn new(config: &'a ServerConfig<'a>) -> Self {
        Self {
            config,
            state: State::AwaitClientHello,
            buffer: Vec::new(),
        }
    }

    /// True once the handshake is complete.
    pub const fn is_finished(&self) -> bool {
        matches!(self.state, State::Done(_))
    }

    /// Take the established connection.
    pub fn into_connection(self) -> Result<Connection> {
        match self.state {
            State::Done(connection) => Ok(*connection),
            _ => Err(ServerError::Failed),
        }
    }

    /// Feed one whole record; get back what to send.
    ///
    /// On failure the returned error carries an alert in
    /// [`ServerError::alert`], and [`Self::alert_record`] frames it. The alert
    /// is not sent automatically because whether to answer a hostile peer at
    /// all is a policy question this module should not decide.
    pub fn read_record(&mut self, record: &[u8]) -> Result<Vec<u8>> {
        if matches!(self.state, State::Failed) {
            return Err(ServerError::Failed);
        }
        match self.read_record_inner(record) {
            Ok(reply) => Ok(reply),
            Err(err) => {
                self.state = State::Failed;
                Err(err)
            }
        }
    }

    /// Frame an alert for a failed handshake, ready to send.
    ///
    /// Returns `None` when the peer should not be told — it already sent an
    /// alert of its own, or the connection was already dead.
    pub fn alert_record(&self, error: &ServerError) -> Option<Vec<u8>> {
        let description = error.alert()?;
        // Always in the clear. A server that failed before deriving keys has
        // none, and one that failed afterwards is telling the peer something
        // the peer can already infer from the connection dying.
        Some(plaintext_record(ContentType::Alert, &[2, description.0]))
    }

    fn read_record_inner(&mut self, record: &[u8]) -> Result<Vec<u8>> {
        if record.len() < HEADER_LEN {
            return Err(RecordError::Truncated {
                len: record.len(),
                min: HEADER_LEN,
            }
            .into());
        }
        if record[0] == CHANGE_CIPHER_SPEC {
            return Ok(Vec::new());
        }

        let declared = usize::from(u16::from_be_bytes([record[3], record[4]]));
        if record.len() != HEADER_LEN + declared {
            return Err(RecordError::LengthMismatch {
                declared,
                available: record.len() - HEADER_LEN,
            }
            .into());
        }
        if declared > MAX_ENCRYPTED_FRAGMENT_LEN {
            return Err(RecordError::EncryptedFragmentTooLong { len: declared }.into());
        }

        let fragment = match &mut self.state {
            State::AwaitClientHello | State::AwaitRetriedClientHello(_) => {
                if ContentType::from_u8(record[0]) == ContentType::Alert {
                    return Err(alert_error(&record[HEADER_LEN..]));
                }
                if ContentType::from_u8(record[0]) != ContentType::Handshake {
                    return Err(ServerError::UnexpectedContentType(ContentType::from_u8(
                        record[0],
                    )));
                }
                record[HEADER_LEN..].to_vec()
            }
            State::AwaitFinished(negotiated) => {
                let opened = negotiated.opener.open(record)?;
                match opened.typ {
                    ContentType::Handshake => opened.fragment,
                    ContentType::Alert => return Err(alert_error(&opened.fragment)),
                    other => return Err(ServerError::UnexpectedContentType(other)),
                }
            }
            State::Done(_) | State::Failed => return Err(ServerError::Failed),
        };

        self.buffer.extend_from_slice(&fragment);

        let mut reply = Vec::new();
        loop {
            let complete = complete_prefix(&self.buffer);
            if complete == 0 {
                return Ok(reply);
            }
            let consumed: Vec<u8> = self.buffer.drain(..complete).collect();
            for message in messages(&consumed)? {
                reply.extend_from_slice(&self.handle(&message)?);
            }
        }
    }

    fn handle(&mut self, message: &Message<'_>) -> Result<Vec<u8>> {
        match &self.state {
            State::AwaitClientHello => self.hello(message),
            State::AwaitRetriedClientHello(_) => self.retried_hello(message),
            State::AwaitFinished(_) => self.client_flight(message),
            State::Done(_) | State::Failed => Err(ServerError::Failed),
        }
    }
}

fn alert_error(body: &[u8]) -> ServerError {
    match body {
        [level, description] => ServerError::PeerAlert(Alert {
            level: AlertLevel::from_wire(*level),
            description: AlertDescription(*description),
        }),
        _ => ServerError::UnexpectedContentType(ContentType::Alert),
    }
}

// ---------------------------------------------------------------------------
// The ClientHello, and everything it decides
// ---------------------------------------------------------------------------

/// What negotiation settled on, so [`ServerHandshake::complete`] can be reached
/// from both the first ClientHello and a retried one without either path
/// growing its own copy of the flight.
struct Selected<'a> {
    suite: CipherSuite,
    aead: Aead,
    hash: Hash,
    scheme: SignatureScheme,
    group: NamedGroup,
    peer_key: &'a [u8],
    /// The pre-shared key this handshake resumes, if it resumes one.
    ///
    /// `Some` changes the flight — no CertificateRequest, no Certificate, no
    /// CertificateVerify — and changes where the key schedule starts. It is
    /// carried here rather than recomputed in `complete`, because the binder
    /// was verified against the ClientHello and `complete` no longer has it.
    psk: Option<AcceptedPsk>,
}

/// A `pre_shared_key` offer this server has decided to accept.
struct AcceptedPsk {
    /// Which of the client's identities was chosen, for `selected_identity`.
    index: u16,
    /// The key itself, out of the ticket.
    psk: Vec<u8>,
}

/// The groups a client says it supports, whether or not it sent a share for
/// them.
///
/// The difference from [`client_key_shares`] is the whole basis for a retry: a
/// group here with no share there is a client that can succeed on a second
/// attempt, not one that has to be refused.
fn client_supported_groups(data: &[u8]) -> core::result::Result<Vec<u16>, HandshakeError> {
    let mut reader = Reader::new(data);
    let mut list = reader.sub_u16().map_err(HandshakeError::Wire)?;
    reader.finish().map_err(HandshakeError::Wire)?;

    let mut out = Vec::new();
    while !list.is_empty() {
        out.push(list.u16().map_err(HandshakeError::Wire)?);
    }
    Ok(out)
}

/// The groups a client sent key shares for, in the order it sent them.
fn client_key_shares(data: &[u8]) -> core::result::Result<Vec<(u16, &[u8])>, HandshakeError> {
    let mut reader = Reader::new(data);
    let mut list = reader.sub_u16().map_err(HandshakeError::Wire)?;
    reader.finish().map_err(HandshakeError::Wire)?;

    let mut out = Vec::new();
    while !list.is_empty() {
        let group = list.u16().map_err(HandshakeError::Wire)?;
        let key = list.vector_u16().map_err(HandshakeError::Wire)?;
        out.push((group, key));
    }
    Ok(out)
}

/// The signature schemes a client will accept.
fn client_signature_schemes(data: &[u8]) -> core::result::Result<Vec<u16>, HandshakeError> {
    let mut reader = Reader::new(data);
    let mut list = reader.sub_u16().map_err(HandshakeError::Wire)?;
    reader.finish().map_err(HandshakeError::Wire)?;

    let mut out = Vec::new();
    while !list.is_empty() {
        out.push(list.u16().map_err(HandshakeError::Wire)?);
    }
    Ok(out)
}

/// True if the client offered TLS 1.3.
fn offers_tls13(data: &[u8]) -> bool {
    let Ok(list) = Reader::new(data).sub_u8() else {
        return false;
    };
    let mut list = list;
    while !list.is_empty() {
        match list.u16() {
            Ok(0x0304) => return true,
            Ok(_) => {}
            Err(_) => return false,
        }
    }
    false
}

/// The `psk_key_exchange_modes` a client offered.
///
/// RFC 8446 §4.2.9: a server MUST NOT resume unless the client offered a mode
/// it can use, and `psk_dhe_ke(1)` is the only one ADR-0003 permits. A client
/// offering `psk_ke` alone is asking to resume without fresh key material,
/// which is a trade this engine does not make.
fn offers_psk_dhe_ke(data: &[u8]) -> bool {
    let Ok(mut list) = Reader::new(data).sub_u8() else {
        return false;
    };
    while !list.is_empty() {
        match list.u8() {
            Ok(1) => return true,
            Ok(_) => {}
            Err(_) => return false,
        }
    }
    false
}

impl ServerHandshake<'_> {
    /// Decide whether to resume, and prove the client holds the key if so.
    ///
    /// `transcript_prefix` is everything in the transcript before this hello —
    /// empty on a first attempt, and `message_hash || HelloRetryRequest` after
    /// a retry. The binder covers that prefix followed by the hello truncated
    /// to just before the binders, which is why this needs the hello's encoded
    /// bytes and not only its parsed form.
    ///
    /// Three outcomes, and the difference between them is the whole of §4.2.11:
    ///
    /// - **No offer, or none this server recognises** — `Ok(None)`, and the
    ///   handshake proceeds in full. An unrecognised ticket is not an error;
    ///   it is what every ticket becomes after a key rotation or an expiry, and
    ///   a server that aborted would break clients for being patient.
    /// - **An identity recognised and its binder verified** — `Ok(Some(_))`.
    /// - **An identity recognised and its binder wrong** —
    ///   [`ServerError::BadBinder`], fatal. Once the server has decided which
    ///   key the client claims to hold, the binder is the only proof it holds
    ///   it, and falling back to a full handshake instead would make that proof
    ///   optional.
    fn accept_psk(
        &self,
        hello: &ClientHello<'_>,
        encoded_hello: &[u8],
        transcript_prefix: &[u8],
        suite: CipherSuite,
        hash: Hash,
    ) -> Result<Option<AcceptedPsk>> {
        // ADR-0003, and checked before anything else so it is refused whether
        // or not this server would have resumed at all.
        if find(&hello.extensions, extension::EARLY_DATA).is_some() {
            return Err(ServerError::EarlyDataOffered);
        }

        let Some(data) = find(&hello.extensions, extension::PRE_SHARED_KEY) else {
            return Ok(None);
        };
        // §4.2.11 makes this a MUST, and it is the one structural rule the
        // binder depends on. Checked even when this server would not have
        // resumed, because a hello built that way is malformed rather than
        // merely unusable.
        match hello.extensions.last() {
            Some(last) if last.typ == extension::PRE_SHARED_KEY => {}
            _ => return Err(ServerError::PskOfferNotLast),
        }

        let Some(tickets) = self.config.tickets else {
            return Ok(None);
        };
        // Client authentication and resumption are not combined here, and the
        // reason is a hole rather than a preference: these tickets carry no
        // client identity, so a resumed connection would report no client
        // certificate and `ClientAuth::required` would go quietly
        // unenforced. Falling back to a full handshake keeps the posture the
        // configuration asked for. See `rusty_tls#43`.
        if self.config.client_auth.is_some() {
            return Ok(None);
        }
        match find(&hello.extensions, extension::PSK_KEY_EXCHANGE_MODES) {
            Some(modes) if offers_psk_dhe_ke(modes) => {}
            _ => return Ok(None),
        }

        let offer = PresharedKeyOffer::parse(data)?;
        let truncated = offer.truncated(encoded_hello)?;
        let mut covered = Vec::with_capacity(transcript_prefix.len() + truncated.len());
        covered.extend_from_slice(transcript_prefix);
        covered.extend_from_slice(truncated);
        let covered = hash.hash(&covered);

        let identity = TicketContents::identity_of(self.config.certificates);
        for (index, offered) in offer.identities.iter().enumerate() {
            let Some(contents) = TicketContents::open(offered.identity, &tickets.keys) else {
                continue;
            };
            let contents = contents.map_err(ServerError::Ticket)?;

            // A PSK is bound to its suite's hash, so one established under a
            // different suite is not a key this handshake can use. Not an
            // error: §4.2.11 leaves the server to select a compatible PSK, and
            // selecting none is a legitimate selection.
            if contents.suite != suite || !contents.is_current(tickets.now) {
                continue;
            }
            // And bound to the identity that issued it. Two configurations
            // sharing a sealing key would otherwise let a session
            // authenticated by one certificate chain continue under the other.
            if contents.identity != identity {
                continue;
            }

            if !verify_psk_binder(hash, &contents.psk, &covered, offer.binders[index]) {
                return Err(ServerError::BadBinder);
            }
            return Ok(Some(AcceptedPsk {
                index: index as u16,
                psk: contents.psk,
            }));
        }

        Ok(None)
    }

    fn hello(&mut self, message: &Message<'_>) -> Result<Vec<u8>> {
        if message.typ != HandshakeType::ClientHello {
            return Err(ServerError::UnexpectedMessage {
                expected: "ClientHello",
                got: message.typ,
            });
        }
        let hello = ClientHello::parse(message.body)?;

        // Version first. Everything below assumes TLS 1.3 semantics, so a
        // client that did not ask for it must be turned away before any of it
        // runs — and it is turned away with `protocol_version`, which is the
        // alert stage 4a taught the client half to read.
        match find(&hello.extensions, extension::SUPPORTED_VERSIONS) {
            Some(data) if offers_tls13(data) => {}
            _ => return Err(ServerError::NotTls13),
        }

        let suite = *self
            .config
            .cipher_suites
            .iter()
            .find(|suite| hello.cipher_suites.contains(&suite.0))
            .ok_or(ServerError::NoSharedCipherSuite)?;
        let (aead, hash) = suite.parts().ok_or(ServerError::NoSharedCipherSuite)?;

        // The signature scheme has to be one the key can produce *and* the
        // client will accept. Signing with anything else produces a signature
        // the client is obliged to reject.
        let offered = find(&hello.extensions, extension::SIGNATURE_ALGORITHMS)
            .ok_or(ServerError::NoSharedSignatureScheme)?;
        let offered = client_signature_schemes(offered)?;
        let scheme = *self
            .config
            .key
            .schemes()
            .iter()
            .find(|scheme| offered.contains(&scheme.0))
            .ok_or(ServerError::NoSharedSignatureScheme)?;

        // A group this server supports, for which the client actually sent a
        // share. A group offered with no share is not a failure — it is what
        // HelloRetryRequest is for, so the miss falls through to `retry`.
        //
        // A *missing* `key_share` extension is treated the same as an empty
        // one rather than as a malformed hello: sending no shares and waiting
        // to be told which group to use is a legitimate thing for a client to
        // do, and it is the case a retry serves best.
        let shares = match find(&hello.extensions, extension::KEY_SHARE) {
            Some(data) => client_key_shares(data)?,
            None => Vec::new(),
        };
        let chosen = self.config.groups.iter().find_map(|group| {
            shares
                .iter()
                .find(|(offered, _)| *offered == group.as_u16())
                .map(|(_, key)| (*group, *key))
        });
        let Some((group, peer_key)) = chosen else {
            return self.retry(message, &hello, suite, aead, hash);
        };

        // Before `complete`, because the binder covers this hello and
        // `complete` is shared with the post-retry path that has a different
        // transcript in front of it.
        let psk = self.accept_psk(&hello, message.encoded, &[], suite, hash)?;

        self.complete(
            message.encoded,
            &hello,
            Selected {
                suite,
                aead,
                hash,
                scheme,
                group,
                peer_key,
                psk,
            },
            true,
        )
    }

    /// Ask the client to try again with a group this server can use.
    ///
    /// RFC 8446 §4.1.4. Reached when the client supports a group in common but
    /// sent no share for it — usually because it guessed at the server's
    /// preference and guessed wrong. Answering `handshake_failure` here, as
    /// this module did until `rusty_tls#44`, refuses a client that would have
    /// completed after one extra round trip, for a reason that is not its
    /// fault.
    fn retry(
        &mut self,
        message: &Message<'_>,
        hello: &ClientHello<'_>,
        suite: CipherSuite,
        aead: Aead,
        hash: Hash,
    ) -> Result<Vec<u8>> {
        // Now the miss is fatal: with no group in common there is nothing to
        // retry *with*, and a retry the client cannot satisfy is worse than a
        // refusal it can understand.
        let offered = find(&hello.extensions, extension::SUPPORTED_GROUPS)
            .ok_or(ServerError::NoSharedGroup)?;
        let offered = client_supported_groups(offered)?;
        let group = *self
            .config
            .groups
            .iter()
            .find(|group| offered.contains(&group.as_u16()))
            .ok_or(ServerError::NoSharedGroup)?;

        // A HelloRetryRequest *is* a ServerHello — same structure, with the
        // §4.1.3 sentinel random. Its `key_share` carries a bare group, with
        // no key: the server has not generated one yet, and will not until it
        // knows the client can meet it there.
        let mut share = Writer::new();
        share.u16(group.as_u16());
        let share = share.into_vec();
        let versions = vec![0x03, 0x04];
        let retry = ServerHello {
            random: &HELLO_RETRY_REQUEST_RANDOM,
            session_id: hello.session_id,
            cipher_suite: suite.0,
            extensions: vec![
                Extension {
                    typ: extension::KEY_SHARE,
                    data: &share,
                },
                Extension {
                    typ: extension::SUPPORTED_VERSIONS,
                    data: &versions,
                },
            ],
        };
        let retry = Message::encode(HandshakeType::ServerHello, &retry.encode());

        // §4.4.1: from here the transcript begins with a synthetic
        // `message_hash` message wrapping Hash(ClientHello1), not with
        // ClientHello1. The client half does the same substitution; getting it
        // wrong on either side produces two parties that agree about every
        // message and disagree about the hash of them.
        let mut transcript_head = Vec::with_capacity(4 + hash.len() + retry.len());
        transcript_head.push(254); // message_hash
        transcript_head.extend_from_slice(&[0x00, 0x00, hash.len() as u8]);
        transcript_head.extend_from_slice(&hash.hash(message.encoded));
        transcript_head.extend_from_slice(&retry);

        let mut out = plaintext_record(ContentType::Handshake, &retry);
        // Appendix D.4 puts this after the server's *first* message, which is
        // this one. The real ServerHello later must not send a second.
        out.extend_from_slice(&plaintext_record(ContentType::ChangeCipherSpec, &[0x01]));

        self.state = State::AwaitRetriedClientHello(Box::new(Retrying {
            group,
            suite,
            aead,
            hash,
            transcript_head,
            random: hello.random.to_vec(),
            session_id: hello.session_id.to_vec(),
        }));
        Ok(out)
    }

    /// The second ClientHello, after a HelloRetryRequest.
    ///
    /// # What §4.1.2 is and is not checked here
    ///
    /// The RFC permits a narrow set of differences between the two hellos and
    /// requires the rest to be identical. This checks the parts that can be
    /// checked without keeping the first hello: `random`, `legacy_session_id`,
    /// the negotiated cipher suite still being offered, TLS 1.3 still being
    /// offered, and a share for the group that was asked for.
    ///
    /// It does **not** diff the two hellos field by field, and that is a
    /// deliberate trade rather than an omission. A full comparison means
    /// retaining ClientHello1 for the round trip — attacker-supplied bytes held
    /// per connection — and §4.4.1's `message_hash` substitution exists
    /// specifically so a server does not have to. What is checked is the part
    /// that makes the second hello attributable to the first; a client that
    /// quietly changed, say, its ALPN list between the two would not be caught.
    fn retried_hello(&mut self, message: &Message<'_>) -> Result<Vec<u8>> {
        if message.typ != HandshakeType::ClientHello {
            return Err(ServerError::UnexpectedMessage {
                expected: "ClientHello",
                got: message.typ,
            });
        }
        let hello = ClientHello::parse(message.body)?;

        let State::AwaitRetriedClientHello(retrying) =
            core::mem::replace(&mut self.state, State::Failed)
        else {
            return Err(ServerError::Failed);
        };
        let retrying = *retrying;

        if hello.random != retrying.random.as_slice()
            || hello.session_id != retrying.session_id.as_slice()
        {
            return Err(ServerError::RetriedHelloChangedIdentity);
        }
        match find(&hello.extensions, extension::SUPPORTED_VERSIONS) {
            Some(data) if offers_tls13(data) => {}
            _ => return Err(ServerError::NotTls13),
        }
        if !hello.cipher_suites.contains(&retrying.suite.0) {
            return Err(ServerError::RetriedHelloChangedIdentity);
        }

        // The signature scheme is re-selected rather than remembered, because
        // it is chosen from this hello and must be one *this* hello offers.
        let offered = find(&hello.extensions, extension::SIGNATURE_ALGORITHMS)
            .ok_or(ServerError::NoSharedSignatureScheme)?;
        let offered = client_signature_schemes(offered)?;
        let scheme = *self
            .config
            .key
            .schemes()
            .iter()
            .find(|scheme| offered.contains(&scheme.0))
            .ok_or(ServerError::NoSharedSignatureScheme)?;

        // §4.1.4 forbids a second HelloRetryRequest, so a client that came back
        // without the share it was asked for has nowhere left to go.
        let shares = find(&hello.extensions, extension::KEY_SHARE)
            .ok_or(ServerError::RetriedHelloStillHasNoShare(retrying.group))?;
        let shares = client_key_shares(shares)?;
        let peer_key = shares
            .iter()
            .find(|(offered, _)| *offered == retrying.group.as_u16())
            .map(|(_, key)| *key)
            .ok_or(ServerError::RetriedHelloStillHasNoShare(retrying.group))?;

        // §4.1.2 requires the binder to be recomputed for the second hello, over
        // a transcript that now begins with the synthetic `message_hash` and
        // the retry — so the prefix is what the retry left behind, and the
        // hello being truncated is this one.
        let psk = self.accept_psk(
            &hello,
            message.encoded,
            &retrying.transcript_head,
            retrying.suite,
            retrying.hash,
        )?;

        let mut transcript_head = retrying.transcript_head;
        transcript_head.extend_from_slice(message.encoded);

        self.complete(
            &transcript_head,
            &hello,
            Selected {
                suite: retrying.suite,
                aead: retrying.aead,
                hash: retrying.hash,
                scheme,
                group: retrying.group,
                peer_key,
                psk,
            },
            // Already sent with the HelloRetryRequest: Appendix D.4 puts it
            // after the server's first message, and it has had one.
            false,
        )
    }

    /// Everything from the real ServerHello onwards.
    ///
    /// `transcript_head` is the transcript up to and including the ClientHello
    /// being answered — the hello itself on a first attempt, or
    /// `message_hash || HelloRetryRequest || ClientHello2` after a retry. That
    /// one parameter is the whole difference between the two paths, which is
    /// why they share this function rather than each having their own copy of
    /// the flight to drift from.
    fn complete(
        &mut self,
        transcript_head: &[u8],
        hello: &ClientHello<'_>,
        selected: Selected<'_>,
        send_change_cipher_spec: bool,
    ) -> Result<Vec<u8>> {
        let Selected {
            suite,
            aead,
            hash,
            scheme,
            group,
            peer_key,
            psk,
        } = selected;

        let kx = KeyExchange::generate(group)?;
        let mut share = Writer::new();
        share.u16(group.as_u16());
        share.vector_u16(|w| w.bytes(kx.public_key()));
        let share = share.into_vec();
        let versions = vec![0x03, 0x04];

        let mut extensions = vec![
            Extension {
                typ: extension::KEY_SHARE,
                data: &share,
            },
            Extension {
                typ: extension::SUPPORTED_VERSIONS,
                data: &versions,
            },
        ];
        // §4.2.11: a ServerHello's `pre_shared_key` is the index of the
        // identity that was chosen, and nothing else. Its presence is the only
        // signal the client gets that this handshake resumed — which is why
        // the client refuses one it did not offer.
        let selected_identity = psk.as_ref().map(|psk| psk.index.to_be_bytes());
        if let Some(index) = &selected_identity {
            extensions.push(Extension {
                typ: extension::PRE_SHARED_KEY,
                data: index,
            });
        }

        let random = random_bytes(32)?;
        let server_hello = ServerHello {
            random: &random,
            // Echoed verbatim: RFC 8446 §4.1.3 requires it, and middleboxes
            // in compatibility mode are watching for it.
            session_id: hello.session_id,
            cipher_suite: suite.0,
            extensions,
        };
        let server_hello = Message::encode(HandshakeType::ServerHello, &server_hello.encode());

        let mut transcript = Transcript::new(hash);
        transcript.add(transcript_head);
        transcript.add(&server_hello);
        let hello_hash = transcript.hash();

        // The second route through the key schedule ADR-0003 names: on a
        // resumed handshake the early secret is extracted from the PSK rather
        // than from zeroes. It is one line, it is the entire difference, and a
        // server that took the wrong branch would fail the client's Finished
        // with nothing to say which of the two sides disagreed.
        let schedule = kx.agree(peer_key, |secret| {
            match &psk {
                Some(psk) => KeySchedule::new_with_psk(hash, &psk.psk),
                None => KeySchedule::new(hash),
            }
            .into_handshake(secret)
        })?;
        let client_handshake_secret = schedule.derive("c hs traffic", &hello_hash);
        let server_handshake_secret = schedule.derive("s hs traffic", &hello_hash);

        let server_keys = traffic_keys(hash, &server_handshake_secret, aead.key_len());
        let mut sealer = Sealer::new(aead, &server_keys.key, &server_keys.iv)?;

        // The flight.
        let mut flight = Vec::new();
        let mut empty = Writer::new();
        empty.vector_u16(|_| {});
        let encrypted_extensions =
            Message::encode(HandshakeType::EncryptedExtensions, &empty.into_vec());
        transcript.add(&encrypted_extensions);
        flight.extend_from_slice(&encrypted_extensions);

        // §4.3.2 puts the request after EncryptedExtensions and before the
        // server's own Certificate. It names the schemes this server will
        // accept; a client that can produce none of them answers empty, which
        // `client_auth.required` then decides about.
        //
        // Never on a resumed handshake: §4.4.2 forbids requesting client
        // authentication in one, and `accept_psk` refuses to resume at all
        // when client authentication is configured, so the two are mutually
        // exclusive twice over.
        let expect = if self.config.client_auth.is_some() && psk.is_none() {
            let mut schemes = Writer::new();
            schemes.vector_u16(|w| {
                for scheme in SignatureScheme::TLS13_SUPPORTED {
                    w.u16(scheme.0);
                }
            });
            let schemes = schemes.into_vec();
            let mut body = Writer::new();
            // An empty certificate_request_context. §4.3.2 allows any value in
            // a handshake request and this server asks exactly once, so there
            // is nothing to disambiguate.
            body.vector_u8(|_| {});
            body.vector_u16(|w| {
                w.u16(extension::SIGNATURE_ALGORITHMS);
                w.vector_u16(|w| w.bytes(&schemes));
            });
            let request = Message::encode(HandshakeType::CertificateRequest, &body.into_vec());
            transcript.add(&request);
            flight.extend_from_slice(&request);
            ExpectFromClient::Certificate
        } else {
            ExpectFromClient::Finished
        };

        // A resumed handshake sends neither, and that is not an optimisation:
        // §4.4.2 says a server MUST NOT send a Certificate when it
        // authenticated with a PSK. The client already validated this chain on
        // the connection the ticket came from, and the binder is what proves
        // this is the same session — signing again would prove nothing the PSK
        // has not already proved, over a transcript the client would then have
        // to reconcile with a certificate it did not ask for.
        if psk.is_none() {
            let certificate = certificate_message(self.config.certificates);
            transcript.add(&certificate);
            flight.extend_from_slice(&certificate);

            // Signed over the transcript through the Certificate, with the
            // §4.4.3 padding and context string. Never over a bare hash.
            let content =
                certificate_verify_content(SERVER_CERTIFICATE_VERIFY_CONTEXT, &transcript.hash());
            let signature = self.config.key.sign(scheme, &content)?;
            let mut verify = Writer::new();
            verify.u16(scheme.0);
            verify.vector_u16(|w| w.bytes(&signature));
            let verify = Message::encode(HandshakeType::CertificateVerify, &verify.into_vec());
            transcript.add(&verify);
            flight.extend_from_slice(&verify);
        }

        let verify_data = finished_verify_data(hash, &server_handshake_secret, &transcript.hash());
        let finished = Message::encode(HandshakeType::Finished, &verify_data);
        transcript.add(&finished);
        flight.extend_from_slice(&finished);

        // The application secrets are bound to the transcript through the
        // server's Finished, so they are derived now and held until the
        // client's Finished proves the peer got here too.
        let after_server_finished = transcript.hash();
        let master = schedule.into_master();
        let client_application_secret = master.derive("c ap traffic", &after_server_finished);
        let server_application_secret = master.derive("s ap traffic", &after_server_finished);

        let client_keys = traffic_keys(hash, &client_handshake_secret, aead.key_len());
        let opener = Opener::new(aead, &client_keys.key, &client_keys.iv)?;

        let mut out = plaintext_record(ContentType::Handshake, &server_hello);
        if send_change_cipher_spec {
            out.extend_from_slice(&plaintext_record(ContentType::ChangeCipherSpec, &[0x01]));
        }
        out.extend_from_slice(&sealer.seal(ContentType::Handshake, &flight, 0)?);

        self.state = State::AwaitFinished(Box::new(Negotiated {
            aead,
            hash,
            suite,
            transcript,
            client_handshake_secret,
            opener,
            client_application_secret,
            server_application_secret,
            master,
            expect,
            client_certificates: Vec::new(),
            client_certificate_transcript: Vec::new(),
        }));
        Ok(out)
    }

    /// The client's closing flight: its Certificate and CertificateVerify when
    /// one was asked for, then its Finished.
    ///
    /// The expectation is a field of the state, not a `match` on what arrived.
    /// With client authentication in play that distinction is the whole
    /// safety property: a server that took a Certificate and then a Finished,
    /// skipping the proof of possession in between, would authenticate anyone
    /// who could copy a certificate off the wire.
    fn client_flight(&mut self, message: &Message<'_>) -> Result<Vec<u8>> {
        let State::AwaitFinished(negotiated) = &mut self.state else {
            return Err(ServerError::Failed);
        };

        match (negotiated.expect, message.typ) {
            (ExpectFromClient::Certificate, HandshakeType::Certificate) => {
                let certificate = CertificateMessage::parse(message.body)?;
                let auth = self
                    .config
                    .client_auth
                    .expect("only requested when configured");

                if certificate.entries.is_empty() {
                    if auth.required {
                        self.state = State::Failed;
                        return Err(ServerError::ClientCertificateRequired);
                    }
                    // Accepted, and recorded as unauthenticated. The
                    // application above can tell the difference by asking the
                    // connection what the peer presented.
                    negotiated.transcript.add_message(message);
                    negotiated.expect = ExpectFromClient::FinishedAfterEmptyCertificate;
                    return Ok(Vec::new());
                }

                let chain: Vec<Vec<u8>> = certificate
                    .entries
                    .iter()
                    .map(|entry| entry.certificate.to_vec())
                    .collect();
                let parsed: core::result::Result<Vec<Certificate<'_>>, _> =
                    chain.iter().map(|der| Certificate::parse(der)).collect();
                let parsed = match parsed {
                    Ok(parsed) => parsed,
                    Err(err) => {
                        self.state = State::Failed;
                        return Err(ServerError::MalformedClientCertificate(err));
                    }
                };
                let (end_entity, intermediates) =
                    parsed.split_first().expect("checked non-empty above");

                // No name check. A client certificate identifies a client and
                // there is no hostname for it to match — using the server's
                // own name here would be nonsense that happened to compile.
                if let Err(err) = validate_path(end_entity, intermediates, auth.anchors, &auth.path)
                {
                    self.state = State::Failed;
                    return Err(ServerError::ClientCertificate(err));
                }

                negotiated.transcript.add_message(message);
                negotiated.client_certificate_transcript = negotiated.transcript.hash();
                negotiated.client_certificates = chain;
                negotiated.expect = ExpectFromClient::CertificateVerify;
                Ok(Vec::new())
            }
            (ExpectFromClient::CertificateVerify, HandshakeType::CertificateVerify) => {
                let verify = CertificateVerify::parse(message.body)?;
                let leaf = negotiated
                    .client_certificates
                    .first()
                    .expect("a CertificateVerify is only expected after a non-empty chain");
                let leaf = match Certificate::parse(leaf) {
                    Ok(leaf) => leaf,
                    Err(err) => {
                        self.state = State::Failed;
                        return Err(ServerError::MalformedClientCertificate(err));
                    }
                };
                // The *client* context string, over the transcript as of the
                // client's Certificate. §4.4.3 gives the two directions
                // different strings precisely so this signature cannot be a
                // replayed server one.
                let content = certificate_verify_content(
                    CLIENT_CERTIFICATE_VERIFY_CONTEXT,
                    &negotiated.client_certificate_transcript,
                );
                if let Err(err) = verify_tls13_signature(
                    SignatureScheme(verify.scheme),
                    &leaf.subject_public_key_info(),
                    &content,
                    verify.signature,
                ) {
                    self.state = State::Failed;
                    return Err(ServerError::ClientCertificateVerify(err));
                }
                negotiated.transcript.add_message(message);
                negotiated.expect = ExpectFromClient::Finished;
                Ok(Vec::new())
            }
            (
                ExpectFromClient::Finished | ExpectFromClient::FinishedAfterEmptyCertificate,
                HandshakeType::Finished,
            ) => self.finished(message),
            (expected, got) => {
                self.state = State::Failed;
                Err(ServerError::UnexpectedMessage {
                    expected: expected.name(),
                    got,
                })
            }
        }
    }

    fn finished(&mut self, message: &Message<'_>) -> Result<Vec<u8>> {
        let State::AwaitFinished(negotiated) = core::mem::replace(&mut self.state, State::Failed)
        else {
            return Err(ServerError::Failed);
        };
        let mut negotiated = *negotiated;

        let verify_data = parse_finished(message.body)?;
        if !verify_finished(
            negotiated.hash,
            &negotiated.client_handshake_secret,
            &negotiated.transcript.hash(),
            verify_data,
        ) {
            return Err(ServerError::BadFinished);
        }

        let client_keys = traffic_keys(
            negotiated.hash,
            &negotiated.client_application_secret,
            negotiated.aead.key_len(),
        );
        let server_keys = traffic_keys(
            negotiated.hash,
            &negotiated.server_application_secret,
            negotiated.aead.key_len(),
        );
        let mut sealer = Sealer::new(negotiated.aead, &server_keys.key, &server_keys.iv)?;

        // `res master` covers the transcript through the *client's* Finished,
        // so it can only be derived once that message is in — every other
        // secret in this handshake is bound to an earlier point. The client
        // half derives its half of this from the same message, and the two
        // agreeing is what makes a ticket redeemable.
        negotiated.transcript.add_message(message);
        let out = self.issue_tickets(&negotiated, &mut sealer)?;

        self.state = State::Done(Box::new(Connection::from_parts(
            negotiated.aead,
            negotiated.hash,
            negotiated.suite,
            sealer,
            Opener::new(negotiated.aead, &client_keys.key, &client_keys.iv)?,
            negotiated.server_application_secret,
            negotiated.client_application_secret,
            negotiated.client_certificates,
        )));
        Ok(out)
    }

    /// Build the NewSessionTickets this handshake earned, sealed under the
    /// application key.
    ///
    /// Empty when no [`Tickets`] is configured, which is the posture this
    /// server had before `rusty_tls#43` and remains a legitimate one.
    ///
    /// Each ticket gets its own nonce, and the nonce is what makes two tickets
    /// from one connection two different keys rather than one key issued
    /// twice. A server that reused it would hand out `n` copies of a single
    /// PSK and quietly undo the reason for issuing more than one.
    ///
    /// `sealer` is the connection's own, used here and then handed on, so the
    /// sequence number the connection starts from is the one that follows
    /// these records rather than zero again.
    fn issue_tickets(&self, negotiated: &Negotiated, sealer: &mut Sealer) -> Result<Vec<u8>> {
        let Some(tickets) = self.config.tickets else {
            return Ok(Vec::new());
        };
        if tickets.count == 0 {
            return Ok(Vec::new());
        }

        let resumption_master = negotiated
            .master
            .derive("res master", &negotiated.transcript.hash());
        let identity = TicketContents::identity_of(self.config.certificates);

        let mut flight = Vec::new();
        for _ in 0..tickets.count {
            let nonce = random_bytes(32)?;
            let psk = expand_label(
                negotiated.hash,
                &resumption_master,
                "resumption",
                &nonce,
                negotiated.hash.len(),
            );
            let sealed = TicketContents {
                suite: negotiated.suite,
                issued_at: tickets.now,
                lifetime: tickets.lifetime,
                identity: identity.clone(),
                psk,
            }
            .seal(tickets.keys.current)
            .map_err(ServerError::Ticket)?;

            // `ticket_age_add` is fresh per ticket and random: it is what a
            // client adds to the ticket's real age before reporting it, so a
            // predictable one would let an observer correlate two connections
            // by the difference between the ages they report.
            let age_add = u32::from_be_bytes(
                random_bytes(4)?
                    .try_into()
                    .expect("four random octets are four octets"),
            );

            let mut body = Writer::new();
            body.u32(tickets.lifetime);
            body.u32(age_add);
            body.vector_u8(|w| w.bytes(&nonce));
            body.vector_u16(|w| w.bytes(&sealed));
            // No extensions. The only one defined for this message is
            // `early_data`, and ADR-0003 puts 0-RTT out of scope — offering a
            // `max_early_data_size` would be advertising a feature this server
            // refuses in the next handshake.
            body.vector_u16(|_| {});
            flight.extend_from_slice(&Message::encode(
                HandshakeType::NewSessionTicket,
                &body.into_vec(),
            ));
        }

        Ok(sealer.seal(ContentType::Handshake, &flight, 0)?)
    }
}

fn random_bytes(len: usize) -> Result<Vec<u8>> {
    use ring::rand::SecureRandom;
    let mut out = vec![0u8; len];
    ring::rand::SystemRandom::new()
        .fill(&mut out)
        .map_err(|_| ServerError::Random)?;
    Ok(out)
}

fn certificate_message(chain: &[Vec<u8>]) -> Vec<u8> {
    let mut body = Writer::new();
    body.vector_u8(|_| {}); // certificate_request_context: empty
    body.vector_u24(|w| {
        for certificate in chain {
            w.vector_u24(|w| w.bytes(certificate));
            w.vector_u16(|_| {}); // per-entry extensions
        }
    });
    Message::encode(HandshakeType::Certificate, &body.into_vec())
}

/// Says nothing about key material.
impl core::fmt::Debug for ServerHandshake<'_> {
    fn fmt(&self, f: &mut core::fmt::Formatter<'_>) -> core::fmt::Result {
        let state = match &self.state {
            State::AwaitClientHello => "AwaitClientHello",
            State::AwaitRetriedClientHello(_) => "AwaitClientHello(after a retry)",
            State::AwaitFinished(_) => "AwaitFinished",
            State::Done(_) => "Done",
            State::Failed => "Failed",
        };
        f.debug_struct("ServerHandshake")
            .field("state", &state)
            .field("buffered", &self.buffer.len())
            .finish()
    }
}

/// What a server's [`Connection`] produces on a read.
///
/// The same type the client uses: a connection is symmetric once the
/// handshake is over, and having two of these would be two places to get a
/// KeyUpdate wrong.
pub use super::client::Incoming;
