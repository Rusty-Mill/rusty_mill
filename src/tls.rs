//! Optional TLS + CredSSP connector for the enhanced-security path (feature
//! `tls`).
//!
//! This is the crate's one concession to a third-party dependency, and it is
//! entirely opt-in: nothing here is compiled unless the `tls` feature is
//! enabled, so the default build stays dependency-free. A TLS stack is the one
//! piece that cannot be hand-rolled responsibly, so [`connect_tls`] wraps
//! [`rustls`] to upgrade the TCP stream before handing off to
//! [`RdpTransport::establish_enhanced`]. When the server selects CredSSP/NLA
//! (`HYBRID`), it runs the [`crate::credssp`] exchange over the TLS channel
//! first, delegating the user's credentials. The CredSSP crypto (NTLMv2) is
//! all in the dependency-free core; only the TLS bytes need the crate.
//!
//! The RDP-over-TLS *protocol* logic still lives in the dependency-free core
//! ([`crate::net`]) — this module only supplies the bytes-on-wire TLS. If you
//! would rather not take the rustls dependency, wrap your `TcpStream` in any
//! TLS implementation yourself and call [`RdpTransport::new_enhanced`] +
//! [`RdpTransport::establish_enhanced`] directly.
//!
//! ## Certificate verification
//!
//! RDP servers overwhelmingly present self-signed certificates and rely on
//! out-of-band trust, so [`connect_tls`] does **not** verify the server
//! certificate. That means it does not protect against an active
//! man-in-the-middle. If you need verification, build your own [`rustls`]
//! stream with a real verifier and use the [`RdpTransport::new_enhanced`] path.

use std::fs::File;
use std::io::{self, Read, Write};
use std::net::TcpStream;
use std::sync::Arc;
use std::time::{SystemTime, UNIX_EPOCH};

use rustls::client::danger::{HandshakeSignatureValid, ServerCertVerified, ServerCertVerifier};
use rustls::pki_types::{CertificateDer, ServerName, UnixTime};
use rustls::{ClientConfig, ClientConnection, DigitallySignedStruct, SignatureScheme, StreamOwned};

use crate::credssp::{CredSspClient, KerberosCredSspClient};
use crate::krb5::aes::AesKey;
use crate::nego::SecurityProtocols;
use crate::net::{EstablishConfig, RdpSession, RdpTransport};

/// The TLS-wrapped stream type an established enhanced-security session runs
/// over.
pub type TlsStream = StreamOwned<ClientConnection, TcpStream>;

/// Connect to an RDP server over TLS and drive the enhanced-security bring-up.
///
/// Opens a TCP connection to `addr` (a `host:port` string), performs the
/// X.224 negotiation requesting `requested`, upgrades the stream to TLS, runs
/// the CredSSP/NLA exchange if the server selected `HYBRID`, and finishes with
/// [`RdpTransport::establish_enhanced`]. Returns the live transport and its
/// [`RdpSession`].
///
/// Pass `requested` = `SSL | HYBRID` to allow either; a server that falls back
/// to standard RDP security returns an error pointing at
/// [`RdpTransport::establish`].
///
/// The server certificate is **not** verified — see the [module
/// docs](self#certificate-verification).
pub fn connect_tls(
    addr: &str,
    config: &EstablishConfig,
    requested: SecurityProtocols,
) -> io::Result<(RdpTransport<TlsStream>, RdpSession)> {
    // Host portion of `host:port`, for the TLS server name.
    let host = addr.rsplit_once(':').map(|(h, _)| h).unwrap_or(addr);

    // 1. X.224 negotiation happens on the raw TCP connection, before TLS.
    let tcp = TcpStream::connect(addr)?;
    let mut pre = RdpTransport::new(tcp);
    let selected = pre.negotiate(requested, Some(&config.username))?;
    let use_nla = match selected {
        s if s == SecurityProtocols::SSL => false,
        s if s == SecurityProtocols::HYBRID || s == SecurityProtocols::HYBRID_EX => true,
        s if s == SecurityProtocols::RDP => {
            return Err(io::Error::new(
                io::ErrorKind::Unsupported,
                "server selected standard RDP security; use RdpTransport::establish instead",
            ));
        }
        other => {
            return Err(io::Error::new(
                io::ErrorKind::Unsupported,
                format!("server selected unsupported protocol {other:?}"),
            ));
        }
    };
    let tcp = pre.into_inner();

    // 2. Upgrade the same TCP connection to TLS.
    let tls_config = ClientConfig::builder()
        .dangerous()
        .with_custom_certificate_verifier(Arc::new(AcceptAnyServerCert))
        .with_no_client_auth();
    let server_name = ServerName::try_from(host.to_string()).map_err(|e| {
        io::Error::new(io::ErrorKind::InvalidInput, format!("bad server name: {e}"))
    })?;
    let conn = ClientConnection::new(Arc::new(tls_config), server_name)
        .map_err(|e| io::Error::new(io::ErrorKind::Other, format!("TLS setup failed: {e}")))?;
    let mut tls = StreamOwned::new(conn, tcp);

    // 3. If the server chose CredSSP, authenticate before the RDP sequence.
    if use_nla {
        run_credssp(&mut tls, config)?;
    }

    // 4. Everything from MCS onward runs inside the TLS tunnel.
    let mut transport = RdpTransport::new_enhanced(tls);
    let session = transport.establish_enhanced(config, selected)?;
    Ok((transport, session))
}

