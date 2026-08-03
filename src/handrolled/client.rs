//! The TLS 1.3 client handshake — stage 3c-ii.
//!
//! Every stage before this one was a function over bytes: same input, same
//! output, no clock, no peer, no memory. This one has state, and its failures
//! are of a different kind. A parser can be wrong about *content*; a state
//! machine can be wrong about *sequence* — accepting a flight in the wrong
//! order, or with a message quietly missing — and the result still looks like
//! a working connection.
//!
//! The dangerous omission is specific and worth naming. A server's Certificate
//! proves nothing on its own: anyone can send anyone's certificate. What proves
//! the peer holds the matching private key is the CertificateVerify, and a
//! client that accepted a flight without one would complete a handshake with an
//! attacker who copied a certificate off the wire. That is why the expected
//! message is a field of the state rather than a `match` on whatever arrived:
//! the private `Expect` enum names the one message that will be accepted, and
//! everything else is refused by default rather than by remembering to.
//!
//! # Sans-IO
//!
//! Nothing here opens a socket. [`ClientHandshake::read_record`] takes one
//! whole TLS record and returns the bytes to send back, which is the same
//! shape `rustls::ClientConnection` has and for the same reason: the transport
//! is the caller's business, and a handshake that owns its own IO cannot be
//! driven by a test.
//!
//! Splitting a byte stream into records is the caller's job too, with
//! [`record_length`] to find the boundaries. That division is the record
//! layer's own: [`super::record`] protects a record, and says in its docs that
//! finding one is a layer above.
//!
//! # What is deliberately not supported
//!
//! Refused, with an error, rather than half-implemented:
//!
//! - **Session resumption, PSK, and 0-RTT.** No ticket is ever used, and the
//!   ClientHello does not offer `psk_key_exchange_modes` — so by RFC 8446
//!   §4.2.9 a conforming server will never send a NewSessionTicket at all.
//!   [`Connection::read`] handles one anyway. That is not dead code being
//!   optimistic: the cost of being wrong is that an unexpected handshake
//!   record gets handed to the caller as application data, which turns a
//!   protocol surprise into silent data corruption. Discarding it is the
//!   cheap, safe answer to a message that should not arrive.
//! - **TLS 1.2 and below.** `supported_versions` offers exactly `0x0304`, and a
//!   ServerHello that does not select it is refused. Stage 4 is where a
//!   fallback would go, if one is ever wanted.
//!
//! # Client certificates
//!
//! Implemented, as of `rusty_tls#42`. A CertificateRequest is answered rather
//! than aborted on: with a [`ClientIdentity`] configured, the client sends its
//! chain and a CertificateVerify signed with the **client** context string of
//! §4.4.3 — a different string from the server's, so a signature made here can
//! never be replayed as a server's.
//!
//! With no identity, or none matching the schemes the server named, the answer
//! is an empty Certificate and no CertificateVerify. §4.4.2 makes that the
//! conforming way to say "I have nothing", and it leaves the decision with the
//! server, which is whose decision it is. A client that aborted instead would
//! be refusing on the server's behalf, and would fail against every server
//! that asks for a certificate but does not insist on one.
//!
//! What is *not* honoured is the request's `certificate_authorities` and
//! `oid_filters`: they narrow which chain a server would prefer, and this
//! client sends the one it was configured with regardless. A server that
//! wanted a different one refuses, which is a worse error message than it
//! could be but not a wrong outcome.
//!
//! # HelloRetryRequest
//!
//! Implemented, including the transcript substitution RFC 8446 §4.4.1
//! requires: once a retry happens, the running hash starts from a synthetic
//! `message_hash` message wrapping `Hash(ClientHello1)` rather than from
//! ClientHello1 itself. Getting that wrong produces a client that works
//! perfectly until it meets a server that asks for a different group, and then
//! fails with a decrypt error that looks like anything but a transcript bug.

use ring::rand::{SecureRandom, SystemRandom};

use super::handshake::{
    certificate_verify_content, complete_prefix, extension, find, messages,
    parse_encrypted_extensions, parse_finished, CertificateMessage, CertificateRequestMessage,
    CertificateVerify, ClientHello, Extension, HandshakeError, HandshakeType, Message, ServerHello,
    Transcript, CLIENT_CERTIFICATE_VERIFY_CONTEXT, SERVER_CERTIFICATE_VERIFY_CONTEXT,
};
use super::kx::{KeyExchange, KxError, NamedGroup};
use super::name::ServerName;
use super::path::{verify_peer_certificate, PathError, PathOptions, TrustAnchor};
use super::record::{
    Aead, ContentType, Opener, RecordError, Sealer, HEADER_LEN, MAX_ENCRYPTED_FRAGMENT_LEN,
};
use super::schedule::{
    finished_verify_data, traffic_keys, update_traffic_secret, verify_finished, Hash, KeySchedule,
};
use super::sign::{SignError, SigningKey};
use super::verify::{verify_tls13_signature, SignatureScheme, VerifyError};
use super::wire::Writer;
use super::x509::Certificate;

/// A TLS alert's severity (RFC 8446 §6).
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
#[non_exhaustive]
pub enum AlertLevel {
    /// `warning(1)`. In TLS 1.3 only `close_notify` and `user_canceled` are
    /// warnings; everything else is fatal whatever the level says.
    Warning,
    /// `fatal(2)`.
    Fatal,
    /// Any other value, preserved.
    Unknown(u8),
}

impl AlertLevel {
    /// Decode a wire value. Shared with [`super::server`], which reads alerts
    /// from the other direction.
    pub(crate) const fn from_wire(value: u8) -> Self {
        Self::from_u8(value)
    }

    const fn from_u8(value: u8) -> Self {
        match value {
            1 => Self::Warning,
            2 => Self::Fatal,
            other => Self::Unknown(other),
        }
    }
}

/// A TLS alert's description.
///
/// A newtype over the wire value rather than an enum, for the same reason
/// [`super::verify::SignatureScheme`] is one: the registry is open, and a peer
/// can send a number this code has never heard of. Reporting it verbatim is
/// more useful than collapsing it to "unknown".
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct AlertDescription(pub u8);

impl AlertDescription {
    /// `close_notify(0)` — an orderly shutdown, not a failure.
    pub const CLOSE_NOTIFY: Self = Self(0);
    /// `bad_record_mac(20)`.
    pub const BAD_RECORD_MAC: Self = Self(20);
    /// `handshake_failure(40)`.
    pub const HANDSHAKE_FAILURE: Self = Self(40);
    /// `bad_certificate(42)`.
    pub const BAD_CERTIFICATE: Self = Self(42);
    /// `certificate_expired(45)`.
    pub const CERTIFICATE_EXPIRED: Self = Self(45);
    /// `certificate_unknown(46)`.
    pub const CERTIFICATE_UNKNOWN: Self = Self(46);
    /// `illegal_parameter(47)`.
    pub const ILLEGAL_PARAMETER: Self = Self(47);
    /// `unknown_ca(48)`.
    pub const UNKNOWN_CA: Self = Self(48);
    /// `decode_error(50)`.
    pub const DECODE_ERROR: Self = Self(50);
    /// `decrypt_error(51)`.
    pub const DECRYPT_ERROR: Self = Self(51);
    /// `protocol_version(70)` — the one a peer sends when it cannot speak the
    /// version that was offered. See the module docs on TLS 1.2.
    pub const PROTOCOL_VERSION: Self = Self(70);
    /// `inappropriate_fallback(86)`.
    pub const INAPPROPRIATE_FALLBACK: Self = Self(86);
    /// `certificate_required(116)` — a server that asked for a client
    /// certificate and was given none.
    pub const CERTIFICATE_REQUIRED: Self = Self(116);
    /// `no_application_protocol(120)`.
    pub const NO_APPLICATION_PROTOCOL: Self = Self(120);

