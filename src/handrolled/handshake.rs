//! TLS 1.3 handshake messages and the transcript hash — stage 3b.
//!
//! Parsing and encoding for the messages a TLS 1.3 client sends and receives,
//! plus the running transcript hash that [`super::schedule`] binds every
//! traffic secret to.
//!
//! # Round-tripping is a correctness property, not a convenience
//!
//! Every message here parses *and* encodes, and the tests require the two to
//! be inverses on the RFC's own bytes: parse the RFC's ClientHello, encode it
//! again, get the identical 196 octets back.
//!
//! That is not a tidiness check. The transcript hash covers the encoded
//! messages, so a client that parses a message one way and re-encodes it
//! another computes a transcript the server does not share, and the handshake
//! fails in a way that looks like a network problem. Worse, an implementation
//! that *normalises* while re-encoding would hash something the peer never
//! sent — which is the same class of bug as re-encoding a certificate before
//! verifying its signature, and the reason [`super::x509`] keeps
//! `tbsCertificate` as a borrow.
//!
//! For the same reason, [`Transcript`] takes encoded bytes and never a parsed
//! message: there is exactly one path from a message to the hash, and it goes
//! through the bytes that were on the wire.
//!
//! # What is here, and what is not
//!
//! Here: the handshake header, ClientHello, ServerHello (including
//! HelloRetryRequest), EncryptedExtensions, Certificate, CertificateVerify,
//! Finished, and the extensions a TLS 1.3 client needs to read — with
//! everything else preserved rather than dropped.
//!
//! Not here: the state machine, the key exchange, and anything that decides
//! whether a handshake should proceed. Those are stage 3c. This module reports
//! what a message says, in the same sense [`super::x509`] reports what a
//! certificate says.

use super::wire::{Reader, WireError, Writer};

/// Everything handshake parsing can refuse.
#[derive(Clone, Debug, PartialEq, Eq)]
#[non_exhaustive]
pub enum HandshakeError {
    /// The wire encoding underneath was malformed.
    Wire(WireError),
    /// A handshake message's declared length did not match the bytes present.
    ///
    /// Distinct from a bare wire error because it means the framing and the
    /// content disagree, which is the seam a message-smuggling attempt lands
    /// on.
    LengthMismatch {
        /// What the `uint24` header declared.
        declared: usize,
        /// What was actually available.
        available: usize,
    },
    /// A message had a type this parser does not handle in this position.
    UnexpectedMessage(u8),
    /// `legacy_version` was not `0x0303`.
    ///
    /// TLS 1.3 pins it and negotiates the real version through the
    /// `supported_versions` extension; a peer that varies it is not speaking
    /// the protocol this parser implements.
    UnexpectedLegacyVersion(u16),
    /// A ClientHello or ServerHello carried a `legacy_compression_method`
    /// other than the single null method TLS 1.3 permits.
    ///
    /// Compression is where CRIME lived. TLS 1.3 removed it entirely, and
    /// accepting a non-null method here would be accepting a protocol this
    /// code does not implement.
    UnexpectedCompression,
    /// The same extension appeared twice in one message (RFC 8446 §4.2).
    DuplicateExtension(u16),
    /// A field was empty where the grammar requires content.
    Empty(&'static str),
}

impl From<WireError> for HandshakeError {
    fn from(err: WireError) -> Self {
        Self::Wire(err)
    }
}

impl core::fmt::Display for HandshakeError {
    fn fmt(&self, f: &mut core::fmt::Formatter<'_>) -> core::fmt::Result {
        match self {
            Self::Wire(err) => write!(f, "malformed encoding: {err}"),
            Self::LengthMismatch {
                declared,
                available,
            } => write!(
                f,
                "a handshake message declares {declared} bytes, {available} available"
            ),
            Self::UnexpectedMessage(typ) => write!(f, "unexpected handshake message type {typ}"),
            Self::UnexpectedLegacyVersion(version) => {
                write!(f, "legacy_version is 0x{version:04x}, expected 0x0303")
            }
            Self::UnexpectedCompression => {
                f.write_str("a compression method other than null was offered")
            }
            Self::DuplicateExtension(id) => write!(f, "extension {id} appears more than once"),
            Self::Empty(what) => write!(f, "{what} is empty"),
        }
    }
}

impl std::error::Error for HandshakeError {
    fn source(&self) -> Option<&(dyn std::error::Error + 'static)> {
        match self {
            Self::Wire(err) => Some(err),
            _ => None,
        }
    }
}

type Result<T> = core::result::Result<T, HandshakeError>;

/// `legacy_version`, pinned at TLS 1.2's value for every TLS 1.3 message.
pub const LEGACY_VERSION: u16 = 0x0303;

/// The `HandshakeType` values this module knows.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
#[non_exhaustive]
pub enum HandshakeType {
    /// `client_hello(1)`.
    ClientHello,
    /// `server_hello(2)`, which also carries HelloRetryRequest.
    ServerHello,
    /// `new_session_ticket(4)`.
    NewSessionTicket,
    /// `encrypted_extensions(8)`.
    EncryptedExtensions,
    /// `certificate(11)`.
    Certificate,
    /// `certificate_request(13)`.
    CertificateRequest,
    /// `certificate_verify(15)`.
    CertificateVerify,
    /// `finished(20)`.
    Finished,
    /// `key_update(24)`.
    KeyUpdate,
    /// Any other value, preserved.
    Unknown(u8),
}

impl HandshakeType {
    /// The wire encoding.
    pub const fn as_u8(self) -> u8 {
        match self {
            Self::ClientHello => 1,
            Self::ServerHello => 2,
            Self::NewSessionTicket => 4,
            Self::EncryptedExtensions => 8,
            Self::Certificate => 11,
            Self::CertificateRequest => 13,
            Self::CertificateVerify => 15,
            Self::Finished => 20,
            Self::KeyUpdate => 24,
            Self::Unknown(other) => other,
        }
    }