/// Run the CredSSP/NLA exchange over an established TLS stream, delegating the
/// user's credentials so the RDP connection sequence can proceed.
fn run_credssp(tls: &mut TlsStream, config: &EstablishConfig) -> io::Result<()> {
    // Complete the TLS handshake so the server certificate is available.
    while tls.conn.is_handshaking() {
        if tls.conn.complete_io(&mut tls.sock)?.0 == 0 && tls.conn.is_handshaking() {
            return Err(io::Error::new(
                io::ErrorKind::UnexpectedEof,
                "TLS handshake stalled",
            ));
        }
    }
    let public_key = server_public_key(tls)?;

    // Gather the nondeterministic inputs from the OS.
    let mut seed = [0u8; 56];
    File::open("/dev/urandom")?.read_exact(&mut seed)?;
    let mut nonce = [0u8; 32];
    nonce.copy_from_slice(&seed[..32]);
    let mut client_challenge = [0u8; 8];
    client_challenge.copy_from_slice(&seed[32..40]);
    let mut exported_session_key = [0u8; 16];
    exported_session_key.copy_from_slice(&seed[40..56]);

    let mut client = CredSspClient::new(
        &config.domain,
        &config.username,
        &config.password,
        &config.client_name,
        public_key,
        nonce,
        client_challenge,
        filetime_now(),
        exported_session_key,
    );

    // Leg 1: NTLM NEGOTIATE.
    tls.write_all(&client.negotiate_request())?;
    tls.flush()?;
    // Leg 2 → Leg 3: CHALLENGE in, AUTHENTICATE + pubKeyAuth out.
    let leg2 = read_ts_request(tls)?;
    let leg3 = client.challenge_response(&leg2).map_err(to_io)?;
    tls.write_all(&leg3)?;
    tls.flush()?;
    // Leg 4 → Leg 5: server public-key confirmation in, credentials out.
    let leg4 = read_ts_request(tls)?;
    let leg5 = client.finish(&leg4).map_err(to_io)?;
    tls.write_all(&leg5)?;
    tls.flush()?;
    Ok(())
}