    /// The registry name, where this code knows one.
    pub const fn name(self) -> Option<&'static str> {
        Some(match self.0 {
            0 => "close_notify",
            10 => "unexpected_message",
            20 => "bad_record_mac",
            22 => "record_overflow",
            40 => "handshake_failure",
            42 => "bad_certificate",
            43 => "unsupported_certificate",
            44 => "certificate_revoked",
            45 => "certificate_expired",
            46 => "certificate_unknown",
            47 => "illegal_parameter",
            48 => "unknown_ca",
            49 => "access_denied",
            50 => "decode_error",
            51 => "decrypt_error",
            70 => "protocol_version",
            71 => "insufficient_security",
            80 => "internal_error",
            86 => "inappropriate_fallback",
            90 => "user_canceled",
            109 => "missing_extension",
            110 => "unsupported_extension",
            112 => "unrecognized_name",
            116 => "certificate_required",
            120 => "no_application_protocol",
            _ => return None,
        })
    }
}

impl core::fmt::Display for AlertDescription {
    fn fmt(&self, f: &mut core::fmt::Formatter<'_>) -> core::fmt::Result {
        match self.name() {
            Some(name) => write!(f, "{name}({})", self.0),
            None => write!(f, "alert {}", self.0),
        }
    }
}

/// An alert the peer sent.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Alert {
    /// The severity the peer claimed.
    pub level: AlertLevel,
    /// What the peer objected to.
    pub description: AlertDescription,
}

impl Alert {
    /// Parse an alert body, which RFC 8446 §6 fixes at two octets.
    fn parse(body: &[u8]) -> Option<Self> {
        match body {
            [level, description] => Some(Self {
                level: AlertLevel::from_u8(*level),
                description: AlertDescription(*description),
            }),
            _ => None,
        }
    }
}

/// Everything the client handshake can refuse.
#[derive(Clone, Debug, PartialEq, Eq)]
#[non_exhaustive]
pub enum ClientError {
    /// A handshake message was malformed.
    Handshake(HandshakeError),
    /// A record was malformed or did not decrypt.
    Record(RecordError),
    /// The key exchange failed.
    Kx(KxError),
    /// Signing this client's own CertificateVerify failed.
    ///
    /// Only reachable when a client certificate is configured — a client with
    /// no identity never signs anything.
    Sign(SignError),
    /// The peer's certificate did not validate, or was not for this server.
    Path(PathError),
    /// The CertificateVerify signature did not check out.
    Verify(VerifyError),
    /// A handshake message arrived where a different one was required.
    ///
    /// The variant this whole module exists to be able to return. See the
    /// module docs on why a missing CertificateVerify is the case that matters.
    UnexpectedMessage {
        /// What the state machine required.
        expected: &'static str,
        /// What arrived.
        got: HandshakeType,
    },
    /// A record arrived carrying content the handshake has no use for.
    UnexpectedContentType(ContentType),
    /// The peer sent an alert.
    ///
    /// Worth its own variant rather than collapsing into "the handshake
    /// failed", because it is the only error carrying the *peer's* opinion of
    /// what went wrong. A server that cannot speak TLS 1.3 says
    /// `protocol_version` here, which is the difference between "something
    /// broke" and "this server is too old for this client".
    PeerAlert(Alert),
    /// The server sent a ServerHello whose `random` carries the RFC 8446
    /// §4.1.3 downgrade sentinel.
    ///
    /// The sentinel means the server *does* support TLS 1.3 but negotiated
    /// something older — which, since this client offers nothing older, means
    /// the ClientHello it saw was not the one that was sent.
    ///
    /// This changes which error is returned, not whether the handshake is
    /// refused: a ServerHello without `supported_versions` is
    /// [`ClientError::NotTls13`] regardless. What it buys is the distinction
    /// between an old server and an active downgrade, which are the same
    /// bytes and very different problems.
    DowngradeDetected,
    /// The ServerHello did not echo the `legacy_session_id` that was sent.
    ///
    /// RFC 8446 §4.1.3 requires the echo and §4.1.3 requires a client to abort
    /// if it is wrong. It is a cheap binding between the ClientHello that was
    /// sent and the ServerHello that came back, and a client that skipped it
    /// would accept a reply to somebody else's hello.
    SessionIdMismatch,
    /// The server did not select TLS 1.3.
    ///
    /// Covers both a missing `supported_versions` in the ServerHello and one
    /// naming another version. A TLS 1.2 server reaching this point is a
    /// downgrade, whether or not it intended one.
    NotTls13,
    /// The server selected a cipher suite the client did not offer.
    UnofferedCipherSuite(u16),
    /// The server's `key_share` named a group the client did not offer, or
    /// none at all.
    BadKeyShare,
    /// The server sent a second HelloRetryRequest.
    ///
    /// RFC 8446 §4.1.4: a client MUST abort rather than retry twice. Two is
    /// all it takes to make a server able to loop a client indefinitely.
    RepeatedHelloRetryRequest,
    /// A HelloRetryRequest asked for a group the client did not offer.
    UnofferedGroup(u16),
    /// The server's Finished did not verify.
    ///
    /// The handshake was tampered with, or the peer did not derive the same
    /// keys. Indistinguishable from outside, and both are fatal.
    BadFinished,
    /// The Certificate message carried no certificates.
    NoCertificates,
    /// A certificate in the chain did not parse.
    ///
    /// Its own variant rather than a [`PathError`], because path validation
    /// takes certificates that have already been parsed — reading DER off the
    /// wire is this module's step, and a peer sending rubbish is a different
    /// event from a chain that does not validate.
    MalformedCertificate(super::x509::X509Error),
    /// A record arrived after the connection was already broken.
    ///
    /// Once a handshake fails it stays failed; there is no path back that does
    /// not risk continuing with half-established state.
    Failed,
    /// The system random source failed.
    Random,
}

