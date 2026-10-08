//! TLS 1.2 handshake messages — stage 4b-iii.
//!
//! The messages a TLS 1.2 ECDHE handshake uses that TLS 1.3 does not have, or
//! has in a different shape. The hello messages are shared with
//! [`super::handshake`]: a TLS 1.2 `ClientHello` is the same structure, and
//! [`ServerHello12`] differs only in tolerating an absent extensions block.
//!
//! Every message parses and encodes, and the two must be inverses on the wire
//! bytes for the same reason as in [`super::handshake`]: the transcript covers
//! what arrived, and `Finished` is a MAC over it.
//!
//! # What differs from TLS 1.3
//!
//! | | TLS 1.2 | TLS 1.3 |
//! | --- | --- | --- |
//! | `Certificate` | bare list of DER certificates | request context, and extensions per entry |
//! | Key exchange | `ServerKeyExchange` carries the server's ephemeral key **and signs it** | `key_share` in the hellos; the signature is a separate `CertificateVerify` |
//! | Server flight end | an explicit, empty `ServerHelloDone` | none |
//! | `Finished` | 12 octets | the hash length |
//!
//! The `ServerKeyExchange` signature is the TLS 1.2 equivalent of
//! `CertificateVerify`, and what it covers is the thing to get right: the two
//! hello randoms as well as the ephemeral key. Signing the key alone would let
//! an attacker replay a captured `ServerKeyExchange` into a new handshake.
//!
//! # Scope
//!
//! ECDHE only, with `named_curve` parameters. Static RSA, finite-field DHE and
//! explicit curve parameters are refused ([`HandshakeError::UnexpectedCurveType`]
//! for the last) rather than parsed.

use super::handshake::{
    parse_extensions, write_extensions, Extension, HandshakeError, HandshakeType, Message,
};
use super::wire::{Reader, Writer};

type Result<T> = core::result::Result<T, HandshakeError>;

/// `ServerHello.server_version` for TLS 1.2.
pub const TLS12: u16 = 0x0303;

/// Length of a TLS 1.2 `Finished`'s `verify_data`.
pub const VERIFY_DATA_LEN: usize = 12;

/// `ECCurveType.named_curve` (RFC 8422 §5.4).
const NAMED_CURVE: u8 = 3;

/// A ServerHello, RFC 5246 §7.4.1.3.
///
/// Unlike [`super::handshake::ServerHello`] this accepts a message with no
/// extensions block at all, which is how servers that predate extensions answer.
/// Accepting it here is what lets the client report *why* such a server is
/// refused (it cannot offer the extended master secret) instead of reporting
/// a truncated message.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct ServerHello12<'a> {
    /// `server_version`.
    pub version: u16,
    /// `random`, 32 octets.
    pub random: &'a [u8],
    /// `session_id`, at most 32 octets. In TLS 1.2 this is a resumption
    /// identifier, not an echo.
    pub session_id: &'a [u8],
    /// The selected cipher suite.
    pub cipher_suite: u16,
    /// `compression_method`.
    pub compression: u8,
    /// The extensions, in order. Empty if the block was absent.
    pub extensions: Vec<Extension<'a>>,
}

impl<'a> ServerHello12<'a> {
    /// Parse a ServerHello body. The version is reported, not checked: a server
    /// that selects something older is a protocol-version error for the caller
    /// to name, not a parse error.
    pub fn parse(body: &'a [u8]) -> Result<Self> {
        let mut reader = Reader::new(body);
        let version = reader.u16()?;
        let random = reader.take(32)?;
        let session_id = reader.vector_u8()?;
        if session_id.len() > 32 {
            return Err(HandshakeError::Malformed("session_id is over 32 octets"));
        }
        let cipher_suite = reader.u16()?;
        let compression = reader.u8()?;
        let extensions = if reader.is_empty() {
            Vec::new()
        } else {
            parse_extensions(&mut reader)?
        };
        reader.finish()?;
        Ok(Self {
            version,
            random,
            session_id,
            cipher_suite,
            compression,
            extensions,
        })
    }

