//! The TLS 1.2 client handshake — stage 4b-iii.
//!
//! A sans-IO ECDHE handshake over the pieces the previous stages built: the
//! record layer ([`super::record12`]), the key derivation
//! ([`super::schedule12`]), the messages ([`super::handshake12`]), and, shared
//! with the TLS 1.3 client, certificate path validation, name matching, key
//! exchange and signature verification.
//!
//! ```text
//! Client                                           Server
//!   ClientHello  ------------------------------->
//!                                                 ServerHello
//!                                                 Certificate
//!                                                 ServerKeyExchange   (signed)
//!                                                 [CertificateRequest]
//!                <-------------------------------  ServerHelloDone
//!   [Certificate]  (empty)
//!   ClientKeyExchange
//!   ChangeCipherSpec
//!   Finished     ------------------------------->
//!                                                 ChangeCipherSpec
//!                <-------------------------------  Finished
//! ```
//!
//! # What this refuses, and why it is a refusal
//!
//! A TLS 1.2 client is the first part of this engine that must tolerate peers
//! older than the protocol it would like to speak, so the temptation is to
//! accept whatever a server offers. Every item here is something a conforming
//! modern server does, and a server that does not is refused rather than
//! accommodated:
//!
//! - **No extended master secret** ([`ClientError::MissingExtendedMasterSecret`]).
//!   Without RFC 7627 two handshakes can share a master secret, which is the
//!   triple handshake attack. [`super::schedule12`] has no function for the
//!   original derivation, so this is a refusal and not a downgrade.
//! - **No secure renegotiation** ([`ClientError::BadRenegotiationInfo`]).
//!   RFC 5746. A server that does not echo it cannot be told from an attacker
//!   splicing a prefix into a victim's session.
//! - **Anything but ECDHE with an AEAD.** CBC, RC4, 3DES, static RSA and
//!   finite-field DHE are never offered, so a server cannot select them.
//! - **Session resumption of either kind.** No session id and no ticket is
//!   offered; resumption is a later stage.
//! - **Client certificates, only if configured.** With no
//!   [`ClientConfig12::identity`], or none whose key can sign a scheme the server
//!   named, a `CertificateRequest` is answered with an empty `Certificate`, the
//!   conforming way to say "none"; a server that insists fails the handshake.
//!   With one, the chain is sent and a `CertificateVerify` proves the key.
//! - **Unsolicited extensions** ([`ClientError::UnofferedExtension`]), as RFC
//!   5246 §7.4.1.4 requires.
//!
//! # A `ChangeCipherSpec` is accepted at exactly one moment
//!
//! After this client has sent its `Finished`, and only as the single octet
//! `0x01`, with no handshake bytes half-read. Accepting one earlier is
//! CVE-2014-0224: an attacker injects it and the peers start encrypting under
//! keys that were never agreed. TLS 1.3 made the record meaningless; in TLS 1.2
//! it is a state transition, so it is treated as one.
//!
//! # Downgrade protection lives one level up
//!
//! Standing alone, this client offers TLS 1.2 and nothing else, so there is
//! nothing to downgrade *from* and the `DOWNGRD` sentinel in
//! `ServerHello.random` is deliberately not checked: a 1.2-only client of a
//! 1.3-capable server is told the truth, not attacked. [`super::negotiate`]
//! builds the client that offers both, continues this handshake from its hello
//! when the server answers in 1.2, and refuses the sentinel there.

use super::client::{
    is_downgrade_sentinel, plaintext_record, random_bytes, Alert, AlertDescription, AlertLevel,
    ClientError, ClientIdentity,
};
use super::handshake::{
    complete_prefix, encode_alpn_offer, extension, find, messages, parse_alpn_selection,
    CertificateVerify, ClientHello, Extension, HandshakeError, HandshakeType, Message,
};
use super::handshake12::{
    self, message, parse_finished, parse_server_hello_done, Certificate12, CertificateRequest12,
    ServerHello12, ServerKeyExchange, TLS12,
};
use super::kx::{KeyExchange, NamedGroup};
use super::limits::Noise;
use super::name::ServerName;
use super::path::{verify_peer_certificate, PathOptions, TrustAnchor};
use super::record::{Aead, ContentType, RecordError, MAX_FRAGMENT_LEN};
use super::record12::{split, Opener, Sealer};
use super::schedule::Hash;
use super::schedule12::{
    extended_master_secret, finished_verify_data, key_block, verify_finished, MasterSecret, Side,
    RANDOM_LEN,
};
use super::verify::{verify_tls12_signature, SignatureScheme};
use super::wire::Writer;
use super::x509::{oid, Certificate};

type Result<T> = core::result::Result<T, ClientError>;

/// The largest handshake message this client will buffer while reassembling.
///
/// A `Certificate` is the big one. Real chains are a few kilobytes; the cap
/// stops a server from making the client buffer up to 16 MiB, which the length
/// field would otherwise allow, before it has authenticated anything.
pub const MAX_HANDSHAKE_BUFFER: usize = 1 << 17;

/// The record version used for every record this client sends. RFC 5246
/// appendix E allows `{03,XX}` on the first; 1.2 throughout is the honest one.
const RECORD_VERSION: u16 = 0x0303;
/// `no_renegotiation(100)`, the warning that refuses a `HelloRequest`.
const NO_RENEGOTIATION: AlertDescription = AlertDescription(100);
/// `id-Ed25519`, compared by encoding since `x509::oid` does not name it.
const ED25519_OID: &[u8] = &[0x2b, 0x65, 0x70];