impl core::fmt::Display for ClientError {
    fn fmt(&self, f: &mut core::fmt::Formatter<'_>) -> core::fmt::Result {
        match self {
            Self::Handshake(err) => write!(f, "malformed handshake message: {err}"),
            Self::Record(err) => write!(f, "record layer: {err}"),
            Self::Kx(err) => write!(f, "key exchange: {err}"),
            Self::Sign(err) => write!(f, "signing this client's CertificateVerify: {err}"),
            Self::Path(err) => write!(f, "certificate: {err}"),
            Self::Verify(err) => write!(f, "handshake signature: {err}"),
            Self::UnexpectedMessage { expected, got } => {
                write!(f, "expected {expected}, got {got:?}")
            }
            Self::UnexpectedContentType(typ) => {
                write!(f, "unexpected content type {typ:?} during the handshake")
            }
            Self::PeerAlert(alert) => write!(
                f,
                "the peer sent a {:?} alert: {}",
                alert.level, alert.description
            ),
            Self::DowngradeDetected => f.write_str(
                "the server signalled a TLS 1.3 downgrade, so the ClientHello it saw was not the one sent",
            ),
            Self::SessionIdMismatch => {
                f.write_str("the server did not echo the session id that was sent")
            }
            Self::NotTls13 => f.write_str("the server did not select TLS 1.3"),
            Self::UnofferedCipherSuite(suite) => {
                write!(
                    f,
                    "the server selected unoffered cipher suite 0x{suite:04x}"
                )
            }
            Self::BadKeyShare => f.write_str("the server's key_share is missing or unusable"),
            Self::RepeatedHelloRetryRequest => f.write_str("a second HelloRetryRequest"),
            Self::UnofferedGroup(group) => {
                write!(f, "a retry asked for unoffered group 0x{group:04x}")
            }
            Self::BadFinished => f.write_str("the server's Finished did not verify"),
            Self::NoCertificates => f.write_str("the server sent an empty certificate chain"),
            Self::MalformedCertificate(err) => write!(f, "a certificate did not parse: {err}"),
            Self::Failed => f.write_str("the connection already failed"),
            Self::Random => f.write_str("the system random source failed"),
        }
    }
}

impl std::error::Error for ClientError {}

impl From<HandshakeError> for ClientError {
    fn from(err: HandshakeError) -> Self {
        Self::Handshake(err)
    }
}
impl From<RecordError> for ClientError {
    fn from(err: RecordError) -> Self {
        Self::Record(err)
    }
}
impl From<KxError> for ClientError {
    fn from(err: KxError) -> Self {
        Self::Kx(err)
    }
}
impl From<SignError> for ClientError {
    fn from(err: SignError) -> Self {
        Self::Sign(err)
    }
}
impl From<PathError> for ClientError {
    fn from(err: PathError) -> Self {
        Self::Path(err)
    }
}
impl From<VerifyError> for ClientError {
    fn from(err: VerifyError) -> Self {
        Self::Verify(err)
    }
}

type Result<T> = core::result::Result<T, ClientError>;

// ---------------------------------------------------------------------------
// Cipher suites
// ---------------------------------------------------------------------------

/// A TLS 1.3 cipher suite: an AEAD and a hash, chosen together.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct CipherSuite(pub u16);

impl CipherSuite {
    /// `TLS_AES_128_GCM_SHA256`, which every TLS 1.3 implementation must have.
    pub const TLS_AES_128_GCM_SHA256: Self = Self(0x1301);
    /// `TLS_AES_256_GCM_SHA384`.
    pub const TLS_AES_256_GCM_SHA384: Self = Self(0x1302);
    /// `TLS_CHACHA20_POLY1305_SHA256`.
    pub const TLS_CHACHA20_POLY1305_SHA256: Self = Self(0x1303);

    /// The suites this client offers, strongest-preference first.
    pub const SUPPORTED: &'static [Self] = &[
        Self::TLS_AES_256_GCM_SHA384,
        Self::TLS_AES_128_GCM_SHA256,
        Self::TLS_CHACHA20_POLY1305_SHA256,
    ];

    /// The AEAD and hash this suite names, or `None` if it names neither.
    ///
    /// The key length comes from the AEAD and the secret length from the hash,
    /// and `TLS_AES_128_GCM_SHA256` is exactly the case where they differ —
    /// a 16-byte key from a 32-byte secret. Deriving one from the other would
    /// work for two of these three.
    pub const fn parts(self) -> Option<(Aead, Hash)> {
        match self.0 {
            0x1301 => Some((Aead::Aes128Gcm, Hash::Sha256)),
            0x1302 => Some((Aead::Aes256Gcm, Hash::Sha384)),
            0x1303 => Some((Aead::ChaCha20Poly1305, Hash::Sha256)),
            _ => None,
        }
    }
}

// ---------------------------------------------------------------------------
// Configuration
// ---------------------------------------------------------------------------

/// What a client needs to know before it can start.
///
/// Every field is required. There is no `Default`, because a default trust
/// anchor set or a default server name is the sort of convenience that ends up
/// authenticating nothing.
pub struct ClientConfig<'a> {
    /// The server being connected to, matched against the certificate.
    pub server_name: ServerName<'a>,
    /// The trust anchors the chain must reach.
    pub anchors: &'a [TrustAnchor<'a>],
    /// Path validation options, including the current time.
    pub path: PathOptions,
    /// The groups to offer, most preferred first. A `key_share` is sent for
    /// the first; the rest are offered so a server can ask for one by
    /// HelloRetryRequest.
    pub groups: &'a [NamedGroup],
    /// The cipher suites to offer, most preferred first.
    pub cipher_suites: &'a [CipherSuite],
    /// What to present if the server asks the client to authenticate.
    ///
    /// `None` means "nothing to offer", which is **not** the same as refusing:
    /// a client with no identity answers a CertificateRequest with an empty
    /// Certificate message, which RFC 8446 §4.4.2 makes the conforming way to
    /// say so. The server then decides whether that is acceptable — which is
    /// the server's decision to make, not the client's to pre-empt by aborting.
    pub identity: Option<&'a ClientIdentity<'a>>,
}

/// A certificate chain and the key that goes with it, for client
/// authentication.
///
/// Separate from [`ClientConfig`] so that the common case — no client
/// certificate — costs nothing to express and cannot be half-configured. A
/// chain without a key, or a key without a chain, is not representable.
pub struct ClientIdentity<'a> {
    /// The chain to present, DER-encoded, end-entity first.
    pub certificates: &'a [Vec<u8>],
    /// The private key for the end-entity certificate.
    pub key: &'a SigningKey,
}

/// A CertificateRequest, reduced to what answering it needs.
struct CertificateRequest {
    /// Echoed verbatim in the client's Certificate — §4.4.2 requires it, and
    /// it is how a server ties the answer to the question when it asks more
    /// than once.
    context: Vec<u8>,
    /// The schemes the server will accept a client signature in.
    schemes: Vec<u16>,
}

// ---------------------------------------------------------------------------
// Record framing
// ---------------------------------------------------------------------------

/// The total length of the record starting at `input`, header included.
///
/// `None` when fewer than [`super::record::HEADER_LEN`] bytes are present, so
/// a caller reading a stream can use it to decide whether a whole record has
/// arrived. Deliberately does not validate the type or version — that is
/// [`ClientHandshake::read_record`]'s job, and a deframer that also judged
/// would be two things.
pub fn record_length(input: &[u8]) -> Option<usize> {
    let header = input.get(..HEADER_LEN)?;
    Some(HEADER_LEN + usize::from(u16::from_be_bytes([header[3], header[4]])))
}

/// Frame a fragment as an unprotected record.
///
/// Only the first flight is unprotected, so `version` is 0x0301 there and
/// 0x0303 afterwards. RFC 8446 §5.1 says the field MUST be ignored on receipt,
/// so this matches convention rather than a requirement.
fn plaintext_record(typ: ContentType, version: u16, fragment: &[u8]) -> Vec<u8> {
    let mut out = Vec::with_capacity(HEADER_LEN + fragment.len());
    out.push(typ.as_u8());
    out.extend_from_slice(&version.to_be_bytes());
    out.extend_from_slice(&(fragment.len() as u16).to_be_bytes());
    out.extend_from_slice(fragment);
    out
}