/// Connect to an RDP server over TLS using a Kerberos ticket for NLA.
///
/// Same as [`connect_tls`] but authenticates with Kerberos instead of NTLM:
/// negotiates `HYBRID`, upgrades to TLS, runs the CredSSP exchange with the
/// caller-provided `AP-REQ` and its `session_key` (obtain these from a KDC or
/// a credential cache — fetching them is out of scope), and finishes with
/// [`RdpTransport::establish_enhanced`].
///
/// The server certificate is **not** verified — see the [module
/// docs](self#certificate-verification).
pub fn connect_tls_kerberos(
    addr: &str,
    config: &EstablishConfig,
    ap_req: Vec<u8>,
    session_key: AesKey,
) -> io::Result<(RdpTransport<TlsStream>, RdpSession)> {
    let host = addr.rsplit_once(':').map(|(h, _)| h).unwrap_or(addr);

    // Negotiate HYBRID on the raw TCP connection.
    let tcp = TcpStream::connect(addr)?;
    let mut pre = RdpTransport::new(tcp);
    let selected = pre.negotiate(SecurityProtocols::HYBRID, Some(&config.username))?;
    if selected != SecurityProtocols::HYBRID && selected != SecurityProtocols::HYBRID_EX {
        return Err(io::Error::new(
            io::ErrorKind::Unsupported,
            format!("server did not select CredSSP/HYBRID (got {selected:?})"),
        ));
    }
    let tcp = pre.into_inner();

    let tls_config = ClientConfig::builder()
        .dangerous()
        .with_custom_certificate_verifier(Arc::new(AcceptAnyServerCert))
        .with_no_client_auth();
    let server_name = ServerName::try_from(host.to_string()).map_err(|e| {
        io::Error::new(io::ErrorKind::InvalidInput, format!("bad server name: {e}"))
    })?;
    let conn = ClientConnection::new(Arc::new(tls_config), server_name)
        .map_err(|e| io::Error::new(io::ErrorKind::Other, format!("TLS setup failed: {e}")))?;
    let mut tls = StreamOwned::new(conn, tcp);

    run_credssp_kerberos(&mut tls, config, ap_req, session_key)?;

    let mut transport = RdpTransport::new_enhanced(tls);
    let session = transport.establish_enhanced(config, selected)?;
    Ok((transport, session))
}

/// Run the Kerberos CredSSP exchange over an established TLS stream.
fn run_credssp_kerberos(
    tls: &mut TlsStream,
    config: &EstablishConfig,
    ap_req: Vec<u8>,
    session_key: AesKey,
) -> io::Result<()> {
    while tls.conn.is_handshaking() {
        if tls.conn.complete_io(&mut tls.sock)?.0 == 0 && tls.conn.is_handshaking() {
            return Err(io::Error::new(
                io::ErrorKind::UnexpectedEof,
                "TLS handshake stalled",
            ));
        }
    }
    let public_key = server_public_key(tls)?;

    // Nonce plus two 16-byte GSS confounders.
    let mut seed = [0u8; 64];
    File::open("/dev/urandom")?.read_exact(&mut seed)?;
    let mut nonce = [0u8; 32];
    nonce.copy_from_slice(&seed[..32]);
    let conf1 = &seed[32..48];
    let conf2 = &seed[48..64];

    let mut client = KerberosCredSspClient::new(
        session_key,
        ap_req,
        public_key,
        nonce,
        &config.domain,
        &config.username,
        &config.password,
    );

    // Leg 1: SPNEGO/AP-REQ + sealed public key.
    tls.write_all(&client.initial_request(conf1))?;
    tls.flush()?;
    // Leg 2 → Leg 3: AP-REP + server public key in, sealed credentials out.
    let leg2 = read_ts_request(tls)?;
    let leg3 = client.finish(&leg2, conf2).map_err(to_io)?;
    tls.write_all(&leg3)?;
    tls.flush()?;
    Ok(())
}

/// The Windows FILETIME (100 ns since 1601-01-01) for the current time.
fn filetime_now() -> [u8; 8] {
    const EPOCH_DIFF_SECS: u64 = 11_644_473_600;
    let unix = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .unwrap_or_default();
    let ticks = (unix.as_secs() + EPOCH_DIFF_SECS)
        .wrapping_mul(10_000_000)
        .wrapping_add(unix.subsec_nanos() as u64 / 100);
    ticks.to_le_bytes()
}