/// How a suite's server proves its identity.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Authentication {
    /// `ECDHE_ECDSA`: an EC or Ed25519 certificate.
    Ecdsa,
    /// `ECDHE_RSA`: an RSA certificate.
    Rsa,
}

/// A TLS 1.2 cipher suite this client can negotiate.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct CipherSuite12(pub u16);

impl CipherSuite12 {
    /// `TLS_ECDHE_ECDSA_WITH_AES_128_GCM_SHA256`.
    pub const ECDHE_ECDSA_WITH_AES_128_GCM_SHA256: Self = Self(0xc02b);
    /// `TLS_ECDHE_ECDSA_WITH_AES_256_GCM_SHA384`.
    pub const ECDHE_ECDSA_WITH_AES_256_GCM_SHA384: Self = Self(0xc02c);
    /// `TLS_ECDHE_RSA_WITH_AES_128_GCM_SHA256`.
    pub const ECDHE_RSA_WITH_AES_128_GCM_SHA256: Self = Self(0xc02f);
    /// `TLS_ECDHE_RSA_WITH_AES_256_GCM_SHA384`.
    pub const ECDHE_RSA_WITH_AES_256_GCM_SHA384: Self = Self(0xc030);
    /// `TLS_ECDHE_ECDSA_WITH_CHACHA20_POLY1305_SHA256` (RFC 7905).
    pub const ECDHE_ECDSA_WITH_CHACHA20_POLY1305_SHA256: Self = Self(0xcca9);
    /// `TLS_ECDHE_RSA_WITH_CHACHA20_POLY1305_SHA256` (RFC 7905).
    pub const ECDHE_RSA_WITH_CHACHA20_POLY1305_SHA256: Self = Self(0xcca8);

    /// Every suite this client implements, most preferred first.
    pub const SUPPORTED: &'static [Self] = &[
        Self::ECDHE_ECDSA_WITH_AES_128_GCM_SHA256,
        Self::ECDHE_RSA_WITH_AES_128_GCM_SHA256,
        Self::ECDHE_ECDSA_WITH_AES_256_GCM_SHA384,
        Self::ECDHE_RSA_WITH_AES_256_GCM_SHA384,
        Self::ECDHE_ECDSA_WITH_CHACHA20_POLY1305_SHA256,
        Self::ECDHE_RSA_WITH_CHACHA20_POLY1305_SHA256,
    ];

    /// The AEAD, the PRF hash and the authentication this suite names, or
    /// `None` for a value that is not one of [`Self::SUPPORTED`].
    ///
    /// The hash belongs to the suite, not the AEAD: it is SHA-384 for the
    /// `*_SHA384` suites and SHA-256 for the rest (RFC 5289).
    pub const fn parts(self) -> Option<(Aead, Hash, Authentication)> {
        match self.0 {
            0xc02b => Some((Aead::Aes128Gcm, Hash::Sha256, Authentication::Ecdsa)),
            0xc02c => Some((Aead::Aes256Gcm, Hash::Sha384, Authentication::Ecdsa)),
            0xc02f => Some((Aead::Aes128Gcm, Hash::Sha256, Authentication::Rsa)),
            0xc030 => Some((Aead::Aes256Gcm, Hash::Sha384, Authentication::Rsa)),
            0xcca9 => Some((Aead::ChaCha20Poly1305, Hash::Sha256, Authentication::Ecdsa)),
            0xcca8 => Some((Aead::ChaCha20Poly1305, Hash::Sha256, Authentication::Rsa)),
            _ => None,
        }
    }
}

/// What a TLS 1.2 client needs to know before it can start. Every field is
/// required, for the reason [`super::client::ClientConfig`]'s are.
pub struct ClientConfig12<'a> {
    /// The server being connected to, matched against the certificate.
    pub server_name: ServerName<'a>,
    /// The trust anchors the chain must reach.
    pub anchors: &'a [TrustAnchor<'a>],
    /// Path validation options, including the current time.
    pub path: PathOptions,
    /// The curves to offer, most preferred first. A `ServerKeyExchange` naming
    /// any other is refused.
    pub groups: &'a [NamedGroup],
    /// The suites to offer, most preferred first.
    pub cipher_suites: &'a [CipherSuite12],
    /// The certificate chain and key to present if the server asks for one.
    ///
    /// `None` answers every request with an empty `Certificate`. A key that can
    /// sign none of the schemes the server listed is treated the same way,
    /// which RFC 5246 section 7.4.6 allows and leaves the decision with the
    /// server.
    pub identity: Option<&'a ClientIdentity<'a>>,
    /// Application protocols to offer (RFC 7301), most preferred first; empty
    /// offers none. The server's choice is [`Connection12::alpn_protocol`].
    pub alpn: &'a [&'a [u8]],
}

/// The message the state machine will accept next, and nothing else.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum Expect {
    ServerHello,
    Certificate,
    ServerKeyExchange,
    /// Either a `CertificateRequest` or the `ServerHelloDone`.
    CertificateRequestOrDone,
    ServerHelloDone,
    /// Our `Finished` is sent; the server's `ChangeCipherSpec` record is next.
    ChangeCipherSpec,
    /// The server's encrypted `Finished`.
    Finished,
}

impl Expect {
    const fn name(self) -> &'static str {
        match self {
            Self::ServerHello => "ServerHello",
            Self::Certificate => "Certificate",
            Self::ServerKeyExchange => "ServerKeyExchange",
            Self::CertificateRequestOrDone => "CertificateRequest or ServerHelloDone",
            Self::ServerHelloDone => "ServerHelloDone",
            Self::ChangeCipherSpec => "ChangeCipherSpec",
            Self::Finished => "Finished",
        }
    }
}

enum Phase {
    Expecting(Expect),
    Done,
    Failed,
}