/// The one-octet record servers send in middlebox-compatibility mode.
const CHANGE_CIPHER_SPEC: u8 = 20;

/// The last eight octets a TLS 1.3-capable server puts in `ServerHello.random`
/// when it negotiates TLS 1.2 anyway (RFC 8446 §4.1.3). The `00` variant marks
/// TLS 1.1 or below.
const DOWNGRADE_SENTINEL_TLS12: [u8; 8] = [0x44, 0x4f, 0x57, 0x4e, 0x47, 0x52, 0x44, 0x01];
/// As [`DOWNGRADE_SENTINEL_TLS12`], for TLS 1.1 and below.
const DOWNGRADE_SENTINEL_OLDER: [u8; 8] = [0x44, 0x4f, 0x57, 0x4e, 0x47, 0x52, 0x44, 0x00];

/// True if a `ServerHello.random` carries either downgrade sentinel.
fn is_downgrade_sentinel(random: &[u8]) -> bool {
    match random.len().checked_sub(8).map(|at| &random[at..]) {
        Some(tail) => tail == DOWNGRADE_SENTINEL_TLS12 || tail == DOWNGRADE_SENTINEL_OLDER,
        None => false,
    }
}

// ---------------------------------------------------------------------------
// State
// ---------------------------------------------------------------------------

/// The message the state machine will accept next, and nothing else.
///
/// A field rather than a `match` on what arrived: the difference is whether
/// "the server skipped CertificateVerify" is a case someone has to remember to
/// write, or the default for every message that is not the one expected.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum Expect {
    EncryptedExtensions,
    Certificate,
    CertificateVerify,
    Finished,
}

impl Expect {
    const fn name(self) -> &'static str {
        match self {
            Self::EncryptedExtensions => "EncryptedExtensions",
            Self::Certificate => "Certificate",
            Self::CertificateVerify => "CertificateVerify",
            Self::Finished => "Finished",
        }
    }
}

/// Everything that exists only once a ServerHello has been accepted.
struct Negotiated {
    suite: CipherSuite,
    aead: Aead,
    hash: Hash,
    transcript: Transcript,
    schedule: KeySchedule,
    client_handshake_secret: Vec<u8>,
    server_handshake_secret: Vec<u8>,
    opener: Opener,
    /// The peer's chain, as DER.
    ///
    /// Stored encoded rather than parsed because a [`Certificate`] borrows the
    /// bytes it was parsed from, and keeping both in one struct would make it
    /// self-referential. Re-parsing at the point of use costs a microsecond
    /// and keeps the lifetime honest.
    certificates: Vec<Vec<u8>>,
    /// The transcript hash as of the end of the Certificate message, which is
    /// what the CertificateVerify signature covers. Captured when the
    /// Certificate is added rather than recomputed later, so a message
    /// arriving in between cannot silently change what was signed.
    certificate_transcript: Vec<u8>,
    /// Set if the server asked this client to authenticate.
    ///
    /// `None` is the overwhelmingly common case and means the client sends no
    /// Certificate at all — not an empty one. An unsolicited Certificate is a
    /// protocol violation, so "was not asked" and "was asked and has nothing"
    /// have to be different states rather than one nullable chain.
    certificate_request: Option<CertificateRequest>,
}

enum State {
    AwaitServerHello {
        client_hello: Vec<u8>,
        kx: KeyExchange,
        retried: bool,
        /// Kept so a HelloRetryRequest can reuse them: RFC 8446 §4.1.2 lists
        /// what a second ClientHello may change, and `random` and
        /// `legacy_session_id` are not on the list.
        random: Vec<u8>,
        session_id: Vec<u8>,
    },
    InFlight {
        expect: Expect,
        negotiated: Box<Negotiated>,
    },
    Done(Box<Connection>),
    Failed,
}

// ---------------------------------------------------------------------------
// The handshake
// ---------------------------------------------------------------------------

/// A TLS 1.3 client handshake in progress.
pub struct ClientHandshake<'a> {
    config: &'a ClientConfig<'a>,
    state: State,
    /// Handshake bytes reassembled across records. Messages may span records
    /// and several may share one, so neither boundary lines up with the other.
    buffer: Vec<u8>,
}

impl<'a> ClientHandshake<'a> {
    /// Start a handshake, returning it and the ClientHello record to send.
    pub fn start(config: &'a ClientConfig<'a>) -> Result<(Self, Vec<u8>)> {
        let group = *config.groups.first().ok_or(ClientError::BadKeyShare)?;
        let kx = KeyExchange::generate(group)?;
        let (hello, random, session_id) = build_client_hello(config, &kx, None)?;

        let record = plaintext_record(ContentType::Handshake, 0x0301, &hello);
        Ok((
            Self {
                config,
                state: State::AwaitServerHello {
                    client_hello: hello,
                    kx,
                    retried: false,
                    random,
                    session_id,
                },
                buffer: Vec::new(),
            },
            record,
        ))
    }

    /// True once the handshake has completed and [`Self::into_connection`]
    /// will succeed.
    pub fn is_finished(&self) -> bool {
        matches!(self.state, State::Done(_))
    }

    /// Take the established connection.
    pub fn into_connection(self) -> Result<Connection> {
        match self.state {
            State::Done(connection) => Ok(*connection),
            _ => Err(ClientError::Failed),
        }
    }

    /// Feed one whole TLS record, and get back the bytes to send in reply.
    ///
    /// `record` must be exactly one record, header included — use
    /// [`record_length`] to find where it ends. The reply is empty for every
    /// record except the one carrying the server's Finished, which is answered
    /// with the client's own flight.
    ///
    /// A failure is permanent. Every later call returns [`ClientError::Failed`]
    /// rather than continuing from a state that is half-established.
    pub fn read_record(&mut self, record: &[u8]) -> Result<Vec<u8>> {
        if matches!(self.state, State::Failed) {
            return Err(ClientError::Failed);
        }
        match self.read_record_inner(record) {
            Ok(reply) => Ok(reply),
            Err(err) => {
                self.state = State::Failed;
                Err(err)
            }
        }
    }

