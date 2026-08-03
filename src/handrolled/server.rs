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
//! - **Client certificates.** No CertificateRequest is ever sent, so no client
//!   is ever authenticated. A caller wanting mutual TLS does not have it.
//! - **Session resumption, tickets, and 0-RTT.** No NewSessionTicket is
//!   issued. Resumption is where a server's most interesting state lives, and
//!   a server that stores nothing cannot be confused about what it stored.
//! - **TLS 1.2 and below.** A ClientHello that does not offer `0x0304` in
//!   `supported_versions` gets a `protocol_version` alert, which is what stage
//!   4a taught this code to send and to read.
//! - **HelloRetryRequest generation.** If the client's `key_share` names no
//!   group this server supports, the handshake ends with a
//!   `handshake_failure` alert rather than a retry. A retry is a legitimate
//!   answer and would be strictly better; it is not implemented, and saying so
//!   is better than a client discovering it.

use super::handshake::{
    certificate_verify_content, complete_prefix, extension, find, messages, parse_finished,
    ClientHello, Extension, HandshakeError, HandshakeType, Message, ServerHello, Transcript,
    SERVER_CERTIFICATE_VERIFY_CONTEXT,
};
use super::kx::{KeyExchange, KxError, NamedGroup};
use super::record::{
    Aead, ContentType, Opener, RecordError, Sealer, HEADER_LEN, MAX_ENCRYPTED_FRAGMENT_LEN,
};
use super::schedule::{finished_verify_data, traffic_keys, verify_finished, Hash, KeySchedule};
use super::sign::{SignError, SigningKey};
use super::wire::{Reader, Writer};

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
    /// The client's `key_share` named no group this server implements.
    ///
    /// A HelloRetryRequest would be the better answer and is not implemented —
    /// see the module docs.
    NoSharedGroup,
    /// The client offered no signature scheme this server's key can produce.
    NoSharedSignatureScheme,
    /// The client's Finished did not verify.
    BadFinished,
    /// The peer sent an alert.
    PeerAlert(Alert),
    /// A record arrived after the connection was already broken.
    Failed,
    /// The system random source failed.
    Random,
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
            Self::NoSharedSignatureScheme => f.write_str("no signature scheme in common"),
            Self::BadFinished => f.write_str("the client's Finished did not verify"),
            Self::PeerAlert(alert) => {
                write!(
                    f,
                    "the peer sent a {:?} alert: {}",
                    alert.level, alert.description
                )
            }
            Self::Failed => f.write_str("the connection already failed"),
            Self::Random => f.write_str("the system random source failed"),
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
            Self::BadFinished => AlertDescription::DECRYPT_ERROR,
            Self::Record(_) => AlertDescription::BAD_RECORD_MAC,
            Self::NoSharedCipherSuite
            | Self::NoSharedGroup
            | Self::NoSharedSignatureScheme
            | Self::Kx(_)
            | Self::Sign(_)
            | Self::Random => AlertDescription::HANDSHAKE_FAILURE,
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
}

enum State {
    AwaitClientHello,
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
            State::AwaitClientHello => {
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
            State::AwaitFinished(_) => self.finished(message),
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

impl ServerHandshake<'_> {
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
        // share. A group in `supported_groups` with no share would need a
        // HelloRetryRequest, which is not implemented — see the module docs.
        let shares =
            find(&hello.extensions, extension::KEY_SHARE).ok_or(ServerError::NoSharedGroup)?;
        let shares = client_key_shares(shares)?;
        let (group, peer_key) = self
            .config
            .groups
            .iter()
            .find_map(|group| {
                shares
                    .iter()
                    .find(|(offered, _)| *offered == group.as_u16())
                    .map(|(_, key)| (*group, *key))
            })
            .ok_or(ServerError::NoSharedGroup)?;

        let kx = KeyExchange::generate(group)?;
        let mut share = Writer::new();
        share.u16(group.as_u16());
        share.vector_u16(|w| w.bytes(kx.public_key()));
        let share = share.into_vec();
        let versions = vec![0x03, 0x04];

        let random = random_bytes(32)?;
        let server_hello = ServerHello {
            random: &random,
            // Echoed verbatim: RFC 8446 §4.1.3 requires it, and middleboxes
            // in compatibility mode are watching for it.
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
        let server_hello = Message::encode(HandshakeType::ServerHello, &server_hello.encode());

        let mut transcript = Transcript::new(hash);
        transcript.add(message.encoded);
        transcript.add(&server_hello);
        let hello_hash = transcript.hash();

        let schedule = kx.agree(peer_key, |secret| {
            KeySchedule::new(hash).into_handshake(secret)
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

        let certificate = certificate_message(self.config.certificates);
        transcript.add(&certificate);
        flight.extend_from_slice(&certificate);

        // Signed over the transcript through the Certificate, with the §4.4.3
        // padding and context string. Never over a bare hash.
        let content =
            certificate_verify_content(SERVER_CERTIFICATE_VERIFY_CONTEXT, &transcript.hash());
        let signature = self.config.key.sign(scheme, &content)?;
        let mut verify = Writer::new();
        verify.u16(scheme.0);
        verify.vector_u16(|w| w.bytes(&signature));
        let verify = Message::encode(HandshakeType::CertificateVerify, &verify.into_vec());
        transcript.add(&verify);
        flight.extend_from_slice(&verify);

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
        out.extend_from_slice(&plaintext_record(ContentType::ChangeCipherSpec, &[0x01]));
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
        }));
        Ok(out)
    }

    fn finished(&mut self, message: &Message<'_>) -> Result<Vec<u8>> {
        if message.typ != HandshakeType::Finished {
            return Err(ServerError::UnexpectedMessage {
                expected: "Finished",
                got: message.typ,
            });
        }
        let State::AwaitFinished(negotiated) = core::mem::replace(&mut self.state, State::Failed)
        else {
            return Err(ServerError::Failed);
        };
        let negotiated = *negotiated;

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

        self.state = State::Done(Box::new(Connection::from_parts(
            negotiated.aead,
            negotiated.hash,
            negotiated.suite,
            Sealer::new(negotiated.aead, &server_keys.key, &server_keys.iv)?,
            Opener::new(negotiated.aead, &client_keys.key, &client_keys.iv)?,
            negotiated.server_application_secret,
            negotiated.client_application_secret,
            Vec::new(),
        )));
        Ok(Vec::new())
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
