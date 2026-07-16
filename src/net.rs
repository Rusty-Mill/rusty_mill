//! Blocking TCP driver — the I/O boundary around the pure codec.
//!
//! Every other module in this crate is I/O-free: it turns bytes into types
//! and back. This module is the one place that touches a socket. It wraps any
//! [`Read`] + [`Write`] (a [`std::net::TcpStream`] in practice) and drives the
//! deterministic part of the RDP connection sequence:
//!
//! 1. [`RdpTransport::negotiate`] — the X.224 security negotiation.
//! 2. [`RdpTransport::mcs_connect`] — the GCC/MCS `Connect-Initial` /
//!    `Connect-Response` exchange.
//! 3. [`RdpTransport::erect_domain`], [`RdpTransport::attach_user`],
//!    [`RdpTransport::join_channel`] — MCS channel setup.
//! 4. [`RdpTransport::security_exchange`] — RSA-encrypt the client random for
//!    standard RDP security, then [`RdpTransport::send_client_info`] and
//!    [`RdpTransport::send_secure`] / [`RdpTransport::recv_secure`] carry the
//!    encrypted, MAC'd PDUs.
//! 5. [`RdpTransport::send_data`] / [`RdpTransport::recv_data`] — raw
//!    I/O-channel traffic.
//!
//! [`server_crypto`] pulls the server's RSA key and random out of the MCS
//! Connect-Response so the caller can derive session keys
//! ([`crate::security::derive_session_keys`]) and build an
//! [`Rc4Session`]. Everything here stays on the standard library, so the crate
//! remains dependency-free. The capability exchange and connection
//! finalization are built with the [`crate::capabilities`] and
//! [`crate::finalization`] modules and sent with [`RdpTransport::send_secure`];
//! chaining them end to end against a live server is the remaining step.

use std::io::{self, Read, Write};

use crate::client_info::{ClientInfo, INFO_UNICODE};
use crate::gcc::{self, UserDataBlock};
use crate::mcs::{ConnectInitial, ConnectResponse, DomainPdu, McsResult};
use crate::nego::{Negotiation, SecurityProtocols};
use crate::security::{self, Rc4Session, RsaPublicKey, SEC_INFO_PKT};
use crate::tpkt::{Tpkt, TPKT_HEADER_LEN};
use crate::x224::{ConnectionPdu, Cookie, X224};

/// The server's cryptographic parameters, extracted from `TS_UD_SC_SEC1`.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ServerCrypto {
    /// The negotiated `encryptionMethod`.
    pub encryption_method: u32,
    /// The server random used in key derivation.
    pub server_random: Vec<u8>,
    /// The server's RSA public key.
    pub public_key: RsaPublicKey,
}

/// Extract the server's crypto parameters from the MCS Connect-Response
/// settings blocks.
///
/// Returns `Ok(None)` when the server selected no encryption (or sent no
/// `SC_SECURITY` block); an error if the certificate cannot be parsed.
pub fn server_crypto(blocks: &[UserDataBlock]) -> io::Result<Option<ServerCrypto>> {
    for block in blocks {
        if let UserDataBlock::ServerSecurity(sec) = block {
            if sec.encryption_method == 0 || sec.server_certificate.is_empty() {
                return Ok(None);
            }
            let public_key =
                security::parse_server_certificate(&sec.server_certificate).map_err(to_io)?;
            return Ok(Some(ServerCrypto {
                encryption_method: sec.encryption_method,
                server_random: sec.server_random.clone(),
                public_key,
            }));
        }
    }
    Ok(None)
}

/// Map a codec [`crate::Error`] into an [`io::Error`] for the transport layer.
fn to_io(e: crate::Error) -> io::Error {
    io::Error::new(io::ErrorKind::InvalidData, e.to_string())
}

fn protocol_error(msg: impl Into<String>) -> io::Error {
    io::Error::new(io::ErrorKind::InvalidData, msg.into())
}

/// A TPKT-framed RDP transport over a byte stream.
pub struct RdpTransport<S> {
    stream: S,
}

impl<S: Read + Write> RdpTransport<S> {
    /// Wrap a connected stream.
    pub fn new(stream: S) -> Self {
        RdpTransport { stream }
    }

    /// Consume the transport and return the underlying stream.
    pub fn into_inner(self) -> S {
        self.stream
    }

    /// Borrow the underlying stream.
    pub fn get_ref(&self) -> &S {
        &self.stream
    }