    /// Decode a wire value. Never fails — what to do about an unknown type is
    /// the state machine's decision, not this module's.
    pub const fn from_u8(value: u8) -> Self {
        match value {
            1 => Self::ClientHello,
            2 => Self::ServerHello,
            4 => Self::NewSessionTicket,
            8 => Self::EncryptedExtensions,
            11 => Self::Certificate,
            13 => Self::CertificateRequest,
            15 => Self::CertificateVerify,
            20 => Self::Finished,
            24 => Self::KeyUpdate,
            other => Self::Unknown(other),
        }
    }
}

/// One handshake message's framing: a type, and the body it wraps.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Message<'a> {
    /// The message type.
    pub typ: HandshakeType,
    /// The body, excluding the four-octet header.
    pub body: &'a [u8],
    /// Header and body together, exactly as they appeared.
    ///
    /// This is what goes into the transcript — see the module docs on why the
    /// hash is fed encoded bytes rather than a parsed message.
    pub encoded: &'a [u8],
}

impl Message<'_> {
    /// Encode a message from its type and body.
    pub fn encode(typ: HandshakeType, body: &[u8]) -> Vec<u8> {
        let mut writer = Writer::new();
        writer.u8(typ.as_u8());
        writer.vector_u24(|w| w.bytes(body));
        writer.into_vec()
    }
}