    fn read_record_inner(&mut self, record: &[u8]) -> Result<Vec<u8>> {
        if record.len() < HEADER_LEN {
            return Err(RecordError::Truncated {
                len: record.len(),
                min: HEADER_LEN,
            }
            .into());
        }

        // RFC 8446 §5: a change_cipher_spec received during the handshake must
        // be dropped without comment. Servers in middlebox-compatibility mode
        // send one, and a client that treated it as an error would fail
        // against a large fraction of the internet for no security benefit.
        if record[0] == CHANGE_CIPHER_SPEC {
            return Ok(Vec::new());
        }

        // An alert before the ServerHello is in the clear, and is the only way
        // a peer gets to say why it is refusing. Reporting it as an unexpected
        // content type would throw away the one piece of diagnosis the peer
        // offered — a server too old for this client says `protocol_version`
        // here, and nothing else in the exchange would tell you that.
        if ContentType::from_u8(record[0]) == ContentType::Alert
            && matches!(self.state, State::AwaitServerHello { .. })
        {
            return Err(match Alert::parse(&record[HEADER_LEN..]) {
                Some(alert) => ClientError::PeerAlert(alert),
                None => ClientError::UnexpectedContentType(ContentType::Alert),
            });
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
            // Before the ServerHello everything is in the clear, and a record
            // claiming to be protected cannot be: there is no key yet.
            State::AwaitServerHello { .. } => {
                if ContentType::from_u8(record[0]) != ContentType::Handshake {
                    return Err(ClientError::UnexpectedContentType(ContentType::from_u8(
                        record[0],
                    )));
                }
                record[HEADER_LEN..].to_vec()
            }
            State::InFlight { negotiated, .. } => {
                let opened = negotiated.opener.open(record)?;
                match opened.typ {
                    ContentType::Handshake => opened.fragment,
                    ContentType::Alert => {
                        return Err(match Alert::parse(&opened.fragment) {
                            Some(alert) => ClientError::PeerAlert(alert),
                            None => ClientError::UnexpectedContentType(ContentType::Alert),
                        })
                    }
                    other => return Err(ClientError::UnexpectedContentType(other)),
                }
            }
            State::Done(_) | State::Failed => return Err(ClientError::Failed),
        };

        self.buffer.extend_from_slice(&fragment);
        self.drain_buffer()
    }

    /// Process every whole message the buffer now holds.
    fn drain_buffer(&mut self) -> Result<Vec<u8>> {
        let mut reply = Vec::new();

        loop {
            let complete = complete_prefix(&self.buffer);
            if complete == 0 {
                return Ok(reply);
            }
            let consumed: Vec<u8> = self.buffer.drain(..complete).collect();

            for message in messages(&consumed)? {
                reply.extend_from_slice(&self.handle_message(&message)?);
            }
        }
    }

    fn handle_message(&mut self, message: &Message<'_>) -> Result<Vec<u8>> {
        match &mut self.state {
            State::AwaitServerHello { .. } => self.handle_server_hello(message),
            State::InFlight { .. } => self.handle_flight_message(message),
            State::Done(_) | State::Failed => Err(ClientError::Failed),
        }
    }
}

// ---------------------------------------------------------------------------
// ClientHello
// ---------------------------------------------------------------------------

fn random_bytes(len: usize) -> Result<Vec<u8>> {
    let mut out = vec![0u8; len];
    SystemRandom::new()
        .fill(&mut out)
        .map_err(|_| ClientError::Random)?;
    Ok(out)
}

/// Build the ClientHello body, header included.
///
/// `identity` is `Some` only for the second hello after a HelloRetryRequest,
/// carrying the first one's `random` and `legacy_session_id`. RFC 8446 §4.1.2
/// enumerates what a retried ClientHello may change and neither is on the
/// list — a client that generated fresh ones would be sending a different
/// hello than the one the server retried, which some servers accept and none
/// are obliged to.
fn build_client_hello(
    config: &ClientConfig<'_>,
    kx: &KeyExchange,
    identity: Option<(&[u8], &[u8])>,
) -> Result<(Vec<u8>, Vec<u8>, Vec<u8>)> {
    let (random, session_id) = match identity {
        Some((random, session_id)) => (random.to_vec(), session_id.to_vec()),
        // RFC 8446 §D.4: a client in middlebox-compatibility mode sends a
        // 32-octet session id it will never use, because middleboxes that
        // predate TLS 1.3 drop handshakes that look too unlike a resumption.
        None => (random_bytes(32)?, random_bytes(32)?),
    };

    let mut server_name = Writer::new();
    if let ServerName::Dns(name) = config.server_name {
        server_name.vector_u16(|w| {
            w.u8(0); // host_name
            w.vector_u16(|w| w.bytes(name.as_bytes()));
        });
    }

    let mut versions = Writer::new();
    versions.vector_u8(|w| w.u16(0x0304));

    let mut groups = Writer::new();
    groups.vector_u16(|w| {
        for group in config.groups {
            w.u16(group.as_u16());
        }
    });

    let mut schemes = Writer::new();
    schemes.vector_u16(|w| {
        for scheme in SignatureScheme::TLS13_SUPPORTED {
            w.u16(scheme.0);
        }
    });

    let mut key_share = Writer::new();
    key_share.vector_u16(|w| {
        w.u16(kx.group().as_u16());
        w.vector_u16(|w| w.bytes(kx.public_key()));
    });

    let (server_name, versions, groups, schemes, key_share) = (
        server_name.into_vec(),
        versions.into_vec(),
        groups.into_vec(),
        schemes.into_vec(),
        key_share.into_vec(),
    );

    let mut extensions = Vec::new();
    // RFC 6066 §3: an IP address is never sent as a server_name, so the
    // extension is absent entirely rather than present and empty.
    if !server_name.is_empty() {
        extensions.push(Extension {
            typ: extension::SERVER_NAME,
            data: &server_name,
        });
    }
    extensions.push(Extension {
        typ: extension::SUPPORTED_VERSIONS,
        data: &versions,
    });
    extensions.push(Extension {
        typ: extension::SUPPORTED_GROUPS,
        data: &groups,
    });
    extensions.push(Extension {
        typ: extension::SIGNATURE_ALGORITHMS,
        data: &schemes,
    });
    extensions.push(Extension {
        typ: extension::KEY_SHARE,
        data: &key_share,
    });

    let hello = ClientHello {
        random: &random,
        session_id: &session_id,
        cipher_suites: config.cipher_suites.iter().map(|s| s.0).collect(),
        extensions,
    };

    Ok((
        Message::encode(HandshakeType::ClientHello, &hello.encode()),
        random,
        session_id,
    ))
}

// ---------------------------------------------------------------------------
// ServerHello
// ---------------------------------------------------------------------------

/// The `key_share` entry in a ServerHello: a group and its key.
fn server_key_share(data: &[u8]) -> Result<(u16, &[u8])> {
    let mut reader = super::wire::Reader::new(data);
    let group = reader.u16().map_err(HandshakeError::Wire)?;
    let key = reader.vector_u16().map_err(HandshakeError::Wire)?;
    reader.finish().map_err(HandshakeError::Wire)?;
    Ok((group, key))
}