/// Read one complete DER `TSRequest` (a definite-length SEQUENCE) from the
/// stream.
fn read_ts_request<S: Read>(stream: &mut S) -> io::Result<Vec<u8>> {
    let mut header = [0u8; 2];
    stream.read_exact(&mut header)?;
    if header[0] != 0x30 {
        return Err(io::Error::new(
            io::ErrorKind::InvalidData,
            "expected TSRequest SEQUENCE",
        ));
    }
    let mut out = vec![header[0], header[1]];
    let content_len = if header[1] & 0x80 == 0 {
        header[1] as usize
    } else {
        let n = (header[1] & 0x7F) as usize;
        if n == 0 || n > 4 {
            return Err(io::Error::new(io::ErrorKind::InvalidData, "bad DER length"));
        }
        let mut lenbuf = vec![0u8; n];
        stream.read_exact(&mut lenbuf)?;
        out.extend_from_slice(&lenbuf);
        lenbuf
            .iter()
            .fold(0usize, |acc, &b| (acc << 8) | b as usize)
    };
    let start = out.len();
    out.resize(start + content_len, 0);
    stream.read_exact(&mut out[start..])?;
    Ok(out)
}

/// Extract the server's `SubjectPublicKeyInfo` DER from the TLS peer
/// certificate, for the CredSSP channel binding.
fn server_public_key(tls: &TlsStream) -> io::Result<Vec<u8>> {
    let certs = tls
        .conn
        .peer_certificates()
        .ok_or_else(|| io::Error::new(io::ErrorKind::Other, "no server certificate presented"))?;
    let cert = certs
        .first()
        .ok_or_else(|| io::Error::new(io::ErrorKind::Other, "empty server certificate chain"))?;
    extract_spki(cert.as_ref())
        .ok_or_else(|| io::Error::new(io::ErrorKind::InvalidData, "cannot parse certificate"))
}

/// Split one DER TLV (single-byte tag), returning `(whole, content, rest)`.
fn split_tlv(buf: &[u8]) -> Option<(&[u8], &[u8], &[u8])> {
    if buf.len() < 2 {
        return None;
    }
    let first = buf[1];
    let (len, header) = if first & 0x80 == 0 {
        (first as usize, 2)
    } else {
        let n = (first & 0x7F) as usize;
        if n == 0 || n > 4 || buf.len() < 2 + n {
            return None;
        }
        let len = buf[2..2 + n]
            .iter()
            .fold(0usize, |acc, &b| (acc << 8) | b as usize);
        (len, 2 + n)
    };
    let end = header.checked_add(len)?;
    if buf.len() < end {
        return None;
    }
    Some((&buf[..end], &buf[header..end], &buf[end..]))
}

/// Extract the `SubjectPublicKeyInfo` from an X.509 certificate DER.
///
/// Walks `Certificate → tbsCertificate`, skips the optional version, serial
/// number, and the four SEQUENCEs (signature, issuer, validity, subject), and
/// returns the following SEQUENCE (the SPKI) verbatim.
fn extract_spki(cert: &[u8]) -> Option<Vec<u8>> {
    let (_, cert_content, _) = split_tlv(cert)?; // Certificate
    let (_, tbs, _) = split_tlv(cert_content)?; // tbsCertificate
    let mut cur = tbs;
    if cur.first() == Some(&0xA0) {
        // [0] EXPLICIT version
        let (_, _, rest) = split_tlv(cur)?;
        cur = rest;
    }
    // serialNumber INTEGER
    let (_, _, rest) = split_tlv(cur)?;
    cur = rest;
    // signature, issuer, validity, subject — four SEQUENCEs.
    for _ in 0..4 {
        let (_, _, rest) = split_tlv(cur)?;
        cur = rest;
    }
    let (spki, _, _) = split_tlv(cur)?;
    Some(spki.to_vec())
}

/// Map a codec [`crate::Error`] into an [`io::Error`].
fn to_io(e: crate::Error) -> io::Error {
    io::Error::new(io::ErrorKind::InvalidData, e.to_string())
}

/// A certificate verifier that accepts any server certificate.
///
/// Appropriate for RDP's typical self-signed-certificate deployments where
/// trust is established out of band; it provides no protection against an
/// active man-in-the-middle.
#[derive(Debug)]
struct AcceptAnyServerCert;

