//! The TLS 1.2 server handshake — stage 4b-iv.
//!
//! The mirror of [`super::client12`], over the same pieces: the record layer,
//! the key derivation, the messages, and — shared with the TLS 1.3 server —
//! the signing key, key exchange and client-certificate path validation.
//!
//! ```text
//! Client                                           Server
//!   ClientHello  ------------------------------->
//!                                                 ServerHello
//!                                                 Certificate
//!                                                 ServerKeyExchange   (signed)
//!                                                 [CertificateRequest]
//!                <-------------------------------  ServerHelloDone
//!   [Certificate]
//!   ClientKeyExchange
//!   [CertificateVerify]
//!   ChangeCipherSpec
//!   Finished     ------------------------------->
//!                                                 ChangeCipherSpec
//!                <-------------------------------  Finished
//! ```
//!
//! # What this refuses
//!
//! A server answers whoever connects, so it holds the same line the client
//! does, for the same reasons, and names each refusal:
//!
//! - **No extended master secret** ([`ServerError::MissingExtendedMasterSecret`]).
//! - **No secure renegotiation** ([`ServerError::BadRenegotiationInfo`]): the
//!   `renegotiation_info` extension or the `0x00ff` signalling suite, and a
//!   non-empty `renegotiated_connection` is never right on a first handshake.
//! - **Anything but ECDHE with an AEAD**, and nothing the signing key cannot
//!   authenticate: a P-256 key is never offered as an RSA suite.
//! - **Resumption.** The `ServerHello` carries an empty session id and no
//!   ticket extension, which is how a server says "start over".
//! - **A client that does not offer TLS 1.2** ([`ServerError::NotTls12`]),
//!   whether by `client_version` or by a `supported_versions` list without it.
//! - **A client that does not accept the certificate's curve.** RFC 8422 §5.1
//!   forbids using a key whose curve the client did not list.
//!
//! The server signs with what its [`SigningKey`] offers: ECDSA and Ed25519 as
//! the key dictates, and RSA-PSS for RSA. It cannot produce a PKCS#1 v1.5
//! signature, which only matters to a client that offers nothing else.
//!
//! # Client authentication
//!
//! Optional, and configured with the same [`ClientAuth`] as the TLS 1.3
//! server, so "ask, and accept whoever turns up empty-handed" versus "ask, and
//! refuse them" is still said out loud. A client that presents a certificate
//! must prove the key with a `CertificateVerify`, which in TLS 1.2 signs the
//! raw handshake messages so far — the scheme hashes them — and is checked
//! with [`verify_tls12_signature`].
//!
//! # Downgrade protection
//!
//! Standing alone, this server speaks TLS 1.2 and nothing else, so it has nothing
//! to be downgraded *from* and writes no `DOWNGRD` sentinel: a TLS 1.2 client of
//! a server that might also have spoken 1.3 would otherwise be told, falsely,
//! that it was attacked. Inside [`super::negotiate`] the same server is the
//! 1.2 half of one that speaks both, and writes it.

use super::client::Alert;
use super::client12::{CipherSuite12, Connection12, Role};
use super::handshake::{
    self, choose_alpn, complete_prefix, encode_alpn_selection, extension, find, messages,
    AlpnChoice, Extension, HandshakeError, HandshakeType, Message,
};
use super::handshake12::{
    self, message, parse_client_key_exchange, parse_finished, Certificate12, CertificateRequest12,
    ClientHello12, ServerHello12, ServerKeyExchange, TLS12,
};
use super::kx::{KeyExchange, NamedGroup};
use super::limits::Noise;
use super::path::{require_signing_key_usage, validate_path};
use super::record::{Aead, ContentType, RecordError, MAX_FRAGMENT_LEN};
use super::record12::{split, Opener, Sealer};
use super::schedule::Hash;
use super::schedule12::{
    extended_master_secret, finished_verify_data, key_block, verify_finished, MasterSecret, Side,
    RANDOM_LEN,
};
use super::server::{plaintext_record, random_bytes, ClientAuth, ServerError};
use super::sign::SigningKey;
use super::verify::{verify_tls12_signature, SignatureScheme};
use super::wire::Reader;
use super::x509::Certificate;