/// What the handshake has learned so far.
struct Hs {
    client_random: [u8; RANDOM_LEN],
    server_random: [u8; RANDOM_LEN],
    /// Every handshake message so far, encoded, in order. `Finished` is a MAC
    /// over these bytes and so is the session hash, so what is stored is what
    /// crossed the wire, never a re-encoding.
    transcript: Vec<u8>,
    suite: CipherSuite12,
    aead: Aead,
    hash: Hash,
    auth: Authentication,
    certificates: Vec<Vec<u8>>,
    /// The curve and public key from `ServerKeyExchange`.
    server_key: Option<(NamedGroup, Vec<u8>)>,
    /// The schemes a `CertificateRequest` named, if the server sent one.
    certificate_request: Option<Vec<u16>>,
    /// The application protocol the server selected, if any.
    alpn: Option<Vec<u8>>,
    /// Set once the client flight is sent.
    established: Option<Established>,
}

/// Keys derived after `ServerHelloDone`.
struct Established {
    master: MasterSecret,
    sealer: Sealer,
    /// Not used until the server's `ChangeCipherSpec` arrives.
    opener: Opener,
}

/// A TLS 1.2 client handshake in progress.
pub struct ClientHandshake12<'a> {
    config: &'a ClientConfig12<'a>,
    phase: Phase,
    hs: Hs,
    /// Handshake bytes reassembled across records.
    buffer: Vec<u8>,
    connection: Option<Connection12>,
    /// True when the ClientHello also offered TLS 1.3, so a ServerHello that
    /// answers in 1.2 must not carry the `DOWNGRD` sentinel.
    offered_tls13: bool,
    /// The session id the hello carried. Empty unless this handshake continues
    /// a combined hello, whose 1.3 compatibility id is not a session.
    offered_session_id: Vec<u8>,
    /// Records that carried nothing, so they cannot go on for ever.
    noise: Noise,
}

fn encode_client_hello(config: &ClientConfig12<'_>, random: &[u8]) -> Result<Vec<u8>> {
    if config.cipher_suites.is_empty() {
        return Err(HandshakeError::Empty("cipher_suites").into());
    }
    if config.groups.is_empty() {
        return Err(HandshakeError::Empty("groups").into());
    }
    for suite in config.cipher_suites {
        if suite.parts().is_none() {
            return Err(ClientError::UnofferedCipherSuite(suite.0));
        }
    }

    let mut names = Writer::new();
    if let ServerName::Dns(name) = &config.server_name {
        names.vector_u16(|w| {
            w.u8(0); // host_name
            w.vector_u16(|w| w.bytes(name.as_bytes()));
        });
    }
    let mut groups = Writer::new();
    groups.vector_u16(|w| config.groups.iter().for_each(|g| w.u16(g.as_u16())));
    let mut schemes = Writer::new();
    schemes.vector_u16(|w| {
        SignatureScheme::TLS12_SUPPORTED
            .iter()
            .for_each(|s| w.u16(s.0))
    });

    let alpn = (!config.alpn.is_empty()).then(|| encode_alpn_offer(config.alpn));
    let mut extensions = Vec::new();
    if !names.is_empty() {
        extensions.push(Extension {
            typ: extension::SERVER_NAME,
            data: names.as_slice(),
        });
    }
    if let Some(alpn) = &alpn {
        extensions.push(Extension {
            typ: extension::ALPN,
            data: alpn.as_slice(),
        });
    }
    // RFC 8422: uncompressed only.
    extensions.push(Extension {
        typ: extension::EC_POINT_FORMATS,
        data: &[1, 0],
    });
    extensions.push(Extension {
        typ: extension::SUPPORTED_GROUPS,
        data: groups.as_slice(),
    });
    extensions.push(Extension {
        typ: extension::SIGNATURE_ALGORITHMS,
        data: schemes.as_slice(),
    });
    extensions.push(Extension {
        typ: extension::EXTENDED_MASTER_SECRET,
        data: &[],
    });
    // RFC 5746: an empty renegotiated_connection on an initial handshake.
    extensions.push(Extension {
        typ: extension::RENEGOTIATION_INFO,
        data: &[0],
    });

    let hello = ClientHello {
        random,
        session_id: &[],
        cipher_suites: config.cipher_suites.iter().map(|s| s.0).collect(),
        extensions,
    };
    Ok(message(HandshakeType::ClientHello, &hello.encode()))
}

impl<'a> ClientHandshake12<'a> {
    /// Start a handshake, returning it and the ClientHello record to send.
    pub fn start(config: &'a ClientConfig12<'a>) -> Result<(Self, Vec<u8>)> {
        let random = random_bytes(RANDOM_LEN)?;
        let hello = encode_client_hello(config, &random)?;
        let record = plaintext_record(ContentType::Handshake, RECORD_VERSION, &hello)?;

        let mut client_random = [0u8; RANDOM_LEN];
        client_random.copy_from_slice(&random);
        let first = config.cipher_suites[0];
        let (aead, hash, auth) = first.parts().ok_or(ClientError::Failed)?;
        Ok((
            Self {
                config,
                phase: Phase::Expecting(Expect::ServerHello),
                hs: Hs {
                    client_random,
                    server_random: [0; RANDOM_LEN],
                    transcript: hello,
                    // Placeholders until the ServerHello selects a suite.
                    suite: first,
                    aead,
                    hash,
                    auth,
                    certificates: Vec::new(),
                    server_key: None,
                    certificate_request: None,
                    alpn: None,
                    established: None,
                },
                buffer: Vec::new(),
                connection: None,
                offered_tls13: false,
                offered_session_id: Vec::new(),
                noise: Noise::default(),
            },
            record,
        ))
    }