    // --- TPKT framing -----------------------------------------------------

    /// Write `payload` as a single TPKT packet.
    pub fn write_tpkt(&mut self, payload: &[u8]) -> io::Result<()> {
        let packet = Tpkt::new(payload).to_vec().map_err(to_io)?;
        self.stream.write_all(&packet)?;
        self.stream.flush()
    }

    /// Read one complete TPKT packet, returning its payload (the X.224 TPDU).
    pub fn read_tpkt(&mut self) -> io::Result<Vec<u8>> {
        let mut header = [0u8; TPKT_HEADER_LEN];
        self.stream.read_exact(&mut header)?;
        let total = Tpkt::peek_total_len(&header)
            .map_err(to_io)?
            .ok_or_else(|| protocol_error("short TPKT header"))?;
        let mut packet = vec![0u8; total];
        packet[..TPKT_HEADER_LEN].copy_from_slice(&header);
        self.stream.read_exact(&mut packet[TPKT_HEADER_LEN..])?;
        let tpkt = Tpkt::decode(&packet).map_err(to_io)?;
        Ok(tpkt.payload.to_vec())
    }

    /// Wrap `payload` in an X.224 Data TPDU and send it.
    pub fn write_x224_data(&mut self, payload: &[u8]) -> io::Result<()> {
        let tpdu = X224::data(payload).to_vec().map_err(to_io)?;
        self.write_tpkt(&tpdu)
    }

    /// Read a TPKT packet and return the inner X.224 Data TPDU payload.
    pub fn read_x224_data(&mut self) -> io::Result<Vec<u8>> {
        let tpdu = self.read_tpkt()?;
        match X224::decode(&tpdu).map_err(to_io)? {
            X224::Data(payload) => Ok(payload.to_vec()),
            other => Err(protocol_error(format!("expected Data TPDU, got {other:?}"))),
        }
    }

    // --- Connection sequence ---------------------------------------------

    /// Perform the X.224 security negotiation and return the server's
    /// selected protocol.
    ///
    /// Sends a Connection Request advertising `requested` (with an optional
    /// `mstshash` cookie) and interprets the Connection Confirm. A server that
    /// omits the negotiation response is treated as selecting standard RDP
    /// security; a negotiation failure becomes an error.
    pub fn negotiate(
        &mut self,
        requested: SecurityProtocols,
        cookie: Option<&str>,
    ) -> io::Result<SecurityProtocols> {
        let request = ConnectionPdu {
            cookie: cookie.map(|c| Cookie::MsTsHash(c.to_string())),
            negotiation: Some(Negotiation::Request {
                flags: 0,
                protocols: requested,
            }),
            ..Default::default()
        };
        let cr = X224::ConnectionRequest(request).to_vec().map_err(to_io)?;
        self.write_tpkt(&cr)?;

        let response = self.read_tpkt()?;
        match X224::decode(&response).map_err(to_io)? {
            X224::ConnectionConfirm(pdu) => match pdu.negotiation {
                Some(Negotiation::Response { selected, .. }) => Ok(selected),
                Some(Negotiation::Failure { code }) => {
                    Err(protocol_error(format!("negotiation failed: {code:?}")))
                }
                _ => Ok(SecurityProtocols::RDP),
            },
            other => Err(protocol_error(format!(
                "expected Connection Confirm, got {other:?}"
            ))),
        }
    }

    /// Perform the GCC/MCS `Connect-Initial` / `Connect-Response` exchange.
    ///
    /// Wraps `client_blocks` in a Conference Create Request and returns the
    /// server's settings blocks parsed from the Conference Create Response.
    pub fn mcs_connect(
        &mut self,
        client_blocks: &[UserDataBlock],
    ) -> io::Result<Vec<UserDataBlock>> {
        let user_data = gcc::encode_user_data(client_blocks).map_err(to_io)?;
        let ccr = gcc::encode_conference_create_request(&user_data).map_err(to_io)?;
        let connect_initial = ConnectInitial::new(ccr).to_vec();
        self.write_x224_data(&connect_initial)?;

        let response = self.read_x224_data()?;
        let connect_response = ConnectResponse::decode(&response).map_err(to_io)?;
        let (_node_id, server_ud) =
            gcc::decode_conference_create_response(&connect_response.user_data).map_err(to_io)?;
        gcc::parse_user_data(&server_ud).map_err(to_io)
    }