use super::client12::{Authentication, MAX_HANDSHAKE_BUFFER};

type Result<T> = core::result::Result<T, ServerError>;

/// `TLS_EMPTY_RENEGOTIATION_INFO_SCSV`, RFC 5746 §3.3.
const EMPTY_RENEGOTIATION_SCSV: u16 = 0x00ff;
/// `certificate_types`: `rsa_sign(1)` and `ecdsa_sign(64)`. The latter is what
/// RFC 8422 §3 uses for Ed25519 certificates as well.
const CERTIFICATE_TYPES: &[u8] = &[1, 64];

/// What a TLS 1.2 server needs before it can answer anything.
pub struct ServerConfig12<'a> {
    /// The certificate chain, DER-encoded, end-entity first.
    pub certificates: &'a [Vec<u8>],
    /// The private key for the end-entity certificate.
    pub key: &'a SigningKey,
    /// The cipher suites this server will select, most preferred first.
    pub cipher_suites: &'a [CipherSuite12],
    /// The key exchange groups this server will use, most preferred first.
    pub groups: &'a [NamedGroup],
    /// Whether, and how, to ask the client to authenticate. `None` sends no
    /// `CertificateRequest`.
    pub client_auth: Option<&'a ClientAuth<'a>>,
    /// Application protocols this server speaks (RFC 7301), most preferred
    /// first; the first the client also offered is selected. Empty ignores
    /// ALPN. A client offering some and sharing none gets
    /// `no_application_protocol`.
    pub alpn: &'a [&'a [u8]],
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum Expect {
    ClientHello,
    ClientCertificate,
    ClientKeyExchange,
    CertificateVerify,
    ChangeCipherSpec,
    Finished,
}

impl Expect {
    const fn name(self) -> &'static str {
        match self {
            Self::ClientHello => "ClientHello",
            Self::ClientCertificate => "Certificate",
            Self::ClientKeyExchange => "ClientKeyExchange",
            Self::CertificateVerify => "CertificateVerify",
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
    /// Every handshake message so far, encoded, in order: what `Finished`
    /// MACs and what the session hash and a `CertificateVerify` cover.
    transcript: Vec<u8>,
    suite: CipherSuite12,
    aead: Aead,
    hash: Hash,
    /// The server's ephemeral key, consumed by `ClientKeyExchange`.
    kx: Option<KeyExchange>,
    client_certificates: Vec<Vec<u8>>,
    /// The application protocol selected by ALPN, if any.
    alpn: Option<Vec<u8>>,
    /// Set once `ClientKeyExchange` has been processed.
    established: Option<Established>,
}

struct Established {
    master: MasterSecret,
    /// Not used until the server's own `ChangeCipherSpec` is sent.
    sealer: Sealer,
    opener: Opener,
}

/// A TLS 1.2 server handshake in progress.
pub struct ServerHandshake12<'a> {
    config: &'a ServerConfig12<'a>,
    phase: Phase,
    hs: Hs,
    /// Handshake bytes reassembled across records.
    buffer: Vec<u8>,
    connection: Option<Connection12>,
    /// True when this server also speaks TLS 1.3 and is answering in 1.2
    /// anyway, which RFC 8446 §4.1.3 requires it to say in `random`.
    signals_tls13: bool,
    /// Records that carried nothing, so they cannot go on for ever.
    noise: Noise,
}

/// The last eight octets of a `ServerHello.random` from a TLS 1.3-capable
/// server that negotiates TLS 1.2 (RFC 8446 §4.1.3): `"DOWNGRD"` and `0x01`.
pub const DOWNGRADE_SENTINEL: [u8; 8] = [0x44, 0x4f, 0x57, 0x4e, 0x47, 0x52, 0x44, 0x01];

/// How a signing key authenticates the server in a suite's terms.
fn key_authentication(key: &SigningKey) -> Authentication {
    if key
        .schemes()
        .contains(&SignatureScheme::RSA_PSS_RSAE_SHA256)
    {
        Authentication::Rsa
    } else {
        Authentication::Ecdsa
    }
}