    /// Continue from a ClientHello that was already sent and also offered
    /// TLS 1.3, and that the server answered in 1.2. `hello` is the message as
    /// sent, which is the start of the transcript, and `random` its random.
    ///
    /// The caller must have built the hello from this configuration's suites
    /// and groups; the handshake judges the server's choices against them.
    pub(super) fn continue_from(
        config: &'a ClientConfig12<'a>,
        hello: Vec<u8>,
        random: &[u8],
        session_id: Vec<u8>,
    ) -> Result<Self> {
        let (mut handshake, _) = Self::start(config)?;
        let mut client_random = [0u8; RANDOM_LEN];
        client_random.copy_from_slice(random);
        handshake.hs.client_random = client_random;
        handshake.hs.transcript = hello;
        handshake.offered_tls13 = true;
        handshake.offered_session_id = session_id;
        Ok(handshake)
    }

    /// The fatal alert record to send for `error`, if the server should be
    /// told.
    ///
    /// In the clear until this client's own `ChangeCipherSpec` has been sent,
    /// and protected after: from then on the server expects nothing else.
    pub fn alert_record(&mut self, error: &ClientError) -> Option<Vec<u8>> {
        let description = error.alert()?;
        if let Some(established) = self.hs.established.as_mut() {
            return established
                .sealer
                .seal(ContentType::Alert, &[2, description.0])
                .ok();
        }
        plaintext_record(ContentType::Alert, RECORD_VERSION, &[2, description.0]).ok()
    }

    /// True once the handshake has completed and [`Self::into_connection`]
    /// will succeed.
    pub fn is_finished(&self) -> bool {
        matches!(self.phase, Phase::Done)
    }

    /// Take the established connection.
    pub fn into_connection(self) -> Result<Connection12> {
        match (self.phase, self.connection) {
            (Phase::Done, Some(connection)) => Ok(connection),
            _ => Err(ClientError::Failed),
        }
    }

    /// Feed one whole TLS record, and get back the bytes to send in reply.
    ///
    /// `record` must be exactly one record, header included; use
    /// [`super::client::record_length`] to find where it ends. The reply is empty except after
    /// the server's `ServerHelloDone`, which is answered with the client's
    /// whole flight.
    ///
    /// A failure is permanent: every later call returns
    /// [`ClientError::Failed`].
    pub fn read_record(&mut self, record: &[u8]) -> Result<Vec<u8>> {
        if !matches!(self.phase, Phase::Expecting(_)) {
            return Err(ClientError::Failed);
        }
        match self.read_record_inner(record) {
            Ok(reply) => Ok(reply),
            Err(err) => {
                self.phase = Phase::Failed;
                Err(err)
            }
        }
    }

    fn expecting(&self) -> Result<Expect> {
        match self.phase {
            Phase::Expecting(expect) => Ok(expect),
            _ => Err(ClientError::Failed),
        }
    }

    fn read_record_inner(&mut self, record: &[u8]) -> Result<Vec<u8>> {
        let (typ, version, fragment) = split(record)?;
        let expect = self.expecting()?;

        // Only the server's Finished is protected. Everything before it,
        // including the server's ChangeCipherSpec, is in the clear.
        if expect == Expect::Finished {
            return self.read_protected(record);
        }

        // The record version of a plaintext record is a hint, not a check:
        // servers differ on whether the first records say 3.1 or 3.3. A major
        // version other than 3 is not TLS.
        if version[0] != 3 {
            return Err(RecordError::UnexpectedVersion(version).into());
        }
        match typ {
            ContentType::Alert => self.plaintext_alert(fragment),
            ContentType::ChangeCipherSpec => self.change_cipher_spec(expect, fragment),
            ContentType::Handshake if expect != Expect::ChangeCipherSpec => {
                self.append_and_drain(fragment)
            }
            other => Err(ClientError::UnexpectedContentType(other)),
        }
    }

    /// An alert in the clear. Fatal ones, and `close_notify` (which cannot be
    /// orderly before the handshake is done), end the handshake; other warnings
    /// are advisory and ignored, as RFC 5246 §7.2 allows.
    fn plaintext_alert(&mut self, fragment: &[u8]) -> Result<Vec<u8>> {
        match Alert::parse(fragment) {
            Some(alert) if alert.is_advisory() => {
                self.noise.warning()?;
                Ok(Vec::new())
            }
            Some(alert) => Err(ClientError::PeerAlert(alert)),
            None => Err(ClientError::UnexpectedContentType(ContentType::Alert)),
        }
    }

    fn change_cipher_spec(&mut self, expect: Expect, fragment: &[u8]) -> Result<Vec<u8>> {
        // Exactly one moment, exactly one octet, and nothing half-read. See the
        // module docs.
        if expect != Expect::ChangeCipherSpec || fragment != [1] || !self.buffer.is_empty() {
            return Err(ClientError::UnexpectedChangeCipherSpec);
        }
        self.phase = Phase::Expecting(Expect::Finished);
        Ok(Vec::new())
    }

    fn read_protected(&mut self, record: &[u8]) -> Result<Vec<u8>> {
        let established = self.hs.established.as_mut().ok_or(ClientError::Failed)?;
        let opened = established.opener.open(record)?;
        match opened.typ {
            ContentType::Handshake => self.append_and_drain(&opened.fragment),
            ContentType::Alert => Err(match Alert::parse(&opened.fragment) {
                Some(alert) => ClientError::PeerAlert(alert),
                None => ClientError::UnexpectedContentType(ContentType::Alert),
            }),
            other => Err(ClientError::UnexpectedContentType(other)),
        }
    }