    /// Send the MCS Erect Domain Request (no response expected).
    pub fn erect_domain(&mut self) -> io::Result<()> {
        let pdu = DomainPdu::ErectDomainRequest {
            sub_height: 0,
            sub_interval: 0,
        }
        .to_vec()
        .map_err(to_io)?;
        self.write_x224_data(&pdu)
    }

    /// Send an Attach User Request and return the assigned `UserId`.
    pub fn attach_user(&mut self) -> io::Result<u16> {
        let req = DomainPdu::AttachUserRequest.to_vec().map_err(to_io)?;
        self.write_x224_data(&req)?;

        let response = self.read_x224_data()?;
        match DomainPdu::decode(&response).map_err(to_io)? {
            DomainPdu::AttachUserConfirm {
                result: McsResult::Successful,
                initiator: Some(user_id),
            } => Ok(user_id),
            DomainPdu::AttachUserConfirm { result, .. } => {
                Err(protocol_error(format!("attach user rejected: {result:?}")))
            }
            other => Err(protocol_error(format!(
                "expected Attach User Confirm, got {other:?}"
            ))),
        }
    }

    /// Join `channel_id` as `user_id`, waiting for the confirm.
    pub fn join_channel(&mut self, user_id: u16, channel_id: u16) -> io::Result<()> {
        let req = DomainPdu::ChannelJoinRequest {
            initiator: user_id,
            channel_id,
        }
        .to_vec()
        .map_err(to_io)?;
        self.write_x224_data(&req)?;

        let response = self.read_x224_data()?;
        match DomainPdu::decode(&response).map_err(to_io)? {
            DomainPdu::ChannelJoinConfirm {
                result: McsResult::Successful,
                ..
            } => Ok(()),
            DomainPdu::ChannelJoinConfirm { result, .. } => Err(protocol_error(format!(
                "channel {channel_id} join rejected: {result:?}"
            ))),
            other => Err(protocol_error(format!(
                "expected Channel Join Confirm, got {other:?}"
            ))),
        }
    }

    // --- I/O channel traffic ---------------------------------------------

    /// Send `data` on `channel_id` as a Send Data Request from `user_id`.
    pub fn send_data(&mut self, user_id: u16, channel_id: u16, data: &[u8]) -> io::Result<()> {
        let pdu = DomainPdu::SendDataRequest {
            initiator: user_id,
            channel_id,
            user_data: data,
        }
        .to_vec()
        .map_err(to_io)?;
        self.write_x224_data(&pdu)
    }

    /// Receive one Send Data Indication, returning `(channel_id, data)`.
    pub fn recv_data(&mut self) -> io::Result<(u16, Vec<u8>)> {
        let response = self.read_x224_data()?;
        match DomainPdu::decode(&response).map_err(to_io)? {
            DomainPdu::SendDataIndication {
                channel_id,
                user_data,
                ..
            } => Ok((channel_id, user_data.to_vec())),
            other => Err(protocol_error(format!(
                "expected Send Data Indication, got {other:?}"
            ))),
        }
    }

    // --- Security commencement and encrypted PDUs ------------------------

    /// Send the Security Exchange PDU: RSA-encrypt `client_random` with the
    /// server's public key and ship it on the I/O channel.
    ///
    /// This PDU is never encrypted — it is what establishes the session keys.
    pub fn security_exchange(
        &mut self,
        user_id: u16,
        io_channel: u16,
        key: &RsaPublicKey,
        client_random: &[u8],
    ) -> io::Result<()> {
        let encrypted = key.encrypt(client_random).map_err(to_io)?;
        let pdu = security::encode_security_exchange(&encrypted);
        self.send_data(user_id, io_channel, &pdu)
    }

    /// Send a PDU under a Basic Security Header on the I/O channel, encrypting
    /// with `session` when present.
    pub fn send_secure(
        &mut self,
        session: Option<&mut Rc4Session>,
        user_id: u16,
        io_channel: u16,
        base_flags: u16,
        payload: &[u8],
    ) -> io::Result<()> {
        let wrapped = security::wrap_pdu(session, base_flags, payload);
        self.send_data(user_id, io_channel, &wrapped)
    }