    /// Encode the body. The extensions block is omitted when there are none,
    /// matching what [`ServerHello12::parse`] accepts.
    pub fn encode(&self) -> Vec<u8> {
        let mut writer = Writer::new();
        writer.u16(self.version);
        writer.bytes(self.random);
        writer.vector_u8(|w| w.bytes(self.session_id));
        writer.u16(self.cipher_suite);
        writer.u8(self.compression);
        if !self.extensions.is_empty() {
            write_extensions(&mut writer, &self.extensions);
        }
        writer.into_vec()
    }
}

/// A `Certificate` message, RFC 5246 §7.4.2: a list of DER certificates,
/// end-entity first.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Certificate12<'a> {
    /// The certificates, each a whole DER encoding.
    pub certificates: Vec<&'a [u8]>,
}

impl<'a> Certificate12<'a> {
    /// Parse a body. An empty list is valid on the wire (a client with nothing
    /// to present sends one), so the caller decides whether it is acceptable.
    pub fn parse(body: &'a [u8]) -> Result<Self> {
        let mut reader = Reader::new(body);
        let mut list = reader.sub_u24()?;
        reader.finish()?;

        let mut certificates = Vec::new();
        while !list.is_empty() {
            let cert = list.vector_u24()?;
            if cert.is_empty() {
                return Err(HandshakeError::Malformed(
                    "a certificate in the chain is empty",
                ));
            }
            certificates.push(cert);
        }
        Ok(Self { certificates })
    }

    /// Encode a body from DER certificates.
    pub fn encode(certificates: &[&[u8]]) -> Vec<u8> {
        let mut writer = Writer::new();
        writer.vector_u24(|w| {
            for cert in certificates {
                w.vector_u24(|w| w.bytes(cert));
            }
        });
        writer.into_vec()
    }
}

/// An ECDHE `ServerKeyExchange`, RFC 8422 §5.4.
///
/// ```text
/// struct {
///     ServerECDHParams params;        // curve_type(1) namedcurve(2) point<1..255>
///     SignatureAndHashAlgorithm alg;  // 2 octets
///     opaque signature<0..2^16-1>;
/// } ServerKeyExchange;
/// ```
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct ServerKeyExchange<'a> {
    /// `NamedCurve`, as a TLS `NamedGroup` value.
    pub named_curve: u16,
    /// The server's ephemeral public key, as an uncompressed point (or the raw
    /// 32 octets for X25519).
    pub public: &'a [u8],
    /// The `SignatureAndHashAlgorithm`, as the TLS `SignatureScheme` value.
    pub scheme: u16,
    /// The signature.
    pub signature: &'a [u8],
    /// `ServerECDHParams` exactly as encoded, which is the part of the message
    /// that is signed. Kept as a borrow of the input for the reason the
    /// transcript is: signing a re-encoding is signing something that was not
    /// sent.
    pub params: &'a [u8],
}

impl<'a> ServerKeyExchange<'a> {
    /// Parse a body, refusing anything but `named_curve` parameters.
    pub fn parse(body: &'a [u8]) -> Result<Self> {
        let mut reader = Reader::new(body);
        let curve_type = reader.u8()?;
        if curve_type != NAMED_CURVE {
            return Err(HandshakeError::UnexpectedCurveType(curve_type));
        }
        let named_curve = reader.u16()?;
        let public = reader.vector_u8()?;
        if public.is_empty() {
            return Err(HandshakeError::Empty("ServerKeyExchange public key"));
        }
        let params = &body[..body.len() - reader.remaining()];
        let scheme = reader.u16()?;
        let signature = reader.vector_u16()?;
        if signature.is_empty() {
            return Err(HandshakeError::Empty("ServerKeyExchange signature"));
        }
        reader.finish()?;
        Ok(Self {
            named_curve,
            public,
            scheme,
            signature,
            params,
        })
    }

    /// The `ServerECDHParams` encoding for a curve and public key: the bytes a
    /// server signs, and the first part of the message it sends.
    pub fn encode_params(named_curve: u16, public: &[u8]) -> Vec<u8> {
        let mut writer = Writer::new();
        writer.u8(NAMED_CURVE);
        writer.u16(named_curve);
        writer.vector_u8(|w| w.bytes(public));
        writer.into_vec()
    }