/// The group an ECDSA scheme's curve is, or `None` for schemes that name none.
fn scheme_curve(scheme: SignatureScheme) -> Option<NamedGroup> {
    match scheme {
        SignatureScheme::ECDSA_SECP256R1_SHA256 => Some(NamedGroup::SecP256R1),
        SignatureScheme::ECDSA_SECP384R1_SHA384 => Some(NamedGroup::SecP384R1),
        _ => None,
    }
}

/// A big-endian `u16` list inside an extension that is exactly that list.
fn u16_list(data: &[u8]) -> core::result::Result<Vec<u16>, HandshakeError> {
    let mut reader = Reader::new(data);
    let mut list = reader.sub_u16()?;
    reader.finish()?;
    let mut out = Vec::new();
    while !list.is_empty() {
        out.push(list.u16()?);
    }
    Ok(out)
}

/// Split handshake bytes into records of at most 2^14.
fn handshake_records(bytes: &[u8]) -> Result<Vec<u8>> {
    let mut out = Vec::new();
    for chunk in bytes.chunks(MAX_FRAGMENT_LEN) {
        out.extend(plaintext_record(ContentType::Handshake, chunk)?);
    }
    Ok(out)
}

impl<'a> ServerHandshake12<'a> {
    /// Begin waiting for a ClientHello. Unlike a client, a server has nothing
    /// to send first.
    pub fn new(config: &'a ServerConfig12<'a>) -> Result<Self> {
        if config.certificates.is_empty() {
            return Err(HandshakeError::Empty("certificates").into());
        }
        if config.cipher_suites.is_empty() {
            return Err(HandshakeError::Empty("cipher_suites").into());
        }
        if config.groups.is_empty() {
            return Err(HandshakeError::Empty("groups").into());
        }
        for suite in config.cipher_suites {
            if suite.parts().is_none() {
                return Err(ServerError::NoSharedCipherSuite);
            }
        }
        let first = config.cipher_suites[0];
        let (aead, hash, _) = first.parts().ok_or(ServerError::Failed)?;
        Ok(Self {
            config,
            phase: Phase::Expecting(Expect::ClientHello),
            hs: Hs {
                client_random: [0; RANDOM_LEN],
                server_random: [0; RANDOM_LEN],
                transcript: Vec::new(),
                // Placeholders until the ClientHello selects a suite.
                suite: first,
                aead,
                hash,
                kx: None,
                client_certificates: Vec::new(),
                alpn: None,
                established: None,
            },
            buffer: Vec::new(),
            connection: None,
            signals_tls13: false,
            noise: Noise::default(),
        })
    }