/// Read every handshake message in `input`, in order, requiring that the
/// last one ends exactly where the input does.
///
/// The only way to obtain a [`Message`], deliberately. A `read_one` taking a
/// cursor would be the natural shape, and it cannot produce a correct
/// [`Message::encoded`]: recovering `header || body` needs the whole buffer,
/// and rebuilding the header instead would put a reconstruction into the
/// transcript rather than the bytes that arrived.
///
/// Handshake messages are concatenated within a record and may span records,
/// so a caller reassembles first and reads messages from the result.
pub fn messages(input: &[u8]) -> Result<Vec<Message<'_>>> {
    let mut out = Vec::new();
    let mut offset = 0usize;

    while offset < input.len() {
        let rest = &input[offset..];
        let mut reader = Reader::new(rest);
        let typ = HandshakeType::from_u8(reader.u8()?);
        let declared = reader.u24()? as usize;
        let total = declared
            .checked_add(4)
            .ok_or(HandshakeError::LengthMismatch {
                declared,
                available: rest.len(),
            })?;
        if total > rest.len() {
            return Err(HandshakeError::LengthMismatch {
                declared,
                available: rest.len().saturating_sub(4),
            });
        }

        out.push(Message {
            typ,
            body: &rest[4..total],
            encoded: &rest[..total],
        });
        offset += total;
    }

    Ok(out)
}

/// How many leading bytes of `input` form whole handshake messages.
///
/// [`messages`] requires its input to end exactly on a message boundary, which
/// is right for a buffer someone has finished assembling and wrong for one
/// still filling up: handshake messages may span records, so a client reading
/// a stream routinely holds a partial one. This reports how much is currently
/// complete, so a caller can hand that prefix to [`messages`] and keep the
/// remainder.
///
/// Never fails. A partial trailing message is not an error — it is the normal
/// state of a buffer between records — and a length that has not arrived yet
/// is simply not complete. Deciding that a peer sent something impossible is
/// [`messages`]' job, on a prefix this function has already agreed is whole.
pub fn complete_prefix(input: &[u8]) -> usize {
    let mut offset = 0usize;

    while let Some(header) = input.get(offset..offset + 4) {
        let declared = u32::from_be_bytes([0, header[1], header[2], header[3]]) as usize;
        let Some(total) = declared.checked_add(4) else {
            break;
        };
        if input.len() - offset < total {
            break;
        }
        offset += total;
    }

    offset
}

// ---------------------------------------------------------------------------
// Extensions
// ---------------------------------------------------------------------------

/// One extension, uninterpreted.
///
/// Kept as a type and a body rather than an enum of every known extension:
/// the set is open by design, a client must tolerate ones it does not know,
/// and this module's job is to report what arrived. Interpreting a specific
/// extension is a matter for whoever needs it.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Extension<'a> {
    /// The `ExtensionType` value.
    pub typ: u16,
    /// `extension_data`, uninterpreted.
    pub data: &'a [u8],
}

/// Extension type numbers this crate refers to by name.
pub mod extension {
    /// `server_name(0)` — SNI.
    pub const SERVER_NAME: u16 = 0;
    /// `supported_groups(10)`.
    pub const SUPPORTED_GROUPS: u16 = 10;
    /// `signature_algorithms(13)`.
    pub const SIGNATURE_ALGORITHMS: u16 = 13;
    /// `application_layer_protocol_negotiation(16)` — ALPN.
    pub const ALPN: u16 = 16;
    /// `supported_versions(43)`.
    pub const SUPPORTED_VERSIONS: u16 = 43;
    /// `key_share(51)`.
    pub const KEY_SHARE: u16 = 51;
}

/// Parse an extensions block, rejecting duplicates.
///
/// RFC 8446 §4.2: "There MUST NOT be more than one extension of the same type
/// in a given extension block." A parser that takes the first, or the last,
/// lets a peer say two things and lets two implementations disagree about
/// which one it said.
fn parse_extensions<'a>(reader: &mut Reader<'a>) -> Result<Vec<Extension<'a>>> {
    let mut block = reader.sub_u16()?;
    let mut extensions: Vec<Extension<'_>> = Vec::new();

    while !block.is_empty() {
        let typ = block.u16()?;
        let data = block.vector_u16()?;
        if extensions.iter().any(|e| e.typ == typ) {
            return Err(HandshakeError::DuplicateExtension(typ));
        }
        extensions.push(Extension { typ, data });
    }

    Ok(extensions)
}