impl ClientHandshake<'_> {
    fn handle_server_hello(&mut self, message: &Message<'_>) -> Result<Vec<u8>> {
        if message.typ != HandshakeType::ServerHello {
            return Err(ClientError::UnexpectedMessage {
                expected: "ServerHello",
                got: message.typ,
            });
        }

        let State::AwaitServerHello {
            client_hello,
            kx,
            retried,
            random,
            session_id,
        } = core::mem::replace(&mut self.state, State::Failed)
        else {
            return Err(ClientError::Failed);
        };

        let hello = ServerHello::parse(message.body)?;

        // The selected suite must be one that was offered, and one this code
        // can actually use. A server naming something else is either confused
        // or steering.
        let suite = CipherSuite(hello.cipher_suite);
        if !self.config.cipher_suites.contains(&suite) {
            return Err(ClientError::UnofferedCipherSuite(hello.cipher_suite));
        }
        let (aead, hash) = suite
            .parts()
            .ok_or(ClientError::UnofferedCipherSuite(hello.cipher_suite))?;

        // TLS 1.3 is negotiated here and nowhere else. `legacy_version` is
        // pinned at 0x0303 for every version, so a server that omits
        // `supported_versions` is offering TLS 1.2.
        match find(&hello.extensions, extension::SUPPORTED_VERSIONS) {
            Some([0x03, 0x04]) => {}
            // No `supported_versions` means TLS 1.2 or older was selected.
            // Either way it is refused; the sentinel only decides which error,
            // and the distinction is worth drawing because the two look
            // identical on the wire and are very different problems. See
            // `ClientError::DowngradeDetected`.
            _ if is_downgrade_sentinel(hello.random) => return Err(ClientError::DowngradeDetected),
            _ => return Err(ClientError::NotTls13),
        }

        // Only now, because the field means something else below TLS 1.3: in a
        // TLS 1.2 ServerHello it is a resumption identifier, not an echo, so a
        // client that checked it first would report a mismatch for a server
        // whose only sin was being old.
        //
        // RFC 8446 §4.1.3 requires the echo, and it binds this ServerHello to
        // the ClientHello that was sent.
        if hello.session_id != session_id {
            return Err(ClientError::SessionIdMismatch);
        }

        if hello.is_hello_retry_request() {
            if retried {
                return Err(ClientError::RepeatedHelloRetryRequest);
            }
            return self.retry(client_hello, message, &hello, hash, &random, &session_id);
        }

        let share =
            find(&hello.extensions, extension::KEY_SHARE).ok_or(ClientError::BadKeyShare)?;
        let (group, peer_key) = server_key_share(share)?;
        // The group the server names must be the one whose share was sent.
        //
        // This is not what stops a mismatched share from producing a usable
        // secret — `agree` uses *this* client's group whatever the label says,
        // so a server that named the wrong one would simply derive a different
        // secret and fail its own Finished a moment later. What the check buys
        // is that the failure is `BadKeyShare` at the point of the
        // disagreement rather than `BadFinished` three messages further on,
        // which is the difference between a diagnosable bug and a mysterious
        // one. Claiming more for it would be claiming a defence that is not
        // there.
        if group != kx.group().as_u16() {
            return Err(ClientError::BadKeyShare);
        }

        let mut transcript = Transcript::new(hash);
        transcript.add(&client_hello);
        transcript.add(message.encoded);
        let hello_hash = transcript.hash();

        let schedule = kx.agree(peer_key, |secret| {
            KeySchedule::new(hash).into_handshake(secret)
        })?;

        let client_handshake_secret = schedule.derive("c hs traffic", &hello_hash);
        let server_handshake_secret = schedule.derive("s hs traffic", &hello_hash);

        let keys = traffic_keys(hash, &server_handshake_secret, aead.key_len());
        let opener = Opener::new(aead, &keys.key, &keys.iv)?;

        self.state = State::InFlight {
            expect: Expect::EncryptedExtensions,
            negotiated: Box::new(Negotiated {
                suite,
                aead,
                hash,
                transcript,
                schedule,
                client_handshake_secret,
                server_handshake_secret,
                opener,
                certificates: Vec::new(),
                certificate_transcript: Vec::new(),
                certificate_request: None,
            }),
        };
        Ok(Vec::new())
    }

    /// Handle a HelloRetryRequest: rebuild the ClientHello for the group the
    /// server asked for, and substitute the transcript RFC 8446 §4.4.1
    /// requires.
    #[allow(clippy::too_many_arguments)]
    fn retry(
        &mut self,
        client_hello: Vec<u8>,
        message: &Message<'_>,
        hello: &ServerHello<'_>,
        hash: Hash,
        random: &[u8],
        session_id: &[u8],
    ) -> Result<Vec<u8>> {
        let share =
            find(&hello.extensions, extension::KEY_SHARE).ok_or(ClientError::BadKeyShare)?;
        if share.len() != 2 {
            return Err(ClientError::BadKeyShare);
        }
        let wanted = u16::from_be_bytes([share[0], share[1]]);
        let group = NamedGroup::from_u16(wanted).ok_or(ClientError::UnofferedGroup(wanted))?;
        if !self.config.groups.contains(&group) {
            return Err(ClientError::UnofferedGroup(wanted));
        }

        let kx = KeyExchange::generate(group)?;
        let (second, random, session_id) =
            build_client_hello(self.config, &kx, Some((random, session_id)))?;

        // §4.4.1: once a retry has happened, the transcript begins with a
        // synthetic `message_hash` message wrapping Hash(ClientHello1) rather
        // than with ClientHello1 itself. The substitution exists so that a
        // server need not retain the first ClientHello, and getting it wrong
        // produces a client that works until it meets a server that retries.
        let mut synthetic = Vec::with_capacity(4 + hash.len());
        synthetic.push(254); // message_hash
        synthetic.extend_from_slice(&[0x00, 0x00, hash.len() as u8]);
        synthetic.extend_from_slice(&hash.hash(&client_hello));

        let mut replayed = synthetic;
        replayed.extend_from_slice(message.encoded);
        replayed.extend_from_slice(&second);

        self.state = State::AwaitServerHello {
            client_hello: replayed,
            kx,
            retried: true,
            random,
            session_id,
        };
        Ok(plaintext_record(ContentType::Handshake, 0x0303, &second))
    }
}

/// A client's Certificate message, RFC 8446 §4.4.2.
///
/// Unlike the server's, the `certificate_request_context` is not empty: it is
/// whatever the CertificateRequest carried, echoed back. An empty `chain` is
/// legitimate and means "nothing to offer".
fn client_certificate_message(context: &[u8], chain: &[Vec<u8>]) -> Vec<u8> {
    let mut body = Writer::new();
    body.vector_u8(|w| w.bytes(context));
    body.vector_u24(|w| {
        for certificate in chain {
            w.vector_u24(|w| w.bytes(certificate));
            w.vector_u16(|_| {}); // per-entry extensions
        }
    });
    Message::encode(HandshakeType::Certificate, &body.into_vec())
}

// ---------------------------------------------------------------------------
// The encrypted flight
// ---------------------------------------------------------------------------