    /// As [`Self::new`], for a server that also speaks TLS 1.3 and has been
    /// handed a client that will not: the `ServerHello` then carries the
    /// `DOWNGRD` sentinel, which is how a client that *did* offer 1.3 learns
    /// its hello was tampered with on the way. A server that speaks only 1.2
    /// must not send it, so this is a different constructor and not a default.
    pub(super) fn downgraded(config: &'a ServerConfig12<'a>) -> Result<Self> {
        Ok(Self {
            signals_tls13: true,
            ..Self::new(config)?
        })
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
            _ => Err(ServerError::Failed),
        }
    }

    /// The fatal alert record to send for `error`, if the peer should be told.
    ///
    /// Always in the clear: the server's write keys are not in force until its
    /// own `ChangeCipherSpec`, which no failure path sends.
    pub fn alert_record(&self, error: &ServerError) -> Option<Vec<u8>> {
        let description = error.alert()?;
        plaintext_record(ContentType::Alert, &[2, description.0]).ok()
    }

    /// Feed one whole TLS record, and get back the bytes to send in reply.
    ///
    /// The reply is empty except after the `ClientHello` (the server's whole
    /// first flight) and after the client's `Finished` (`ChangeCipherSpec` and
    /// the server's `Finished`). A failure is permanent: every later call
    /// returns [`ServerError::Failed`].
    pub fn read_record(&mut self, record: &[u8]) -> Result<Vec<u8>> {
        if !matches!(self.phase, Phase::Expecting(_)) {
            return Err(ServerError::Failed);
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
            _ => Err(ServerError::Failed),
        }
    }

    fn read_record_inner(&mut self, record: &[u8]) -> Result<Vec<u8>> {
        let (typ, version, fragment) = split(record)?;
        let expect = self.expecting()?;

        // Only the client's Finished is protected; its ChangeCipherSpec is not.
        if expect == Expect::Finished {
            return self.read_protected(record);
        }
        // The version of a plaintext record is a hint: clients differ on
        // whether the first record says 3.1 or 3.3. Major 3 is TLS at all.
        if version[0] != 3 {
            return Err(RecordError::UnexpectedVersion(version).into());
        }
        match typ {
            ContentType::Alert => self.plaintext_alert(fragment),
            ContentType::ChangeCipherSpec => self.change_cipher_spec(expect, fragment),
            ContentType::Handshake if expect != Expect::ChangeCipherSpec => {
                self.append_and_drain(fragment)
            }
            other => Err(ServerError::UnexpectedContentType(other)),
        }
    }

    fn plaintext_alert(&mut self, fragment: &[u8]) -> Result<Vec<u8>> {
        match Alert::parse(fragment) {
            Some(alert) if alert.is_advisory() => {
                self.noise.warning()?;
                Ok(Vec::new())
            }
            Some(alert) => Err(ServerError::PeerAlert(alert)),
            None => Err(ServerError::UnexpectedContentType(ContentType::Alert)),
        }
    }

    /// Accepted at exactly one moment: after the client's last handshake
    /// message before `Finished`, as the single octet `0x01`, with no
    /// handshake bytes half-read. See [`super::client12`] on CVE-2014-0224.
    fn change_cipher_spec(&mut self, expect: Expect, fragment: &[u8]) -> Result<Vec<u8>> {
        if expect != Expect::ChangeCipherSpec || fragment != [1] || !self.buffer.is_empty() {
            return Err(ServerError::UnexpectedChangeCipherSpec);
        }
        self.phase = Phase::Expecting(Expect::Finished);
        Ok(Vec::new())
    }

    fn read_protected(&mut self, record: &[u8]) -> Result<Vec<u8>> {
        let established = self.hs.established.as_mut().ok_or(ServerError::Failed)?;
        let opened = established.opener.open(record)?;
        match opened.typ {
            ContentType::Handshake => self.append_and_drain(&opened.fragment),
            ContentType::Alert => Err(match Alert::parse(&opened.fragment) {
                Some(alert) => ServerError::PeerAlert(alert),
                None => ServerError::UnexpectedContentType(ContentType::Alert),
            }),
            other => Err(ServerError::UnexpectedContentType(other)),
        }
    }

    fn append_and_drain(&mut self, fragment: &[u8]) -> Result<Vec<u8>> {
        if self.buffer.len() + fragment.len() > MAX_HANDSHAKE_BUFFER {
            return Err(ServerError::HandshakeTooLarge);
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
                // The server answers a ClientHello with a whole flight and then
                // waits; a client has nothing legitimate to say before it has
                // read that flight, so anything in the same record, or half-read,
                // is refused (BoGo `Partial*WithClientHello`) — before the
                // flight is built.
                if msg.typ == HandshakeType::ClientHello
                    && (index != last || !self.buffer.is_empty())
                {
                    return Err(ServerError::UnexpectedMessage {
                        expected: "nothing after ClientHello",
                        got: msg.typ,
                    });
                }
                reply.extend_from_slice(&self.handle_message(msg)?);
            }
        }
    }

    fn handle_message(&mut self, msg: &Message<'_>) -> Result<Vec<u8>> {
        // A message after the client's Finished, in the same record, is a
        // violation worth naming; it must not be handled.
        if matches!(self.phase, Phase::Done) {
            return Err(ServerError::UnexpectedMessage {
                expected: "nothing after Finished",
                got: msg.typ,
            });
        }
        let expect = self.expecting()?;
        match (expect, msg.typ) {
            (Expect::ClientHello, HandshakeType::ClientHello) => {
                self.hs.transcript.extend_from_slice(msg.encoded);
                self.client_hello(msg.body)
            }
            (Expect::ClientCertificate, HandshakeType::Certificate) => {
                self.hs.transcript.extend_from_slice(msg.encoded);
                self.client_certificate(msg.body)?;
                Ok(Vec::new())
            }
            (Expect::ClientKeyExchange, HandshakeType::ClientKeyExchange) => {
                self.hs.transcript.extend_from_slice(msg.encoded);
                self.client_key_exchange(msg.body)?;
                Ok(Vec::new())
            }
            (Expect::CertificateVerify, HandshakeType::CertificateVerify) => {
                self.certificate_verify(msg)?;
                Ok(Vec::new())
            }
            (Expect::Finished, HandshakeType::Finished) => self.client_finished(msg),
            _ => Err(ServerError::UnexpectedMessage {
                expected: expect.name(),
                got: msg.typ,
            }),
        }
    }

    /// Validate the offer, choose parameters, and build the whole first flight.
    fn client_hello(&mut self, body: &[u8]) -> Result<Vec<u8>> {
        let hello = ClientHello12::parse(body)?;
        let config = self.config;

        // 1. Version. client_version 0x0303 is also what a TLS 1.3 client
        // sends, so `supported_versions` has the say when it is present (RFC
        // 8446 §4.2.1, and BoGo `ConflictingVersionNegotiation-2`); the field
        // is consulted only when it is not.
        match hello.supported_versions()? {
            Some(versions) if !versions.contains(&TLS12) => {
                return Err(ServerError::NotTls12(hello.version))
            }
            None if hello.version < TLS12 => return Err(ServerError::NotTls12(hello.version)),
            _ => {}
        }
        if !hello.compression.contains(&0) {
            return Err(ServerError::UnacceptableOffer(
                "null compression not offered",
            ));
        }

        // 2. The things this server insists on.
        match find(&hello.extensions, extension::EXTENDED_MASTER_SECRET) {
            Some([]) => {}
            _ => return Err(ServerError::MissingExtendedMasterSecret),
        }
        match find(&hello.extensions, extension::RENEGOTIATION_INFO) {
            Some([0]) => {}
            Some(_) => return Err(ServerError::BadRenegotiationInfo),
            None if hello.cipher_suites.contains(&EMPTY_RENEGOTIATION_SCSV) => {}
            None => return Err(ServerError::BadRenegotiationInfo),
        }
        let point_formats = find(&hello.extensions, extension::EC_POINT_FORMATS);
        if let Some(formats) = point_formats {
            match formats.split_first() {
                Some((&len, list)) if usize::from(len) == list.len() && list.contains(&0) => {}
                _ => {
                    return Err(ServerError::UnacceptableOffer(
                        "no uncompressed point format",
                    ))
                }
            }
        }

        // 3. Parameters. The key decides what can be offered; the client
        // narrows it.
        let client_groups = find(&hello.extensions, extension::SUPPORTED_GROUPS)
            .map(u16_list)
            .transpose()?
            .ok_or(ServerError::NoSharedGroup)?;
        let client_schemes = find(&hello.extensions, extension::SIGNATURE_ALGORITHMS)
            .map(u16_list)
            .transpose()?
            .ok_or(ServerError::NoSharedSignatureScheme)?;

        let scheme = config
            .key
            .schemes()
            .iter()
            .copied()
            .filter(|s| client_schemes.contains(&s.0))
            .find(|s| scheme_curve(*s).is_none_or(|curve| client_groups.contains(&curve.as_u16())))
            .ok_or(ServerError::NoSharedSignatureScheme)?;

        let authentication = key_authentication(config.key);
        let (suite, aead, hash) = config
            .cipher_suites
            .iter()
            .filter(|s| hello.cipher_suites.contains(&s.0))
            .find_map(|s| match s.parts() {
                Some((aead, hash, auth)) if auth == authentication => Some((*s, aead, hash)),
                _ => None,
            })
            .ok_or(ServerError::NoSharedCipherSuite)?;
        let group = config
            .groups
            .iter()
            .copied()
            .find(|g| client_groups.contains(&g.as_u16()))
            .ok_or(ServerError::NoSharedGroup)?;

        // 4. The flight.
        let mut random = random_bytes(RANDOM_LEN)?;
        if self.signals_tls13 {
            random[RANDOM_LEN - 8..].copy_from_slice(&DOWNGRADE_SENTINEL);
        }
        self.hs.client_random.copy_from_slice(hello.random);
        self.hs.server_random.copy_from_slice(&random);
        self.hs.suite = suite;
        self.hs.aead = aead;
        self.hs.hash = hash;

        let mut extensions = vec![
            Extension {
                typ: extension::RENEGOTIATION_INFO,
                data: &[0],
            },
            Extension {
                typ: extension::EXTENDED_MASTER_SECRET,
                data: &[],
            },
        ];
        if point_formats.is_some() {
            extensions.push(Extension {
                typ: extension::EC_POINT_FORMATS,
                data: &[1, 0],
            });
        }
        let alpn_answer;
        match choose_alpn(config.alpn, &hello.extensions)? {
            AlpnChoice::Unused => {}
            AlpnChoice::NoOverlap => return Err(ServerError::NoApplicationProtocol),
            AlpnChoice::Selected(protocol) => {
                alpn_answer = encode_alpn_selection(protocol);
                extensions.push(Extension {
                    typ: extension::ALPN,
                    data: &alpn_answer,
                });
                self.hs.alpn = Some(protocol.to_vec());
            }
        }
        let server_hello = ServerHello12 {
            version: TLS12,
            random: &random,
            session_id: &[],
            cipher_suite: suite.0,
            compression: 0,
            extensions,
        };
        let mut flight = message(HandshakeType::ServerHello, &server_hello.encode());

        let chain: Vec<&[u8]> = config.certificates.iter().map(Vec::as_slice).collect();
        flight.extend(message(
            HandshakeType::Certificate,
            &Certificate12::encode(&chain),
        ));

        // The signature covers both randoms as well as the key, so a captured
        // ServerKeyExchange cannot be replayed into another handshake.
        let kx = KeyExchange::generate(group)?;
        let params = ServerKeyExchange::encode_params(group.as_u16(), kx.public_key());
        let signed = handshake12::signed_content(&self.hs.client_random, &random, &params);
        let signature = config.key.sign(scheme, &signed)?;
        flight.extend(message(
            HandshakeType::ServerKeyExchange,
            &ServerKeyExchange::encode(&params, scheme.0, &signature),
        ));
        self.hs.kx = Some(kx);

        if config.client_auth.is_some() {
            let request = CertificateRequest12 {
                certificate_types: CERTIFICATE_TYPES,
                signature_algorithms: SignatureScheme::TLS12_SUPPORTED
                    .iter()
                    .map(|s| s.0)
                    .collect(),
                authorities: &[],
            };
            flight.extend(message(
                HandshakeType::CertificateRequest,
                &request.encode(),
            ));
        }
        flight.extend(message(HandshakeType::ServerHelloDone, &[]));

        self.hs.transcript.extend_from_slice(&flight);
        self.phase = Phase::Expecting(if config.client_auth.is_some() {
            Expect::ClientCertificate
        } else {
            Expect::ClientKeyExchange
        });
        handshake_records(&flight)
    }

    fn client_certificate(&mut self, body: &[u8]) -> Result<()> {
        let auth = self.config.client_auth.ok_or(ServerError::Failed)?;
        let chain = Certificate12::parse(body)?;
        let Some((end_entity, rest)) = chain.certificates.split_first() else {
            // An empty Certificate is the conforming way to say "none". Whether
            // that is acceptable is this server's decision.
            if auth.required {
                return Err(ServerError::ClientCertificateRequired);
            }
            self.phase = Phase::Expecting(Expect::ClientKeyExchange);
            return Ok(());
        };

        let leaf =
            Certificate::parse(end_entity).map_err(ServerError::MalformedClientCertificate)?;
        let intermediates = rest
            .iter()
            .map(|der| Certificate::parse(der))
            .collect::<core::result::Result<Vec<_>, _>>()
            .map_err(ServerError::MalformedClientCertificate)?;
        // No name check: a client certificate identifies a client, and there is
        // no hostname for it to match.
        validate_path(&leaf, &intermediates, auth.anchors, &auth.path)
            .map(|_| ())
            .and_then(|()| require_signing_key_usage(&leaf))
            .map_err(ServerError::ClientCertificate)?;

        self.hs.client_certificates = chain.certificates.iter().map(|der| der.to_vec()).collect();
        self.phase = Phase::Expecting(Expect::ClientKeyExchange);
        Ok(())
    }

    fn client_key_exchange(&mut self, body: &[u8]) -> Result<()> {
        let public = parse_client_key_exchange(body)?;
        let kx = self.hs.kx.take().ok_or(ServerError::Failed)?;
        let pre_master = kx.agree(public, |shared| shared.to_vec())?;

        // The session hash covers everything up to and including this message,
        // and not the CertificateVerify that may follow.
        let hash = self.hs.hash;
        let session_hash = hash.hash(&self.hs.transcript);
        let master = extended_master_secret(hash, &pre_master, &session_hash)?;
        let keys = key_block(
            hash,
            self.hs.aead,
            &master,
            &self.hs.client_random,
            &self.hs.server_random,
        );
        let sealer = Sealer::new(self.hs.aead, &keys.server_write_key, &keys.server_write_iv)?;
        let opener = Opener::new(self.hs.aead, &keys.client_write_key, &keys.client_write_iv)?;
        self.hs.established = Some(Established {
            master,
            sealer,
            opener,
        });

        self.phase = Phase::Expecting(if self.hs.client_certificates.is_empty() {
            Expect::ChangeCipherSpec
        } else {
            Expect::CertificateVerify
        });
        Ok(())
    }

    /// RFC 5246 §7.4.8: a signature over every handshake message before this
    /// one, which proves the client holds the certificate's key.
    fn certificate_verify(&mut self, msg: &Message<'_>) -> Result<()> {
        let verify = handshake::CertificateVerify::parse(msg.body)?;
        let leaf = self
            .hs
            .client_certificates
            .first()
            .ok_or(ServerError::Failed)?;
        let leaf = Certificate::parse(leaf).map_err(ServerError::MalformedClientCertificate)?;
        verify_tls12_signature(
            SignatureScheme(verify.scheme),
            &leaf.subject_public_key_info(),
            &self.hs.transcript,
            verify.signature,
        )
        .map_err(ServerError::ClientCertificateVerify)?;

        self.hs.transcript.extend_from_slice(msg.encoded);
        self.phase = Phase::Expecting(Expect::ChangeCipherSpec);
        Ok(())
    }

    fn client_finished(&mut self, msg: &Message<'_>) -> Result<Vec<u8>> {
        // A Finished of the wrong length is a Finished that does not verify.
        let received = parse_finished(msg.body).map_err(|_| ServerError::BadFinished)?;
        // Nothing may follow the Finished in its record, or be left half-read.
        if !self.buffer.is_empty() {
            return Err(ServerError::UnexpectedMessage {
                expected: "nothing after Finished",
                got: HandshakeType::Unknown(0),
            });
        }
        let established = self.hs.established.take().ok_or(ServerError::Failed)?;
        let hash = self.hs.hash;
        if !verify_finished(
            hash,
            &established.master,
            Side::Client,
            &hash.hash(&self.hs.transcript),
            received,
        ) {
            return Err(ServerError::BadFinished);
        }
        self.hs.transcript.extend_from_slice(msg.encoded);

        let verify_data = finished_verify_data(
            hash,
            &established.master,
            Side::Server,
            &hash.hash(&self.hs.transcript),
        )?;
        let mut sealer = established.sealer;
        let finished = sealer.seal(
            ContentType::Handshake,
            &message(HandshakeType::Finished, &verify_data),
        )?;

        let mut reply = plaintext_record(ContentType::ChangeCipherSpec, &[1])?;
        reply.extend(finished);

        self.connection = Some(
            Connection12::new(
                Role::Server,
                sealer,
                established.opener,
                self.hs.suite,
                core::mem::take(&mut self.hs.client_certificates),
            )
            .with_alpn(self.hs.alpn.take()),
        );
        self.phase = Phase::Done;
        Ok(reply)
    }
}

impl core::fmt::Debug for ServerHandshake12<'_> {
    fn fmt(&self, f: &mut core::fmt::Formatter<'_>) -> core::fmt::Result {
        let state = match &self.phase {
            Phase::Expecting(expect) => expect.name(),
            Phase::Done => "done",
            Phase::Failed => "failed",
        };
        f.debug_struct("ServerHandshake12")
            .field("state", &state)
            .finish_non_exhaustive()
    }
}