fn write_extensions(writer: &mut Writer, extensions: &[Extension<'_>]) {
    writer.vector_u16(|w| {
        for extension in extensions {
            w.u16(extension.typ);
            w.vector_u16(|w| w.bytes(extension.data));
        }
    });
}

/// Look one extension up by type.
pub fn find<'a>(extensions: &[Extension<'a>], typ: u16) -> Option<&'a [u8]> {
    extensions.iter().find(|e| e.typ == typ).map(|e| e.data)
}

// ---------------------------------------------------------------------------
// ClientHello
// ---------------------------------------------------------------------------

/// A ClientHello, RFC 8446 §4.1.2.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct ClientHello<'a> {
    /// `random`, 32 octets.
    pub random: &'a [u8],
    /// `legacy_session_id`, which TLS 1.3 uses only for middlebox
    /// compatibility.
    pub session_id: &'a [u8],
    /// The offered cipher suites, in order.
    pub cipher_suites: Vec<u16>,
    /// The extensions, in order.
    pub extensions: Vec<Extension<'a>>,
}

impl<'a> ClientHello<'a> {
    /// Parse a ClientHello body — the message with its four-octet header
    /// already removed.
    pub fn parse(body: &'a [u8]) -> Result<Self> {
        let mut reader = Reader::new(body);

        let version = reader.u16()?;
        if version != LEGACY_VERSION {
            return Err(HandshakeError::UnexpectedLegacyVersion(version));
        }
        let random = reader.take(32)?;
        let session_id = reader.vector_u8()?;

        let mut suites = reader.sub_u16()?;
        let mut cipher_suites = Vec::new();
        while !suites.is_empty() {
            cipher_suites.push(suites.u16()?);
        }
        if cipher_suites.is_empty() {
            return Err(HandshakeError::Empty("cipher_suites"));
        }

        // TLS 1.3 permits exactly one compression method, null. Compression
        // is where CRIME lived; the protocol removed it, and this refuses to
        // pretend otherwise.
        let compression = reader.vector_u8()?;
        if compression != [0] {
            return Err(HandshakeError::UnexpectedCompression);
        }

        let extensions = parse_extensions(&mut reader)?;
        reader.finish()?;

        Ok(Self {
            random,
            session_id,
            cipher_suites,
            extensions,
        })
    }

    /// Encode the body. Inverse of [`ClientHello::parse`] on any input that
    /// parses — the tests require it byte for byte on the RFC's own message.
    pub fn encode(&self) -> Vec<u8> {
        let mut writer = Writer::new();
        writer.u16(LEGACY_VERSION);
        writer.bytes(self.random);
        writer.vector_u8(|w| w.bytes(self.session_id));
        writer.vector_u16(|w| {
            for suite in &self.cipher_suites {
                w.u16(*suite);
            }
        });
        writer.vector_u8(|w| w.u8(0));
        write_extensions(&mut writer, &self.extensions);
        writer.into_vec()
    }
}

// ---------------------------------------------------------------------------
// ServerHello
// ---------------------------------------------------------------------------

/// The `random` value that marks a ServerHello as a HelloRetryRequest
/// (RFC 8446 §4.1.3) — `SHA-256("HelloRetryRequest")`.
pub const HELLO_RETRY_REQUEST_RANDOM: [u8; 32] = [
    0xcf, 0x21, 0xad, 0x74, 0xe5, 0x9a, 0x61, 0x11, 0xbe, 0x1d, 0x8c, 0x02, 0x1e, 0x65, 0xb8, 0x91,
    0xc2, 0xa2, 0x11, 0x16, 0x7a, 0xbb, 0x8c, 0x5e, 0x07, 0x9e, 0x09, 0xe2, 0xc8, 0xa8, 0x33, 0x9c,
];

/// A ServerHello, RFC 8446 §4.1.3.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct ServerHello<'a> {
    /// `random`, 32 octets.
    pub random: &'a [u8],
    /// The echoed `legacy_session_id_echo`.
    pub session_id: &'a [u8],
    /// The selected cipher suite.
    pub cipher_suite: u16,
    /// The extensions, in order.
    pub extensions: Vec<Extension<'a>>,
}