    fn append_and_drain(&mut self, fragment: &[u8]) -> Result<Vec<u8>> {
        if self.buffer.len() + fragment.len() > MAX_HANDSHAKE_BUFFER {
            return Err(ClientError::HandshakeTooLarge);
        }
        if fragment.is_empty() {
            self.noise.empty_record()?;
        } else {
            self.noise.data();
        }
        self.buffer.extend_from_slice(fragment);

        let mut reply = Vec::new();
        loop {
            let complete = complete_prefix(&self.buffer);
            if complete == 0 {
                return Ok(reply);
            }
            let consumed: Vec<u8> = self.buffer.drain(..complete).collect();
            let parsed = messages(&consumed)?;
            let last = parsed.len().saturating_sub(1);
            for (index, msg) in parsed.iter().enumerate() {
                // The server's flight ends at ServerHelloDone and nothing may
                // follow it: this client answers it with its own flight, and
                // bytes of a message left in the buffer would be glued to
                // whatever comes next (BoGo `Partial*WithServerHelloDone`).
                // Checked before the flight is built, so the refusal is an
                // alert in the clear: nothing protected has been sent.
                if msg.typ == HandshakeType::ServerHelloDone
                    && (index != last || !self.buffer.is_empty())
                {
                    return Err(ClientError::UnexpectedMessage {
                        expected: "nothing after ServerHelloDone",
                        got: msg.typ,
                    });
                }
                reply.extend_from_slice(&self.handle_message(msg)?);
            }
        }
    }

    fn handle_message(&mut self, msg: &Message<'_>) -> Result<Vec<u8>> {
        // A message after the server's Finished, in the same record, is a
        // protocol violation worth naming; it must not be handled, and it must
        // not surface as a bare `Failed`.
        if matches!(self.phase, Phase::Done) {
            return Err(ClientError::UnexpectedMessage {
                expected: "nothing after Finished",
                got: msg.typ,
            });
        }
        let expect = self.expecting()?;
        let unexpected = || ClientError::UnexpectedMessage {
            expected: expect.name(),
            got: msg.typ,
        };

        match (expect, msg.typ) {
            (Expect::ServerHello, HandshakeType::ServerHello) => {
                self.hs.transcript.extend_from_slice(msg.encoded);
                self.server_hello(msg.body)?;
                Ok(Vec::new())
            }
            (Expect::Certificate, HandshakeType::Certificate) => {
                self.hs.transcript.extend_from_slice(msg.encoded);
                self.certificate(msg.body)?;
                Ok(Vec::new())
            }
            (Expect::ServerKeyExchange, HandshakeType::ServerKeyExchange) => {
                self.hs.transcript.extend_from_slice(msg.encoded);
                self.server_key_exchange(msg.body)?;
                Ok(Vec::new())
            }
            (Expect::CertificateRequestOrDone, HandshakeType::CertificateRequest) => {
                self.hs.transcript.extend_from_slice(msg.encoded);
                let request = CertificateRequest12::parse(msg.body)?;
                self.hs.certificate_request = Some(request.signature_algorithms);
                self.phase = Phase::Expecting(Expect::ServerHelloDone);
                Ok(Vec::new())
            }
            (
                Expect::CertificateRequestOrDone | Expect::ServerHelloDone,
                HandshakeType::ServerHelloDone,
            ) => {
                self.hs.transcript.extend_from_slice(msg.encoded);
                parse_server_hello_done(msg.body)?;
                self.client_flight()
            }
            (Expect::Finished, HandshakeType::Finished) => {
                self.server_finished(msg.body)?;
                Ok(Vec::new())
            }
            _ => Err(unexpected()),
        }
    }

    fn server_hello(&mut self, body: &[u8]) -> Result<()> {
        let hello = ServerHello12::parse(body)?;
        if hello.version != TLS12 {
            return Err(ClientError::NotTls12(hello.version));
        }
        // RFC 8446 §4.1.3: a client that offered 1.3 and is answered in 1.2 by
        // a server that *also* speaks 1.3 is being told so in `random`. The
        // only way that happens is a hello that was altered on the way.
        if self.offered_tls13 && is_downgrade_sentinel(hello.random) {
            return Err(ClientError::DowngradeDetected);
        }
        // A ServerHello that echoes the session id the hello carried claims to
        // resume a session. The id in a combined hello is TLS 1.3's
        // middlebox-compatibility filler (RFC 8446 §D.4), not a session, so an
        // echo is a server that is confused about what it is resuming.
        if !self.offered_session_id.is_empty() && hello.session_id == self.offered_session_id {
            return Err(ClientError::SessionIdMismatch);
        }
        let suite = CipherSuite12(hello.cipher_suite);
        if !self.config.cipher_suites.contains(&suite) {
            return Err(ClientError::UnofferedCipherSuite(hello.cipher_suite));
        }
        let (aead, hash, auth) = suite
            .parts()
            .ok_or(ClientError::UnofferedCipherSuite(hello.cipher_suite))?;
        if hello.compression != 0 {
            return Err(HandshakeError::UnexpectedCompression.into());
        }

        // RFC 5246 §7.4.1.4: a server may only answer extensions it was offered.
        // The set is what the ClientHello carried, minus the ones a server
        // never echoes with data (supported_groups, signature_algorithms).
        for ext in &hello.extensions {
            let allowed = match ext.typ {
                // Only as an answer to a name that was sent.
                extension::SERVER_NAME => matches!(self.config.server_name, ServerName::Dns(_)),
                // RFC 8422 lets a server volunteer its groups, and BoringSSL's
                // suite checks that a client carries on.
                extension::SUPPORTED_GROUPS => true,
                extension::EC_POINT_FORMATS
                | extension::EXTENDED_MASTER_SECRET
                | extension::RENEGOTIATION_INFO => true,
                // Only as an answer to a list that was sent.
                extension::ALPN => !self.config.alpn.is_empty(),
                _ => false,
            };
            if !allowed {
                return Err(ClientError::UnofferedExtension(ext.typ));
            }
        }
        if let Some(data) = find(&hello.extensions, extension::ALPN) {
            let selected = parse_alpn_selection(data)?;
            if !self.config.alpn.contains(&selected) {
                return Err(ClientError::UnofferedAlpn);
            }
            self.hs.alpn = Some(selected.to_vec());
        }
        if find(&hello.extensions, extension::SERVER_NAME).is_some_and(|data| !data.is_empty()) {
            return Err(ClientError::Handshake(HandshakeError::Malformed(
                "a server_name acknowledgement is not empty",
            )));
        }
        match find(&hello.extensions, extension::EXTENDED_MASTER_SECRET) {
            Some([]) => {}
            _ => return Err(ClientError::MissingExtendedMasterSecret),
        }
        match find(&hello.extensions, extension::RENEGOTIATION_INFO) {
            Some([0]) => {}
            _ => return Err(ClientError::BadRenegotiationInfo),
        }
        if let Some(formats) = find(&hello.extensions, extension::EC_POINT_FORMATS) {
            // A one-octet length, then the formats; uncompressed (0) must be there.
            match formats.split_first() {
                Some((&len, list)) if usize::from(len) == list.len() && list.contains(&0) => {}
                _ => return Err(ClientError::UnsupportedPointFormat),
            }
        }

        self.hs.server_random.copy_from_slice(hello.random);
        self.hs.suite = suite;
        self.hs.aead = aead;
        self.hs.hash = hash;
        self.hs.auth = auth;
        self.phase = Phase::Expecting(Expect::Certificate);
        Ok(())
    }