    /// What the signature covers (RFC 5246 §7.4.3):
    /// `client_random + server_random + ServerECDHParams`.
    ///
    /// Both randoms, so a captured message cannot be replayed into another
    /// handshake.
    pub fn signed_content(&self, client_random: &[u8], server_random: &[u8]) -> Vec<u8> {
        signed_content(client_random, server_random, self.params)
    }

    /// Encode a body from already-encoded parameters.
    pub fn encode(params: &[u8], scheme: u16, signature: &[u8]) -> Vec<u8> {
        let mut writer = Writer::new();
        writer.bytes(params);
        writer.u16(scheme);
        writer.vector_u16(|w| w.bytes(signature));
        writer.into_vec()
    }
}

/// `client_random + server_random + ServerECDHParams`.
pub fn signed_content(client_random: &[u8], server_random: &[u8], params: &[u8]) -> Vec<u8> {
    let mut out = Vec::with_capacity(client_random.len() + server_random.len() + params.len());
    out.extend_from_slice(client_random);
    out.extend_from_slice(server_random);
    out.extend_from_slice(params);
    out
}

/// Encode an ECDHE `ClientKeyExchange` body: the client's public key as a
/// `opaque<1..255>`.
pub fn encode_client_key_exchange(public: &[u8]) -> Vec<u8> {
    let mut writer = Writer::new();
    writer.vector_u8(|w| w.bytes(public));
    writer.into_vec()
}

/// Parse an ECDHE `ClientKeyExchange` body.
pub fn parse_client_key_exchange(body: &[u8]) -> Result<&[u8]> {
    let mut reader = Reader::new(body);
    let public = reader.vector_u8()?;
    if public.is_empty() {
        return Err(HandshakeError::Empty("ClientKeyExchange public key"));
    }
    reader.finish()?;
    Ok(public)
}

/// A TLS 1.2 `CertificateRequest`, RFC 5246 §7.4.4.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct CertificateRequest12<'a> {
    /// `certificate_types`.
    pub certificate_types: &'a [u8],
    /// `supported_signature_algorithms`, as `SignatureScheme` values.
    pub signature_algorithms: Vec<u16>,
    /// `certificate_authorities`, the DER-encoded names the server accepts.
    pub authorities: &'a [u8],
}

impl<'a> CertificateRequest12<'a> {
    /// Parse a body.
    pub fn parse(body: &'a [u8]) -> Result<Self> {
        let mut reader = Reader::new(body);
        let certificate_types = reader.vector_u8()?;
        if certificate_types.is_empty() {
            return Err(HandshakeError::Empty("certificate_types"));
        }
        let mut algorithms = reader.sub_u16()?;
        if algorithms.is_empty() {
            return Err(HandshakeError::MissingSignatureAlgorithms);
        }
        let mut signature_algorithms = Vec::new();
        while !algorithms.is_empty() {
            signature_algorithms.push(algorithms.u16()?);
        }
        let authorities = reader.vector_u16()?;
        reader.finish()?;
        Ok(Self {
            certificate_types,
            signature_algorithms,
            authorities,
        })
    }

    /// Encode a body.
    pub fn encode(&self) -> Vec<u8> {
        let mut writer = Writer::new();
        writer.vector_u8(|w| w.bytes(self.certificate_types));
        writer.vector_u16(|w| {
            for scheme in &self.signature_algorithms {
                w.u16(*scheme);
            }
        });
        writer.vector_u16(|w| w.bytes(self.authorities));
        writer.into_vec()
    }
}

/// Check a `ServerHelloDone` body, which must be empty.
pub fn parse_server_hello_done(body: &[u8]) -> Result<()> {
    if body.is_empty() {
        Ok(())
    } else {
        Err(HandshakeError::Malformed("ServerHelloDone has a body"))
    }
}

/// Parse a `Finished` body and check it is [`VERIFY_DATA_LEN`] octets.
pub fn parse_finished(body: &[u8]) -> Result<&[u8]> {
    if body.len() == VERIFY_DATA_LEN {
        Ok(body)
    } else {
        Err(HandshakeError::Malformed("Finished is not 12 octets"))
    }
}

/// Encode a complete handshake message, header included.
pub fn message(typ: HandshakeType, body: &[u8]) -> Vec<u8> {
    Message::encode(typ, body)
}