impl<'a> ServerHello<'a> {
    /// Parse a ServerHello body.
    pub fn parse(body: &'a [u8]) -> Result<Self> {
        let mut reader = Reader::new(body);

        let version = reader.u16()?;
        if version != LEGACY_VERSION {
            return Err(HandshakeError::UnexpectedLegacyVersion(version));
        }
        let random = reader.take(32)?;
        let session_id = reader.vector_u8()?;
        let cipher_suite = reader.u16()?;

        let compression = reader.u8()?;
        if compression != 0 {
            return Err(HandshakeError::UnexpectedCompression);
        }

        let extensions = parse_extensions(&mut reader)?;
        reader.finish()?;

        Ok(Self {
            random,
            session_id,
            cipher_suite,
            extensions,
        })
    }

    /// True if this ServerHello is really a HelloRetryRequest.
    ///
    /// TLS 1.3 encodes a retry as a ServerHello with a fixed `random` rather
    /// than a distinct message type, so a client that only matches on the
    /// message type will treat a retry as a real ServerHello and derive keys
    /// from a handshake that has not happened.
    pub fn is_hello_retry_request(&self) -> bool {
        self.random == HELLO_RETRY_REQUEST_RANDOM
    }

    /// Encode the body.
    pub fn encode(&self) -> Vec<u8> {
        let mut writer = Writer::new();
        writer.u16(LEGACY_VERSION);
        writer.bytes(self.random);
        writer.vector_u8(|w| w.bytes(self.session_id));
        writer.u16(self.cipher_suite);
        writer.u8(0);
        write_extensions(&mut writer, &self.extensions);
        writer.into_vec()
    }
}

// ---------------------------------------------------------------------------
// The encrypted-flight messages
// ---------------------------------------------------------------------------

/// An EncryptedExtensions message, RFC 8446 §4.3.1: extensions and nothing
/// else.
pub fn parse_encrypted_extensions(body: &[u8]) -> Result<Vec<Extension<'_>>> {
    let mut reader = Reader::new(body);
    let extensions = parse_extensions(&mut reader)?;
    reader.finish()?;
    Ok(extensions)
}

/// One entry in a Certificate message.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct CertificateEntry<'a> {
    /// The certificate, DER-encoded — feed straight to
    /// [`super::x509::Certificate::parse`].
    pub certificate: &'a [u8],
    /// This entry's extensions, uninterpreted.
    pub extensions: &'a [u8],
}

/// A Certificate message, RFC 8446 §4.4.2.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct CertificateMessage<'a> {
    /// `certificate_request_context`, empty for a server's Certificate in a
    /// handshake the client did not request a certificate in.
    pub context: &'a [u8],
    /// The chain, end-entity certificate first.
    pub entries: Vec<CertificateEntry<'a>>,
}

impl<'a> CertificateMessage<'a> {
    /// Parse a Certificate body.
    pub fn parse(body: &'a [u8]) -> Result<Self> {
        let mut reader = Reader::new(body);
        let context = reader.vector_u8()?;

        let mut list = reader.sub_u24()?;
        let mut entries = Vec::new();
        while !list.is_empty() {
            let certificate = list.vector_u24()?;
            let extensions = list.vector_u16()?;
            entries.push(CertificateEntry {
                certificate,
                extensions,
            });
        }
        reader.finish()?;

        Ok(Self { context, entries })
    }

    /// Encode the body.
    pub fn encode(&self) -> Vec<u8> {
        let mut writer = Writer::new();
        writer.vector_u8(|w| w.bytes(self.context));
        writer.vector_u24(|w| {
            for entry in &self.entries {
                w.vector_u24(|w| w.bytes(entry.certificate));
                w.vector_u16(|w| w.bytes(entry.extensions));
            }
        });
        writer.into_vec()
    }
}