    fn certificate(&mut self, body: &[u8]) -> Result<()> {
        let chain = Certificate12::parse(body)?;
        let (end_entity, rest) = chain
            .certificates
            .split_first()
            .ok_or(ClientError::NoCertificates)?;

        let leaf = Certificate::parse(end_entity).map_err(ClientError::MalformedCertificate)?;
        let intermediates = rest
            .iter()
            .map(|der| Certificate::parse(der))
            .collect::<core::result::Result<Vec<_>, _>>()
            .map_err(ClientError::MalformedCertificate)?;

        // The name, the chain and the time, in one call that cannot be
        // half-performed. A signature on the key exchange proves possession of
        // the key; only this proves the key is the server's.
        verify_peer_certificate(
            &leaf,
            &intermediates,
            self.config.anchors,
            &self.config.server_name,
            &self.config.path,
        )?;

        // The suite names how the server authenticates; the certificate has to
        // be the kind that can.
        let key = leaf.subject_public_key_info().algorithm.oid;
        let fits = match self.hs.auth {
            Authentication::Rsa => key == oid::RSA_ENCRYPTION,
            Authentication::Ecdsa => key == oid::EC_PUBLIC_KEY || key.as_bytes() == ED25519_OID,
        };
        if !fits {
            return Err(ClientError::KeyTypeMismatch);
        }

        self.hs.certificates = chain.certificates.iter().map(|der| der.to_vec()).collect();
        self.phase = Phase::Expecting(Expect::ServerKeyExchange);
        Ok(())
    }

    fn server_key_exchange(&mut self, body: &[u8]) -> Result<()> {
        let ske = ServerKeyExchange::parse(body)?;

        let group = NamedGroup::from_u16(ske.named_curve)
            .filter(|g| self.config.groups.contains(g))
            .ok_or(ClientError::UnofferedGroup(ske.named_curve))?;

        let scheme = SignatureScheme(ske.scheme);
        if !SignatureScheme::TLS12_SUPPORTED.contains(&scheme) {
            return Err(ClientError::UnofferedSignatureScheme(ske.scheme));
        }

        let leaf_der = self.hs.certificates.first().ok_or(ClientError::Failed)?;
        let leaf = Certificate::parse(leaf_der).map_err(ClientError::MalformedCertificate)?;
        let signed = ske.signed_content(&self.hs.client_random, &self.hs.server_random);
        verify_tls12_signature(
            scheme,
            &leaf.subject_public_key_info(),
            &signed,
            ske.signature,
        )?;

        self.hs.server_key = Some((group, ske.public.to_vec()));
        self.phase = Phase::Expecting(Expect::CertificateRequestOrDone);
        Ok(())
    }