impl ClientHandshake<'_> {
    fn handle_flight_message(&mut self, message: &Message<'_>) -> Result<Vec<u8>> {
        let State::InFlight { expect, negotiated } = &mut self.state else {
            return Err(ClientError::Failed);
        };

        // RFC 8446 §4.3.2. It sits between EncryptedExtensions and the
        // server's Certificate, so it is accepted exactly where a Certificate
        // is expected and leaves `expect` where it was — the server's
        // Certificate is still the next thing required, and a
        // CertificateRequest must not stand in for it.
        if message.typ == HandshakeType::CertificateRequest {
            if *expect != Expect::Certificate {
                return Err(ClientError::UnexpectedMessage {
                    expected: expect.name(),
                    got: message.typ,
                });
            }
            // Twice is not a longer request, it is a confused peer. Accepting
            // the second would let a server replace the question after the
            // client had already decided how to answer it.
            if negotiated.certificate_request.is_some() {
                return Err(ClientError::UnexpectedMessage {
                    expected: "Certificate",
                    got: message.typ,
                });
            }
            let request = CertificateRequestMessage::parse(message.body)?;
            negotiated.certificate_request = Some(CertificateRequest {
                context: request.context.to_vec(),
                schemes: request.schemes,
            });
            negotiated.transcript.add_message(message);
            return Ok(Vec::new());
        }

        let wanted = match (*expect, message.typ) {
            (Expect::EncryptedExtensions, HandshakeType::EncryptedExtensions)
            | (Expect::Certificate, HandshakeType::Certificate)
            | (Expect::CertificateVerify, HandshakeType::CertificateVerify)
            | (Expect::Finished, HandshakeType::Finished) => *expect,
            (expected, got) => {
                return Err(ClientError::UnexpectedMessage {
                    expected: expected.name(),
                    got,
                })
            }
        };

        match wanted {
            Expect::EncryptedExtensions => {
                parse_encrypted_extensions(message.body)?;
                negotiated.transcript.add_message(message);
                *expect = Expect::Certificate;
                Ok(Vec::new())
            }
            Expect::Certificate => {
                let certificate = CertificateMessage::parse(message.body)?;
                if certificate.entries.is_empty() {
                    return Err(ClientError::NoCertificates);
                }
                negotiated.certificates = certificate
                    .entries
                    .iter()
                    .map(|entry| entry.certificate.to_vec())
                    .collect();
                negotiated.transcript.add_message(message);
                // What the CertificateVerify will have signed, captured now.
                negotiated.certificate_transcript = negotiated.transcript.hash();
                *expect = Expect::CertificateVerify;
                Ok(Vec::new())
            }
            Expect::CertificateVerify => {
                let verify = CertificateVerify::parse(message.body)?;
                self.check_peer(&verify)?;
                let State::InFlight { expect, negotiated } = &mut self.state else {
                    return Err(ClientError::Failed);
                };
                negotiated.transcript.add_message(message);
                *expect = Expect::Finished;
                Ok(Vec::new())
            }
            Expect::Finished => self.finish(message),
        }
    }

    /// Validate the chain, the name, and the handshake signature.
    ///
    /// All three, in one place, because any one of them alone accepts an
    /// attacker: a valid chain for the wrong name, a right name from an
    /// untrusted issuer, and a certificate the peer does not hold the key for
    /// are three different ways to be talking to the wrong server.
    fn check_peer(&self, verify: &CertificateVerify<'_>) -> Result<()> {
        let State::InFlight { negotiated, .. } = &self.state else {
            return Err(ClientError::Failed);
        };

        let parsed: core::result::Result<Vec<Certificate<'_>>, _> = negotiated
            .certificates
            .iter()
            .map(|der| Certificate::parse(der))
            .collect();
        let parsed = parsed.map_err(ClientError::MalformedCertificate)?;
        let (leaf, intermediates) = parsed.split_first().ok_or(ClientError::NoCertificates)?;

        verify_peer_certificate(
            leaf,
            intermediates,
            self.config.anchors,
            &self.config.server_name,
            &self.config.path,
        )?;

        let content = certificate_verify_content(
            SERVER_CERTIFICATE_VERIFY_CONTEXT,
            &negotiated.certificate_transcript,
        );
        verify_tls13_signature(
            SignatureScheme(verify.scheme),
            &leaf.subject_public_key_info(),
            &content,
            verify.signature,
        )?;
        Ok(())
    }

    /// The server's Finished: check it, then send ours and install the
    /// application keys.
    fn finish(&mut self, message: &Message<'_>) -> Result<Vec<u8>> {
        let State::InFlight { negotiated, .. } = core::mem::replace(&mut self.state, State::Failed)
        else {
            return Err(ClientError::Failed);
        };
        let mut negotiated = *negotiated;

        let verify_data = parse_finished(message.body)?;
        if !verify_finished(
            negotiated.hash,
            &negotiated.server_handshake_secret,
            &negotiated.transcript.hash(),
            verify_data,
        ) {
            return Err(ClientError::BadFinished);
        }

        // The application secrets are bound to the transcript *through* the
        // server's Finished, so this has to happen after adding it and before
        // adding ours.
        negotiated.transcript.add_message(message);
        let after_server_finished = negotiated.transcript.hash();

        let master = negotiated.schedule.into_master();
        let client_application_secret = master.derive("c ap traffic", &after_server_finished);
        let server_application_secret = master.derive("s ap traffic", &after_server_finished);

        // If the server asked this client to authenticate, the answer goes
        // here: after the server's Finished, before ours. Both messages are in
        // the transcript ours covers, so the ordering is not cosmetic — a
        // Finished computed before them proves nothing about them.
        let mut flight = Vec::new();
        if let Some(request) = &negotiated.certificate_request {
            let identity = self.config.identity.filter(|identity| {
                identity
                    .key
                    .schemes()
                    .iter()
                    .any(|s| request.schemes.contains(&s.0))
            });

            let chain: &[Vec<u8>] = match identity {
                Some(identity) => identity.certificates,
                // §4.4.2: an empty certificate_list is how a client says it has
                // nothing to offer. Sending it, rather than aborting, leaves
                // the decision with the server — which is whose decision it is.
                None => &[],
            };
            let certificate = client_certificate_message(&request.context, chain);
            negotiated.transcript.add(&certificate);
            flight.extend_from_slice(&certificate);

            if let Some(identity) = identity {
                let scheme = *identity
                    .key
                    .schemes()
                    .iter()
                    .find(|scheme| request.schemes.contains(&scheme.0))
                    .expect("filtered on exactly this above");
                // The *client* context string. §4.4.3 gives the two directions
                // different ones so a signature made here cannot be replayed
                // as a server's, and vice versa.
                let content = certificate_verify_content(
                    CLIENT_CERTIFICATE_VERIFY_CONTEXT,
                    &negotiated.transcript.hash(),
                );
                let signature = identity.key.sign(scheme, &content)?;
                let mut verify = Writer::new();
                verify.u16(scheme.0);
                verify.vector_u16(|w| w.bytes(&signature));
                let verify = Message::encode(HandshakeType::CertificateVerify, &verify.into_vec());
                negotiated.transcript.add(&verify);
                flight.extend_from_slice(&verify);
            }
        }

        // Ours covers everything the server's did, plus the server's Finished,
        // plus anything this client just added above.
        let client_verify_data = finished_verify_data(
            negotiated.hash,
            &negotiated.client_handshake_secret,
            &negotiated.transcript.hash(),
        );
        let client_finished = Message::encode(HandshakeType::Finished, &client_verify_data);
        flight.extend_from_slice(&client_finished);

        let handshake_keys = traffic_keys(
            negotiated.hash,
            &negotiated.client_handshake_secret,
            negotiated.aead.key_len(),
        );
        let mut sealer = Sealer::new(negotiated.aead, &handshake_keys.key, &handshake_keys.iv)?;

        // Middlebox compatibility again: a bare change_cipher_spec ahead of
        // the first protected record the client sends.
        let mut reply = plaintext_record(ContentType::ChangeCipherSpec, 0x0303, &[0x01]);
        reply.extend_from_slice(&sealer.seal(ContentType::Handshake, &flight, 0)?);

        let application =
            |secret: &[u8]| traffic_keys(negotiated.hash, secret, negotiated.aead.key_len());
        let client_keys = application(&client_application_secret);
        let server_keys = application(&server_application_secret);

        self.state = State::Done(Box::new(Connection {
            aead: negotiated.aead,
            hash: negotiated.hash,
            suite: negotiated.suite,
            sealer: Sealer::new(negotiated.aead, &client_keys.key, &client_keys.iv)?,
            opener: Opener::new(negotiated.aead, &server_keys.key, &server_keys.iv)?,
            client_secret: client_application_secret,
            server_secret: server_application_secret,
            certificates: negotiated.certificates,
        }));
        Ok(reply)
    }
}