impl ServerCertVerifier for AcceptAnyServerCert {
    fn verify_server_cert(
        &self,
        _end_entity: &CertificateDer<'_>,
        _intermediates: &[CertificateDer<'_>],
        _server_name: &ServerName<'_>,
        _ocsp_response: &[u8],
        _now: UnixTime,
    ) -> Result<ServerCertVerified, rustls::Error> {
        Ok(ServerCertVerified::assertion())
    }

    fn verify_tls12_signature(
        &self,
        _message: &[u8],
        _cert: &CertificateDer<'_>,
        _dss: &DigitallySignedStruct,
    ) -> Result<HandshakeSignatureValid, rustls::Error> {
        Ok(HandshakeSignatureValid::assertion())
    }

    fn verify_tls13_signature(
        &self,
        _message: &[u8],
        _cert: &CertificateDer<'_>,
        _dss: &DigitallySignedStruct,
    ) -> Result<HandshakeSignatureValid, rustls::Error> {
        Ok(HandshakeSignatureValid::assertion())
    }

    fn supported_verify_schemes(&self) -> Vec<SignatureScheme> {
        vec![
            SignatureScheme::RSA_PKCS1_SHA1,
            SignatureScheme::ECDSA_SHA1_Legacy,
            SignatureScheme::RSA_PKCS1_SHA256,
            SignatureScheme::ECDSA_NISTP256_SHA256,
            SignatureScheme::RSA_PKCS1_SHA384,
            SignatureScheme::ECDSA_NISTP384_SHA384,
            SignatureScheme::RSA_PKCS1_SHA512,
            SignatureScheme::ECDSA_NISTP521_SHA512,
            SignatureScheme::RSA_PSS_SHA256,
            SignatureScheme::RSA_PSS_SHA384,
            SignatureScheme::RSA_PSS_SHA512,
            SignatureScheme::ED25519,
            SignatureScheme::ED448,
        ]
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::credssp::TsRequest;

    /// Build a DER TLV with a single-byte tag and short-form length.
    fn tlv(tag: u8, content: &[u8]) -> Vec<u8> {
        let mut v = vec![tag, content.len() as u8];
        v.extend_from_slice(content);
        v
    }

    #[test]
    fn extract_spki_walks_certificate() {
        // Minimal Certificate { tbsCertificate { [0]ver, serial, sig, issuer,
        // validity, subject, spki }, sigAlg, sigValue }.
        let version = tlv(0xA0, &tlv(0x02, &[0x02])); // [0] { INTEGER 2 }
        let serial = tlv(0x02, &[0x01]);
        let seq_empty = tlv(0x30, &[]);
        let spki = tlv(0x30, &[0xDE, 0xAD, 0xBE, 0xEF]);

        let mut tbs = Vec::new();
        tbs.extend_from_slice(&version);
        tbs.extend_from_slice(&serial);
        for _ in 0..4 {
            tbs.extend_from_slice(&seq_empty); // sig, issuer, validity, subject
        }
        tbs.extend_from_slice(&spki);

        let mut cert_content = tlv(0x30, &tbs);
        cert_content.extend_from_slice(&seq_empty); // signatureAlgorithm
        cert_content.extend_from_slice(&tlv(0x03, &[0x00])); // signatureValue
        let cert = tlv(0x30, &cert_content);

        assert_eq!(extract_spki(&cert), Some(spki));
    }

    #[test]
    fn read_ts_request_short_and_long_form() {
        // Short form (< 128 bytes of content).
        let small = TsRequest {
            version: 6,
            nego_tokens: vec![vec![1, 2, 3]],
            ..Default::default()
        }
        .to_vec();
        let mut cursor = io::Cursor::new(small.clone());
        assert_eq!(read_ts_request(&mut cursor).unwrap(), small);

        // Long form (a token large enough to force a multi-byte length).
        let big = TsRequest {
            version: 6,
            nego_tokens: vec![vec![0xAB; 300]],
            ..Default::default()
        }
        .to_vec();
        assert!(big[1] & 0x80 != 0, "expected long-form length");
        let mut cursor = io::Cursor::new(big.clone());
        assert_eq!(read_ts_request(&mut cursor).unwrap(), big);
    }
}