    /// Everything the client sends after `ServerHelloDone`, in one buffer.
    fn client_flight(&mut self) -> Result<Vec<u8>> {
        let (group, server_public) = self.hs.server_key.take().ok_or(ClientError::Failed)?;

        // 1. Optional Certificate, then ClientKeyExchange. A key that can sign
        // none of the schemes named is as good as no key (RFC 5246 7.4.6).
        let identity = self.hs.certificate_request.as_ref().and_then(|schemes| {
            let identity = self.config.identity?;
            let scheme = *identity
                .key
                .schemes()
                .iter()
                .find(|scheme| schemes.contains(&scheme.0))?;
            Some((identity, scheme))
        });
        let mut handshake = Vec::new();
        if self.hs.certificate_request.is_some() {
            let chain: Vec<&[u8]> = identity
                .map(|(identity, _)| identity.certificates.iter().map(Vec::as_slice).collect())
                .unwrap_or_default();
            handshake.extend_from_slice(&message(
                HandshakeType::Certificate,
                &Certificate12::encode(&chain),
            ));
        }
        let kx = KeyExchange::generate(group)?;
        handshake.extend_from_slice(&message(
            HandshakeType::ClientKeyExchange,
            &handshake12::encode_client_key_exchange(kx.public_key()),
        ));
        self.hs.transcript.extend_from_slice(&handshake);

        // 2. Keys: the session hash covers everything up to and including
        // ClientKeyExchange, and not the CertificateVerify that follows (RFC 7627).
        let hash = self.hs.hash;
        let pre_master = kx.agree(&server_public, |shared| shared.to_vec())?;
        let session_hash = hash.hash(&self.hs.transcript);
        let master = extended_master_secret(hash, &pre_master, &session_hash)?;
        let keys = key_block(
            hash,
            self.hs.aead,
            &master,
            &self.hs.client_random,
            &self.hs.server_random,
        );
        let mut sealer = Sealer::new(self.hs.aead, &keys.client_write_key, &keys.client_write_iv)?;
        let opener = Opener::new(self.hs.aead, &keys.server_write_key, &keys.server_write_iv)?;

        // RFC 5246 7.4.8: the signature is over every handshake message so far,
        // as sent, so it is made here, after the Certificate and ClientKeyExchange
        // are in the transcript and before the Finished that covers it.
        if let Some((identity, scheme)) = identity {
            let signature = identity.key.sign(scheme, &self.hs.transcript)?;
            let verify = CertificateVerify {
                scheme: scheme.0,
                signature: &signature,
            };
            let verify = message(HandshakeType::CertificateVerify, &verify.encode());
            self.hs.transcript.extend_from_slice(&verify);
            handshake.extend_from_slice(&verify);
        }

        // 3. Finished, over the same transcript, protected with the new key.
        let verify_data =
            finished_verify_data(hash, &master, Side::Client, &hash.hash(&self.hs.transcript))?;
        let finished = message(HandshakeType::Finished, &verify_data);
        self.hs.transcript.extend_from_slice(&finished);
        let finished_record = sealer.seal(ContentType::Handshake, &finished)?;

        // 4. The flight: handshake record, ChangeCipherSpec, Finished.
        let mut flight = plaintext_record(ContentType::Handshake, RECORD_VERSION, &handshake)?;
        flight.extend(plaintext_record(
            ContentType::ChangeCipherSpec,
            RECORD_VERSION,
            &[1],
        )?);
        flight.extend(finished_record);

        self.hs.established = Some(Established {
            master,
            sealer,
            opener,
        });
        self.phase = Phase::Expecting(Expect::ChangeCipherSpec);
        Ok(flight)
    }

    fn server_finished(&mut self, body: &[u8]) -> Result<()> {
        // A Finished of the wrong length is a Finished that does not verify
        // (BoGo `TrailingMessageData-*Finished`): decrypt_error, not decode_error.
        let received = parse_finished(body).map_err(|_| ClientError::BadFinished)?;
        // Nothing may follow the Finished in its record, or be left half-read.
        if !self.buffer.is_empty() {
            return Err(ClientError::UnexpectedMessage {
                expected: "nothing after Finished",
                got: HandshakeType::Unknown(0),
            });
        }
        // Borrowed for the check and taken only after it passes: a failure here
        // is answered with an alert under the keys this client already sends
        // with, and they live in `established`.
        let established = self.hs.established.as_ref().ok_or(ClientError::Failed)?;
        let hash = self.hs.hash;
        let handshake_hash = hash.hash(&self.hs.transcript);
        if !verify_finished(
            hash,
            &established.master,
            Side::Server,
            &handshake_hash,
            received,
        ) {
            return Err(ClientError::BadFinished);
        }
        let established = self.hs.established.take().ok_or(ClientError::Failed)?;

        self.connection = Some(
            Connection12::new(
                Role::Client,
                established.sealer,
                established.opener,
                self.hs.suite,
                core::mem::take(&mut self.hs.certificates),
            )
            .with_alpn(self.hs.alpn.take()),
        );
        self.phase = Phase::Done;
        Ok(())
    }
}

impl core::fmt::Debug for ClientHandshake12<'_> {
    fn fmt(&self, f: &mut core::fmt::Formatter<'_>) -> core::fmt::Result {
        let state = match &self.phase {
            Phase::Expecting(expect) => expect.name(),
            Phase::Done => "done",
            Phase::Failed => "failed",
        };
        f.debug_struct("ClientHandshake12")
            .field("state", &state)
            .finish_non_exhaustive()
    }
}

// ---------------------------------------------------------------------------
// The established connection
// ---------------------------------------------------------------------------

/// What [`Connection12::read`] found in a record.
#[derive(Clone, Debug, PartialEq, Eq)]
#[non_exhaustive]
pub enum Incoming12 {
    /// Application data (possibly empty: RFC 5246 allows zero-length records).
    Application(Vec<u8>),
    /// A record that needed no reply (an advisory alert).
    Handled,
    /// Bytes the caller must send: the warning that refuses a renegotiation
    /// request.
    Reply(Vec<u8>),
    /// The peer closed in an orderly way (`close_notify`).
    Closed,
}

/// Which end of the connection this is; decides only what a post-handshake
/// request to renegotiate looks like.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(super) enum Role {
    Client,
    Server,
}

/// An established TLS 1.2 connection, of either role.
///
/// Its error type is [`ClientError`] because this module was written first;
/// a server's connection reports the same variants.
pub struct Connection12 {
    role: Role,
    sealer: Sealer,
    opener: Opener,
    suite: CipherSuite12,
    certificates: Vec<Vec<u8>>,
    alpn: Option<Vec<u8>>,
    closed: bool,
    failed: bool,
    /// Records that carried nothing, so they cannot go on for ever.
    noise: Noise,
}