// ---------------------------------------------------------------------------
// The established connection
// ---------------------------------------------------------------------------

/// What a completed handshake leaves behind.
pub struct Connection {
    aead: Aead,
    hash: Hash,
    suite: CipherSuite,
    sealer: Sealer,
    opener: Opener,
    client_secret: Vec<u8>,
    server_secret: Vec<u8>,
    certificates: Vec<Vec<u8>>,
}

/// What came out of a record after the handshake.
#[derive(Clone, Debug, PartialEq, Eq)]
#[non_exhaustive]
pub enum Incoming {
    /// Application data.
    Application(Vec<u8>),
    /// A post-handshake message that needed no reply — a NewSessionTicket,
    /// which this client does not use but must tolerate.
    Handled,
    /// A post-handshake message answered with these bytes, which the caller
    /// must send: a KeyUpdate that requested one in return.
    Reply(Vec<u8>),
    /// The peer closed the connection in an orderly way (`close_notify`).
    ///
    /// Distinct from an error: a server that has finished sending says this,
    /// and a caller that treated it as a failure would report every completed
    /// response as broken.
    Closed,
}

impl Connection {
    /// Assemble a connection from an already-completed handshake.
    ///
    /// `pub(crate)` because the only legitimate way to obtain one is to finish
    /// a handshake — this exists so [`super::server`] can build the same type
    /// from the other side rather than duplicating the record-layer and
    /// post-handshake logic, which would be two places to get a KeyUpdate
    /// wrong.
    #[allow(clippy::too_many_arguments)]
    pub(crate) fn from_parts(
        aead: Aead,
        hash: Hash,
        suite: CipherSuite,
        sealer: Sealer,
        opener: Opener,
        send_secret: Vec<u8>,
        receive_secret: Vec<u8>,
        certificates: Vec<Vec<u8>>,
    ) -> Self {
        Self {
            aead,
            hash,
            suite,
            sealer,
            opener,
            client_secret: send_secret,
            server_secret: receive_secret,
            certificates,
        }
    }

    /// The cipher suite in use.
    pub const fn cipher_suite(&self) -> CipherSuite {
        self.suite
    }

    /// The peer's certificate chain, DER-encoded, end-entity first.
    pub fn peer_certificates(&self) -> &[Vec<u8>] {
        &self.certificates
    }

    /// Protect application data as one record.
    pub fn write(&mut self, data: &[u8]) -> Result<Vec<u8>> {
        Ok(self.sealer.seal(ContentType::ApplicationData, data, 0)?)
    }

    /// Unprotect one whole record.
    ///
    /// Handles the post-handshake messages a server may send at any time. A
    /// client that treated a NewSessionTicket as application data would hand
    /// its caller a ticket as if the server had sent it, which is how a
    /// protocol bug becomes a data-corruption bug.
    pub fn read(&mut self, record: &[u8]) -> Result<Incoming> {
        // A change_cipher_spec after the handshake is not permitted, but it
        // costs nothing to drop and some middleboxes still emit one.
        if record.first() == Some(&CHANGE_CIPHER_SPEC) {
            return Ok(Incoming::Handled);
        }

        let opened = self.opener.open(record)?;
        match opened.typ {
            ContentType::ApplicationData => Ok(Incoming::Application(opened.fragment)),
            ContentType::Handshake => self.post_handshake(&opened.fragment),
            ContentType::Alert => match Alert::parse(&opened.fragment) {
                // An orderly close is not a failure. Reporting it as one made
                // every completed HTTP response look broken, which the interop
                // suite worked around by treating an unexpected content type
                // as "the correct place to stop" — a missing feature described
                // as correct behaviour.
                Some(alert) if alert.description == AlertDescription::CLOSE_NOTIFY => {
                    Ok(Incoming::Closed)
                }
                Some(alert) => Err(ClientError::PeerAlert(alert)),
                None => Err(ClientError::UnexpectedContentType(ContentType::Alert)),
            },
            other => Err(ClientError::UnexpectedContentType(other)),
        }
    }

    fn post_handshake(&mut self, fragment: &[u8]) -> Result<Incoming> {
        let complete = complete_prefix(fragment);
        let mut reply = Vec::new();

        for message in messages(&fragment[..complete])? {
            match message.typ {
                // Read and discarded. This client never resumes, and a server
                // that offers a ticket is doing nothing wrong.
                HandshakeType::NewSessionTicket => {}
                HandshakeType::KeyUpdate => {
                    // RFC 8446 §4.6.3. The body is one octet:
                    // update_not_requested(0) or update_requested(1).
                    let requested = message.body == [0x01];
                    self.server_secret = update_traffic_secret(self.hash, &self.server_secret);
                    let keys = traffic_keys(self.hash, &self.server_secret, self.aead.key_len());
                    self.opener = Opener::new(self.aead, &keys.key, &keys.iv)?;

                    if requested {
                        // Answer before rekeying our own direction: the reply
                        // goes out under the key the peer still has.
                        let body = Message::encode(HandshakeType::KeyUpdate, &[0x00]);
                        reply.extend_from_slice(&self.sealer.seal(
                            ContentType::Handshake,
                            &body,
                            0,
                        )?);
                        self.client_secret = update_traffic_secret(self.hash, &self.client_secret);
                        let keys =
                            traffic_keys(self.hash, &self.client_secret, self.aead.key_len());
                        self.sealer = Sealer::new(self.aead, &keys.key, &keys.iv)?;
                    }
                }
                other => {
                    return Err(ClientError::UnexpectedMessage {
                        expected: "a post-handshake message",
                        got: other,
                    })
                }
            }
        }

        if reply.is_empty() {
            Ok(Incoming::Handled)
        } else {
            Ok(Incoming::Reply(reply))
        }
    }
}

/// Says nothing about key material, for the reason [`super::kx`] gives.
impl core::fmt::Debug for Connection {
    fn fmt(&self, f: &mut core::fmt::Formatter<'_>) -> core::fmt::Result {
        f.debug_struct("Connection")
            .field("cipher_suite", &self.suite)
            .field("peer_certificates", &self.certificates.len())
            .field("keys", &"<redacted>")
            .finish()
    }
}

/// Says nothing about key material either.
impl core::fmt::Debug for ClientHandshake<'_> {
    fn fmt(&self, f: &mut core::fmt::Formatter<'_>) -> core::fmt::Result {
        let state = match &self.state {
            State::AwaitServerHello { retried, .. } => {
                if *retried {
                    "AwaitServerHello(after retry)"
                } else {
                    "AwaitServerHello"
                }
            }
            State::InFlight { expect, .. } => expect.name(),
            State::Done(_) => "Done",
            State::Failed => "Failed",
        };
        f.debug_struct("ClientHandshake")
            .field("state", &state)
            .field("buffered", &self.buffer.len())
            .finish()
    }
}