/// A CertificateVerify message, RFC 8446 §4.4.3.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct CertificateVerify<'a> {
    /// The `SignatureScheme` the signature uses.
    pub scheme: u16,
    /// The signature itself.
    pub signature: &'a [u8],
}

impl<'a> CertificateVerify<'a> {
    /// Parse a CertificateVerify body.
    pub fn parse(body: &'a [u8]) -> Result<Self> {
        let mut reader = Reader::new(body);
        let scheme = reader.u16()?;
        let signature = reader.vector_u16()?;
        reader.finish()?;
        if signature.is_empty() {
            return Err(HandshakeError::Empty("signature"));
        }
        Ok(Self { scheme, signature })
    }

    /// Encode the body.
    pub fn encode(&self) -> Vec<u8> {
        let mut writer = Writer::new();
        writer.u16(self.scheme);
        writer.vector_u16(|w| w.bytes(self.signature));
        writer.into_vec()
    }
}

/// The bytes a CertificateVerify signature is computed over, RFC 8446 §4.4.3.
///
/// ```text
/// 64 octets of 0x20, then a context string, then 0x00, then the transcript
/// hash.
/// ```
///
/// The padding and context string are not decoration. Without them the signed
/// blob would be a bare hash, and a signature over a bare hash from one
/// protocol is a valid signature over the same hash in another — this is the
/// cross-protocol attack the construction exists to prevent, and it is why the
/// client and server context strings differ.
pub fn certificate_verify_content(context: &str, transcript_hash: &[u8]) -> Vec<u8> {
    let mut out = Vec::with_capacity(64 + context.len() + 1 + transcript_hash.len());
    out.extend(core::iter::repeat_n(0x20u8, 64));
    out.extend_from_slice(context.as_bytes());
    out.push(0x00);
    out.extend_from_slice(transcript_hash);
    out
}

/// The context string for a signature made by a server.
pub const SERVER_CERTIFICATE_VERIFY_CONTEXT: &str = "TLS 1.3, server CertificateVerify";
/// The context string for a signature made by a client.
pub const CLIENT_CERTIFICATE_VERIFY_CONTEXT: &str = "TLS 1.3, client CertificateVerify";

/// A Finished message's body is its `verify_data` and nothing else.
pub fn parse_finished(body: &[u8]) -> Result<&[u8]> {
    if body.is_empty() {
        return Err(HandshakeError::Empty("verify_data"));
    }
    Ok(body)
}

// ---------------------------------------------------------------------------
// Transcript
// ---------------------------------------------------------------------------

/// The running hash of every handshake message, in order.
///
/// Deliberately accepts encoded bytes and never a parsed message: the hash
/// must cover what was on the wire, and offering an `add(&ClientHello)` would
/// invite hashing a re-encoding that differs from what arrived. See the module
/// docs.
#[derive(Clone, Debug)]
pub struct Transcript {
    hash: super::schedule::Hash,
    buffer: Vec<u8>,
}

impl Transcript {
    /// Start an empty transcript for a cipher suite's hash.
    pub fn new(hash: super::schedule::Hash) -> Self {
        Self {
            hash,
            buffer: Vec::new(),
        }
    }

    /// Append one encoded handshake message, header included.
    pub fn add(&mut self, encoded: &[u8]) {
        self.buffer.extend_from_slice(encoded);
    }

    /// Append a message's encoded form.
    pub fn add_message(&mut self, message: &Message<'_>) {
        self.add(message.encoded);
    }

    /// The hash of everything added so far.
    ///
    /// TLS needs the transcript hash at several points *without* ending it —
    /// the ClientHello..ServerHello hash is used while more messages are still
    /// to come — so this does not consume the transcript.
    pub fn hash(&self) -> Vec<u8> {
        self.hash.hash(&self.buffer)
    }

    /// The bytes accumulated so far, for a caller that needs them directly.
    pub fn as_bytes(&self) -> &[u8] {
        &self.buffer
    }
}