impl Connection12 {
    pub(super) fn new(
        role: Role,
        sealer: Sealer,
        opener: Opener,
        suite: CipherSuite12,
        certificates: Vec<Vec<u8>>,
    ) -> Self {
        Self {
            role,
            sealer,
            opener,
            suite,
            certificates,
            alpn: None,
            closed: false,
            failed: false,
            noise: Noise::default(),
        }
    }

    /// The negotiated cipher suite.
    pub fn suite(&self) -> CipherSuite12 {
        self.suite
    }

    /// The application protocol ALPN selected, or `None` if none was.
    pub fn alpn_protocol(&self) -> Option<&[u8]> {
        self.alpn.as_deref()
    }

    pub(super) fn with_alpn(mut self, alpn: Option<Vec<u8>>) -> Self {
        self.alpn = alpn;
        self
    }

    /// The peer's certificate chain as DER, end-entity first: the server's, on
    /// a client; the client's, if it authenticated, on a server.
    pub fn peer_certificates(&self) -> &[Vec<u8>] {
        &self.certificates
    }

    /// Protect application data, splitting it into records of at most 2^14.
    /// Empty input produces no records.
    pub fn write(&mut self, data: &[u8]) -> Result<Vec<u8>> {
        if self.closed || self.failed {
            return Err(ClientError::Failed);
        }
        let mut out = Vec::new();
        for chunk in data.chunks(MAX_FRAGMENT_LEN) {
            out.extend(self.sealer.seal(ContentType::ApplicationData, chunk)?);
        }
        Ok(out)
    }

    /// The fatal alert record to send for `error`, protected.
    pub fn alert_record(&mut self, error: &ClientError) -> Option<Vec<u8>> {
        let description = error.alert()?;
        self.sealer
            .seal(ContentType::Alert, &[2, description.0])
            .ok()
    }

    /// Send `close_notify` and stop writing. The peer's own `close_notify` is
    /// still read.
    pub fn close(&mut self) -> Result<Vec<u8>> {
        if self.failed {
            return Err(ClientError::Failed);
        }
        self.closed = true;
        Ok(self
            .sealer
            .seal(ContentType::Alert, &[1, AlertDescription::CLOSE_NOTIFY.0])?)
    }

    /// Unprotect one whole record.
    ///
    /// Any failure to authenticate is permanent: TLS 1.2 makes a bad record MAC
    /// fatal (RFC 5246 §6.2.3.3), so every later call returns
    /// [`ClientError::Failed`].
    pub fn read(&mut self, record: &[u8]) -> Result<Incoming12> {
        if self.failed {
            return Err(ClientError::Failed);
        }
        match self.read_inner(record) {
            Ok(incoming) => Ok(incoming),
            Err(err) => {
                self.failed = true;
                Err(err)
            }
        }
    }

    fn read_inner(&mut self, record: &[u8]) -> Result<Incoming12> {
        let opened = self.opener.open(record)?;
        match opened.typ {
            ContentType::ApplicationData => {
                if opened.fragment.is_empty() {
                    self.noise.empty_record()?;
                } else {
                    self.noise.data();
                }
                Ok(Incoming12::Application(opened.fragment))
            }
            ContentType::Alert => match Alert::parse(&opened.fragment) {
                Some(alert) if alert.description == AlertDescription::CLOSE_NOTIFY => {
                    self.closed = true;
                    Ok(Incoming12::Closed)
                }
                Some(alert) if alert.level == AlertLevel::Warning => {
                    self.noise.warning()?;
                    Ok(Incoming12::Handled)
                }
                Some(alert) => Err(ClientError::PeerAlert(alert)),
                None => Err(ClientError::UnexpectedContentType(ContentType::Alert)),
            },
            ContentType::Handshake => {
                // The only post-handshake message expected is a request to
                // renegotiate: a HelloRequest to a client, a ClientHello to a
                // server. Renegotiation is not implemented, and RFC 5746 says
                // to refuse it with a no_renegotiation warning and carry on.
                if self.asks_to_renegotiate(&opened.fragment) {
                    let reply = self
                        .sealer
                        .seal(ContentType::Alert, &[1, NO_RENEGOTIATION.0])?;
                    return Ok(Incoming12::Reply(reply));
                }
                let got = opened
                    .fragment
                    .first()
                    .map_or(HandshakeType::Unknown(0), |&t| HandshakeType::from_u8(t));
                Err(ClientError::UnexpectedMessage {
                    expected: "application data",
                    got,
                })
            }
            other => Err(ClientError::UnexpectedContentType(other)),
        }
    }
}

impl Connection12 {
    /// Whether `fragment` is, whole, the message that starts a renegotiation.
    /// A fragment that merely begins like one is not: it is an error.
    fn asks_to_renegotiate(&self, fragment: &[u8]) -> bool {
        match self.role {
            Role::Client => fragment == [0, 0, 0, 0],
            Role::Server => match fragment {
                [typ, rest @ ..] if *typ == HandshakeType::ClientHello.as_u8() => {
                    rest.len() >= 3
                        && rest.len() - 3
                            == usize::from(rest[0]) << 16
                                | usize::from(rest[1]) << 8
                                | usize::from(rest[2])
                }
                _ => false,
            },
        }
    }
}

impl core::fmt::Debug for Connection12 {
    fn fmt(&self, f: &mut core::fmt::Formatter<'_>) -> core::fmt::Result {
        f.debug_struct("Connection12")
            .field("suite", &self.suite)
            .field("closed", &self.closed)
            .finish_non_exhaustive()
    }
}