    /// Receive a security-wrapped PDU on the I/O channel, returning
    /// `(channel_id, security_flags, body)` and decrypting with `session` when
    /// the header sets `SEC_ENCRYPT`.
    pub fn recv_secure(
        &mut self,
        session: Option<&mut Rc4Session>,
    ) -> io::Result<(u16, u16, Vec<u8>)> {
        let (channel, data) = self.recv_data()?;
        let (flags, body) = security::unwrap_pdu(session, &data).map_err(to_io)?;
        Ok((channel, flags, body))
    }

    /// Send the Client Info PDU (logon data) on the I/O channel, encrypting
    /// with `session` when present.
    pub fn send_client_info(
        &mut self,
        session: Option<&mut Rc4Session>,
        user_id: u16,
        io_channel: u16,
        info: &ClientInfo,
    ) -> io::Result<()> {
        // The info PDU always uses the Unicode string form here.
        let mut info = info.clone();
        info.flags |= INFO_UNICODE;
        self.send_secure(session, user_id, io_channel, SEC_INFO_PKT, &info.to_vec())
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::collections::VecDeque;

    /// An in-memory duplex stream: reads drain `inbound`, writes append to
    /// `outbound`.
    struct MockStream {
        inbound: VecDeque<u8>,
        outbound: Vec<u8>,
    }

    impl MockStream {
        fn new(inbound: Vec<u8>) -> Self {
            MockStream {
                inbound: inbound.into(),
                outbound: Vec::new(),
            }
        }
    }

    impl Read for MockStream {
        fn read(&mut self, buf: &mut [u8]) -> io::Result<usize> {
            let mut n = 0;
            while n < buf.len() {
                match self.inbound.pop_front() {
                    Some(b) => {
                        buf[n] = b;
                        n += 1;
                    }
                    None => break,
                }
            }
            if n == 0 && !buf.is_empty() {
                return Err(io::ErrorKind::UnexpectedEof.into());
            }
            Ok(n)
        }
    }

    impl Write for MockStream {
        fn write(&mut self, buf: &[u8]) -> io::Result<usize> {
            self.outbound.extend_from_slice(buf);
            Ok(buf.len())
        }
        fn flush(&mut self) -> io::Result<()> {
            Ok(())
        }
    }

    /// Wrap an X.224 TPDU in TPKT the way a server would frame it.
    fn framed(tpdu: Vec<u8>) -> Vec<u8> {
        Tpkt::new(&tpdu).to_vec().unwrap()
    }

    #[test]
    fn tpkt_framing_roundtrip() {
        // A server that will echo one payload back to us.
        let payload = [0xDE, 0xAD, 0xBE, 0xEF];
        let inbound = Tpkt::new(&payload).to_vec().unwrap();
        let mut t = RdpTransport::new(MockStream::new(inbound));
        assert_eq!(t.read_tpkt().unwrap(), payload);

        t.write_tpkt(&[0x01, 0x02]).unwrap();
        let out = t.into_inner().outbound;
        assert_eq!(Tpkt::decode(&out).unwrap().payload, &[0x01, 0x02]);
    }

    #[test]
    fn negotiate_returns_selected_protocol() {
        // Server replies with a Connection Confirm selecting TLS.
        let confirm = X224::ConnectionConfirm(ConnectionPdu {
            negotiation: Some(Negotiation::Response {
                flags: 0,
                selected: SecurityProtocols::SSL,
            }),
            ..Default::default()
        })
        .to_vec()
        .unwrap();
        let mut t = RdpTransport::new(MockStream::new(framed(confirm)));

        let selected = t
            .negotiate(
                SecurityProtocols::RDP | SecurityProtocols::SSL,
                Some("user"),
            )
            .unwrap();
        assert_eq!(selected, SecurityProtocols::SSL);

        // The client sent a Connection Request carrying the cookie.
        let out = t.into_inner().outbound;
        assert!(out.windows(9).any(|w| w == b"mstshash="));
    }

    #[test]
    fn negotiate_reports_failure() {
        use crate::nego::NegFailureCode;
        let failure = X224::ConnectionConfirm(ConnectionPdu {
            negotiation: Some(Negotiation::Failure {
                code: NegFailureCode::HybridRequiredByServer,
            }),
            ..Default::default()
        })
        .to_vec()
        .unwrap();
        let mut t = RdpTransport::new(MockStream::new(framed(failure)));
        assert!(t.negotiate(SecurityProtocols::RDP, None).is_err());
    }

    #[test]
    fn attach_user_returns_user_id() {
        let confirm = X224::data(
            &DomainPdu::AttachUserConfirm {
                result: McsResult::Successful,
                initiator: Some(1007),
            }
            .to_vec()
            .unwrap(),
        )
        .to_vec()
        .unwrap();
        let mut t = RdpTransport::new(MockStream::new(framed(confirm)));
        assert_eq!(t.attach_user().unwrap(), 1007);
    }

    #[test]
    fn join_channel_accepts_confirm() {
        let confirm = X224::data(
            &DomainPdu::ChannelJoinConfirm {
                result: McsResult::Successful,
                initiator: 1007,
                requested: 1003,
                channel_id: Some(1003),
            }
            .to_vec()
            .unwrap(),
        )
        .to_vec()
        .unwrap();
        let mut t = RdpTransport::new(MockStream::new(framed(confirm)));
        assert!(t.join_channel(1007, 1003).is_ok());
    }

    #[test]
    fn recv_data_extracts_channel_payload() {
        let indication = X224::data(
            &DomainPdu::SendDataIndication {
                initiator: 1002,
                channel_id: 1003,
                user_data: &[0xAA, 0xBB, 0xCC],
            }
            .to_vec()
            .unwrap(),
        )
        .to_vec()
        .unwrap();
        let mut t = RdpTransport::new(MockStream::new(framed(indication)));
        let (channel, data) = t.recv_data().unwrap();
        assert_eq!(channel, 1003);
        assert_eq!(data, [0xAA, 0xBB, 0xCC]);
    }

    #[test]
    fn security_exchange_sends_encrypted_random() {
        // No server response needed; we only inspect what the client writes.
        let mut t = RdpTransport::new(MockStream::new(Vec::new()));
        // Tiny RSA key so the encrypt succeeds on a short "random".
        let key = RsaPublicKey {
            modulus_le: vec![0xA1, 0x0C],
            exponent: 17,
        };
        t.security_exchange(1007, 1003, &key, &[42]).unwrap();

        let out = t.into_inner().outbound;
        // Peel TPKT → X.224 Data → MCS Send Data Request → Security Exchange.
        let payload = Tpkt::decode(&out).unwrap().payload.to_vec();
        let X224::Data(mcs) = X224::decode(&payload).unwrap() else {
            panic!("expected Data TPDU");
        };
        let DomainPdu::SendDataRequest { user_data, .. } = DomainPdu::decode(mcs).unwrap() else {
            panic!("expected Send Data Request");
        };
        // Security Exchange header flags = SEC_EXCHANGE_PKT.
        assert_eq!(u16::from_le_bytes([user_data[0], user_data[1]]), 0x0001);
    }

    #[test]
    fn client_info_is_encrypted_when_session_present() {
        use crate::security::{derive_session_keys, RANDOM_LEN, SEC_ENCRYPT};

        let keys = derive_session_keys(&[1u8; RANDOM_LEN], &[2u8; RANDOM_LEN], 0x02);
        let mut session = Rc4Session::new(&keys);
        let mut t = RdpTransport::new(MockStream::new(Vec::new()));
        let info = ClientInfo::new("CORP", "alice", "secret");
        t.send_client_info(Some(&mut session), 1007, 1003, &info)
            .unwrap();

        let out = t.into_inner().outbound;
        let payload = Tpkt::decode(&out).unwrap().payload.to_vec();
        let X224::Data(mcs) = X224::decode(&payload).unwrap() else {
            panic!("expected Data TPDU");
        };
        let DomainPdu::SendDataRequest { user_data, .. } = DomainPdu::decode(mcs).unwrap() else {
            panic!("expected Send Data Request");
        };
        let flags = u16::from_le_bytes([user_data[0], user_data[1]]);
        assert!(flags & SEC_INFO_PKT != 0);
        assert!(flags & SEC_ENCRYPT != 0);
    }

    #[test]
    fn recv_secure_decrypts_indication() {
        use crate::security::{derive_session_keys, SessionKeys, RANDOM_LEN};

        // Server-side keys are the mirror of the client's.
        let client_keys = derive_session_keys(&[3u8; RANDOM_LEN], &[4u8; RANDOM_LEN], 0x02);
        let server_keys = SessionKeys {
            mac_key: client_keys.mac_key.clone(),
            encrypt_key: client_keys.decrypt_key.clone(),
            decrypt_key: client_keys.encrypt_key.clone(),
        };
        let mut server_session = Rc4Session::new(&server_keys);
        let mut client_session = Rc4Session::new(&client_keys);

        // Server wraps a licensing-ish payload encrypted and frames it.
        let wrapped = security::wrap_pdu(Some(&mut server_session), 0, b"server-payload");
        let indication = X224::data(
            &DomainPdu::SendDataIndication {
                initiator: 1002,
                channel_id: 1003,
                user_data: &wrapped,
            }
            .to_vec()
            .unwrap(),
        )
        .to_vec()
        .unwrap();

        let mut t = RdpTransport::new(MockStream::new(framed(indication)));
        let (channel, _flags, body) = t.recv_secure(Some(&mut client_session)).unwrap();
        assert_eq!(channel, 1003);
        assert_eq!(body, b"server-payload");
    }

    #[test]
    fn server_crypto_extracts_key() {
        use crate::gcc::{ServerSecurityData, ENCRYPTION_METHOD_128BIT};
        use crate::security::RsaPublicKey;

        // Build a minimal proprietary certificate for a 64-bit key.
        let modulus = [0x11u8, 0x22, 0x33, 0x44, 0x55, 0x66, 0x77, 0x88];
        let mut cert = crate::cursor::Writer::new();
        cert.write_u32_le(1); // CERT_TYPE_PROPRIETARY
        cert.write_u32_le(1);
        cert.write_u32_le(1);
        cert.write_u16_le(0x0006);
        cert.write_u16_le(0);
        cert.write_u32_le(0x3141_5352); // "RSA1"
        cert.write_u32_le(modulus.len() as u32 + 8);
        cert.write_u32_le(modulus.len() as u32 * 8);
        cert.write_u32_le(modulus.len() as u32 - 1);
        cert.write_u32_le(65537);
        cert.write_bytes(&modulus);
        cert.write_bytes(&[0u8; 8]);
        cert.write_u16_le(0x0008);
        cert.write_u16_le(0);

        let blocks = vec![UserDataBlock::ServerSecurity(ServerSecurityData {
            encryption_method: ENCRYPTION_METHOD_128BIT,
            encryption_level: 2,
            server_random: vec![0xAB; 32],
            server_certificate: cert.into_vec(),
        })];
        let crypto = server_crypto(&blocks).unwrap().unwrap();
        assert_eq!(crypto.encryption_method, ENCRYPTION_METHOD_128BIT);
        assert_eq!(crypto.server_random, vec![0xAB; 32]);
        assert_eq!(
            crypto.public_key,
            RsaPublicKey {
                modulus_le: modulus.to_vec(),
                exponent: 65537,
            }
        );
    }

    #[test]
    fn server_crypto_none_when_no_encryption() {
        use crate::gcc::ServerSecurityData;
        let blocks = vec![UserDataBlock::ServerSecurity(ServerSecurityData {
            encryption_method: 0,
            encryption_level: 0,
            server_random: Vec::new(),
            server_certificate: Vec::new(),
        })];
        assert_eq!(server_crypto(&blocks).unwrap(), None);
    }

    #[test]
    fn mcs_connect_parses_server_blocks() {
        use crate::gcc::{ServerCoreData, ServerNetworkData};

        // Server builds a Connect-Response wrapping SC_CORE + SC_NET.
        let server_blocks = vec![
            UserDataBlock::ServerCore(ServerCoreData {
                version: 0x0008_0004,
                client_requested_protocols: Some(0),
                early_capability_flags: None,
            }),
            UserDataBlock::ServerNetwork(ServerNetworkData {
                io_channel_id: 1003,
                channel_ids: vec![],
            }),
        ];
        let server_ud = gcc::encode_user_data(&server_blocks).unwrap();
        let ccrsp = gcc::encode_conference_create_response(1002, &server_ud).unwrap();
        let response = ConnectResponse {
            result: McsResult::Successful,
            called_connect_id: 0,
            domain_parameters: crate::mcs::DomainParameters::client_target(),
            user_data: ccrsp,
        };
        let inbound = framed(X224::data(&response.to_vec()).to_vec().unwrap());

        let mut t = RdpTransport::new(MockStream::new(inbound));
        let blocks = t
            .mcs_connect(&[UserDataBlock::ServerCore(ServerCoreData {
                version: 0x0008_0004,
                client_requested_protocols: None,
                early_capability_flags: None,
            })])
            .unwrap();
        assert_eq!(blocks, server_blocks);
    }
}
