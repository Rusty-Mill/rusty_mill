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
    ClientHello, Extension, HandshakeError, HandshakeType, Message, ServerHello, Transcript,
    HELLO_RETRY_REQUEST_RANDOM, SERVER_CERTIFICATE_VERIFY_CONTEXT,
};
use super::kx::{KeyExchange, KxError, NamedGroup};
use super::record::{
    Aead, ContentType, Opener, RecordError, Sealer, HEADER_LEN, MAX_ENCRYPTED_FRAGMENT_LEN,
};
use super::schedule::{finished_verify_data, traffic_keys, verify_finished, Hash, KeySchedule};
use super::sign::{SignError, SigningKey};
use super::verify::SignatureScheme;
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
            Self::RetriedHelloStillHasNoShare(group) => write!(
                f,
                "the retried ClientHello still carried no share for {group:?}"
            ),
            Self::RetriedHelloChangedIdentity => {
                f.write_str("the retried ClientHello was not the same hello")
            }
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
            | Self::RetriedHelloStillHasNoShare(_)
            | Self::Kx(_)
            | Self::NoSharedSignatureScheme
            | Self::Sign(_)
            | Self::Random => AlertDescription::HANDSHAKE_FAILURE,
            // §4.1.2 calls a hello that changed where it may not an illegal
            // parameter, which is more specific than "handshake failure" and
            // tells the client which of the two hellos to look at.
            Self::RetriedHelloChangedIdentity => AlertDescription::ILLEGAL_PARAMETER,
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
        } = selected;

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
        transcript.add(transcript_head);
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
