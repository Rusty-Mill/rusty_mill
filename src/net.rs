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
//! [`RdpTransport::establish`] chains the whole standard-RDP sequence —
//! negotiation, MCS connect, channel setup, security exchange, encrypted
//! Client Info, licensing, capability exchange, and connection finalization —
//! into one call and returns an active [`RdpSession`]. [`server_crypto`] pulls
//! the server's RSA key and random out of the Connect-Response for it.
//!
//! Everything here stays on the standard library, so the crate remains
//! dependency-free.

use std::io::{self, Read, Write};

use crate::capabilities::{client_capability_sets, ConfirmActive, DemandActive};
use crate::client_info::{ClientInfo, INFO_UNICODE};
use crate::finalization::client_finalization_sequence;
use crate::gcc::{
    self, ClientClusterData, ClientCoreData, ClientNetworkData, ClientSecurityData,
    ServerNetworkData, UserDataBlock, ENCRYPTION_METHOD_128BIT, ENCRYPTION_METHOD_40BIT,
};
use crate::license::LicensePdu;
use crate::mcs::{ConnectInitial, ConnectResponse, DomainPdu, McsResult, MCS_GLOBAL_CHANNEL_ID};
use crate::nego::{Negotiation, SecurityProtocols};
use crate::pdu::{ShareControlHeader, PDUTYPE_DEMANDACTIVEPDU};
use crate::security::{
    self, derive_session_keys, Rc4Session, RsaPublicKey, RANDOM_LEN, SEC_INFO_PKT, SEC_LICENSE_PKT,
};
use crate::tpkt::{Tpkt, TPKT_HEADER_LEN};
use crate::x224::{ConnectionPdu, Cookie, X224};

/// Settings for [`RdpTransport::establish`].
#[derive(Debug, Clone)]
pub struct EstablishConfig {
    /// Desktop width in pixels.
    pub desktop_width: u16,
    /// Desktop height in pixels.
    pub desktop_height: u16,
    /// Session color depth (bits per pixel).
    pub bits_per_pixel: u16,
    /// Logon domain (may be empty).
    pub domain: String,
    /// Logon user name.
    pub username: String,
    /// Logon password (may be empty).
    pub password: String,
    /// Client host name reported to the server.
    pub client_name: String,
}

impl EstablishConfig {
    /// Build a config for the given desktop size and credentials, defaulting
    /// to 16bpp and a `rusty-rdp` client name.
    pub fn new(width: u16, height: u16, domain: &str, username: &str, password: &str) -> Self {
        EstablishConfig {
            desktop_width: width,
            desktop_height: height,
            bits_per_pixel: 16,
            domain: domain.to_string(),
            username: username.to_string(),
            password: password.to_string(),
            client_name: "rusty-rdp".to_string(),
        }
    }
}

/// A session that reached the "active" state after [`RdpTransport::establish`].
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct RdpSession {
    /// The client's assigned MCS `UserId`.
    pub user_id: u16,
    /// The MCS I/O channel carrying the main RDP data.
    pub io_channel: u16,
    /// The share id assigned by the server's Demand Active PDU.
    pub share_id: u32,
    /// The server's MCS channel id (the Demand Active `pduSource`).
    pub server_channel: u16,
}

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
///
/// After [`RdpTransport::establish`] (or a manual `security_exchange`) the
/// transport holds the RC4 session used to encrypt and decrypt I/O-channel
/// traffic.
pub struct RdpTransport<S> {
    stream: S,
    session: Option<Rc4Session>,
}

impl<S: Read + Write> RdpTransport<S> {
    /// Wrap a connected stream.
    pub fn new(stream: S) -> Self {
        RdpTransport {
            stream,
            session: None,
        }
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

    // --- Full session establishment --------------------------------------

    /// Send a Share Data / Share Control PDU on the I/O channel, encrypting it
    /// with the stored session when one is active.
    fn send_share(&mut self, user_id: u16, io_channel: u16, share: &[u8]) -> io::Result<()> {
        let mut session = self.session.take();
        let result = match session.as_mut() {
            Some(s) => self.send_secure(Some(s), user_id, io_channel, 0, share),
            None => self.send_data(user_id, io_channel, share),
        };
        self.session = session;
        result
    }

    /// Receive one security-wrapped PDU using the stored session, returning
    /// `(security_flags, body)`.
    fn recv_wrapped(&mut self) -> io::Result<(u16, Vec<u8>)> {
        let (_channel, data) = self.recv_data()?;
        let mut session = self.session.take();
        let result = security::unwrap_pdu(session.as_mut(), &data).map_err(to_io);
        self.session = session;
        result
    }

    /// Drive the entire standard-RDP connection sequence and return the active
    /// session.
    ///
    /// Performs, in order: X.224 negotiation (standard RDP security only), the
    /// GCC/MCS connect, channel setup, the RSA security exchange and session-
    /// key derivation, the encrypted Client Info PDU, the licensing exchange,
    /// the capability exchange (Demand → Confirm Active), and the client's
    /// connection-finalization sequence. `client_random` must be 32 bytes.
    ///
    /// Requires a server that offers standard RDP security *with* encryption;
    /// a TLS/CredSSP selection or an unencrypted session returns an error.
    pub fn establish(
        &mut self,
        config: &EstablishConfig,
        client_random: &[u8; RANDOM_LEN],
    ) -> io::Result<RdpSession> {
        // 1. Negotiate standard RDP security.
        let selected = self.negotiate(SecurityProtocols::RDP, Some(&config.username))?;
        if selected != SecurityProtocols::RDP {
            return Err(protocol_error(format!(
                "server requires {selected:?}; establish only supports standard RDP security"
            )));
        }

        // 2. MCS connect with our client settings.
        let client_blocks = vec![
            UserDataBlock::ClientCore(ClientCoreData::new(
                config.desktop_width,
                config.desktop_height,
                &config.client_name,
            )),
            UserDataBlock::ClientSecurity(ClientSecurityData {
                encryption_methods: ENCRYPTION_METHOD_40BIT | ENCRYPTION_METHOD_128BIT,
                ext_encryption_methods: 0,
            }),
            UserDataBlock::ClientNetwork(ClientNetworkData { channels: vec![] }),
            UserDataBlock::ClientCluster(ClientClusterData {
                flags: 0x0D,
                redirected_session_id: 0,
            }),
        ];
        let server_blocks = self.mcs_connect(&client_blocks)?;
        let crypto = server_crypto(&server_blocks)?.ok_or_else(|| {
            protocol_error(
                "server selected no encryption; establish requires standard RDP security",
            )
        })?;
        let io_channel = server_blocks
            .iter()
            .find_map(|b| match b {
                UserDataBlock::ServerNetwork(ServerNetworkData { io_channel_id, .. }) => {
                    Some(*io_channel_id)
                }
                _ => None,
            })
            .unwrap_or(MCS_GLOBAL_CHANNEL_ID);
        let virtual_channels: Vec<u16> = server_blocks
            .iter()
            .find_map(|b| match b {
                UserDataBlock::ServerNetwork(ServerNetworkData { channel_ids, .. }) => {
                    Some(channel_ids.clone())
                }
                _ => None,
            })
            .unwrap_or_default();

        // 3. Channel setup.
        self.erect_domain()?;
        let user_id = self.attach_user()?;
        self.join_channel(user_id, user_id)?;
        self.join_channel(user_id, io_channel)?;
        for vc in virtual_channels {
            self.join_channel(user_id, vc)?;
        }

        // 4. Security commencement.
        if crypto.server_random.len() != RANDOM_LEN {
            return Err(protocol_error(format!(
                "server random is {} bytes, expected {RANDOM_LEN}",
                crypto.server_random.len()
            )));
        }
        let mut server_random = [0u8; RANDOM_LEN];
        server_random.copy_from_slice(&crypto.server_random);
        self.security_exchange(user_id, io_channel, &crypto.public_key, client_random)?;
        let keys = derive_session_keys(client_random, &server_random, crypto.encryption_method);
        self.session = Some(Rc4Session::new(&keys));

        // 5. Encrypted Client Info (logon).
        let info = ClientInfo::new(&config.domain, &config.username, &config.password);
        {
            let mut session = self.session.take();
            let result = self.send_client_info(session.as_mut(), user_id, io_channel, &info);
            self.session = session;
            result?;
        }

        // 6. Licensing + 7. capability exchange: read until Demand Active.
        let (share_id, server_channel) = self.await_activation(io_channel)?;

        // 7b. Confirm Active with our capability sets.
        let confirm = ConfirmActive::new(
            share_id,
            client_capability_sets(
                config.desktop_width,
                config.desktop_height,
                config.bits_per_pixel,
            ),
        );
        let confirm_bytes = confirm.encode(user_id).map_err(to_io)?;
        self.send_share(user_id, io_channel, &confirm_bytes)?;

        // 8. Client connection-finalization sequence.
        for pdu in client_finalization_sequence(server_channel) {
            let bytes = pdu.encode(share_id, user_id).map_err(to_io)?;
            self.send_share(user_id, io_channel, &bytes)?;
        }

        Ok(RdpSession {
            user_id,
            io_channel,
            share_id,
            server_channel,
        })
    }

    /// Consume licensing PDUs until the server signals a valid client, then
    /// read the Demand Active PDU. Returns `(share_id, server_channel)`.
    fn await_activation(&mut self, _io_channel: u16) -> io::Result<(u32, u16)> {
        for _ in 0..16 {
            let (flags, body) = self.recv_wrapped()?;
            if flags & SEC_LICENSE_PKT != 0 {
                match LicensePdu::decode(&body).map_err(to_io)? {
                    LicensePdu::ErrorAlert(msg) if msg.is_valid_client() => continue,
                    LicensePdu::ErrorAlert(msg) => {
                        return Err(protocol_error(format!(
                            "licensing rejected: error {:#x}",
                            msg.error_code
                        )));
                    }
                    LicensePdu::Other { msg_type, .. } => {
                        return Err(protocol_error(format!(
                            "server requires full licensing (message type {msg_type:#x}), \
                             which is not implemented"
                        )));
                    }
                }
            }
            // Not a licensing PDU: expect a Share Control PDU.
            let (control, _) = ShareControlHeader::decode(&body).map_err(to_io)?;
            if control.pdu_type == PDUTYPE_DEMANDACTIVEPDU {
                let (server_channel, demand) = DemandActive::decode(&body).map_err(to_io)?;
                return Ok((demand.share_id, server_channel));
            }
            // Ignore any other share control PDU (e.g. an early Deactivate All).
        }
        Err(protocol_error(
            "did not receive a Demand Active PDU during activation",
        ))
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
    fn establish_drives_full_standard_rdp_handshake() {
        use crate::capabilities::{CapabilitySet, DemandActive, GeneralCapabilitySet};
        use crate::gcc::{
            ServerCoreData, ServerNetworkData, ServerSecurityData, ENCRYPTION_METHOD_128BIT,
        };
        use crate::license::{LicenseErrorMessage, LicensePdu};
        use crate::mcs::DomainParameters;
        use crate::security::{derive_session_keys, SessionKeys, RANDOM_LEN};

        let client_random = [0x5Au8; RANDOM_LEN];
        let server_random = [0xA5u8; RANDOM_LEN];

        // The server's session is the mirror of the client's derived keys.
        let client_keys =
            derive_session_keys(&client_random, &server_random, ENCRYPTION_METHOD_128BIT);
        let server_keys = SessionKeys {
            mac_key: client_keys.mac_key.clone(),
            encrypt_key: client_keys.decrypt_key.clone(),
            decrypt_key: client_keys.encrypt_key.clone(),
        };
        let mut server_session = Rc4Session::new(&server_keys);

        // A minimal proprietary certificate (tiny RSA key; the mock never
        // decrypts, it only needs the client's encrypt step to succeed).
        let modulus = [0xA1u8, 0x0C];
        let mut cert = crate::cursor::Writer::new();
        cert.write_u32_le(1);
        cert.write_u32_le(1);
        cert.write_u32_le(1);
        cert.write_u16_le(0x0006);
        cert.write_u16_le(0);
        cert.write_u32_le(0x3141_5352);
        cert.write_u32_le(modulus.len() as u32 + 8);
        cert.write_u32_le(modulus.len() as u32 * 8);
        cert.write_u32_le(modulus.len() as u32 - 1);
        cert.write_u32_le(17);
        cert.write_bytes(&modulus);
        cert.write_bytes(&[0u8; 8]);
        cert.write_u16_le(0x0008);
        cert.write_u16_le(0);

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
            UserDataBlock::ServerSecurity(ServerSecurityData {
                encryption_method: ENCRYPTION_METHOD_128BIT,
                encryption_level: 2,
                server_random: server_random.to_vec(),
                server_certificate: cert.into_vec(),
            }),
        ];
        let server_ud = gcc::encode_user_data(&server_blocks).unwrap();
        let ccrsp = gcc::encode_conference_create_response(1002, &server_ud).unwrap();
        let connect_response = ConnectResponse {
            result: McsResult::Successful,
            called_connect_id: 0,
            domain_parameters: DomainParameters::client_target(),
            user_data: ccrsp,
        };

        let data_ind = |payload: &[u8]| -> Vec<u8> {
            framed(
                X224::data(
                    &DomainPdu::SendDataIndication {
                        initiator: 1002,
                        channel_id: 1003,
                        user_data: payload,
                    }
                    .to_vec()
                    .unwrap(),
                )
                .to_vec()
                .unwrap(),
            )
        };

        // Licensing: encrypted valid-client alert.
        let license = LicensePdu::ErrorAlert(LicenseErrorMessage::valid_client())
            .to_vec()
            .unwrap();
        let license_wrapped =
            security::wrap_pdu(Some(&mut server_session), SEC_LICENSE_PKT, &license);

        // Demand Active from the server's channel (1002), encrypted.
        let demand = DemandActive {
            share_id: 0x0000_1234,
            source_descriptor: b"RDP\0".to_vec(),
            capability_sets: vec![CapabilitySet::General(GeneralCapabilitySet::default())],
            session_id: 0,
        };
        let demand_bytes = demand.encode(1002).unwrap();
        let demand_wrapped = security::wrap_pdu(Some(&mut server_session), 0, &demand_bytes);

        // Assemble the server's byte stream in the exact order the client reads.
        let confirm = |requested: u16, channel: u16| -> Vec<u8> {
            framed(
                X224::data(
                    &DomainPdu::ChannelJoinConfirm {
                        result: McsResult::Successful,
                        initiator: 1007,
                        requested,
                        channel_id: Some(channel),
                    }
                    .to_vec()
                    .unwrap(),
                )
                .to_vec()
                .unwrap(),
            )
        };
        let mut inbound = Vec::new();
        // 1. Connection Confirm (RDP selected).
        inbound.extend(framed(
            X224::ConnectionConfirm(ConnectionPdu {
                negotiation: Some(Negotiation::Response {
                    flags: 0,
                    selected: SecurityProtocols::RDP,
                }),
                ..Default::default()
            })
            .to_vec()
            .unwrap(),
        ));
        // 2. MCS Connect-Response.
        inbound.extend(framed(
            X224::data(&connect_response.to_vec()).to_vec().unwrap(),
        ));
        // 3. Attach User Confirm.
        inbound.extend(framed(
            X224::data(
                &DomainPdu::AttachUserConfirm {
                    result: McsResult::Successful,
                    initiator: Some(1007),
                }
                .to_vec()
                .unwrap(),
            )
            .to_vec()
            .unwrap(),
        ));
        // 4/5. Channel join confirms (user channel, then I/O channel).
        inbound.extend(confirm(1007, 1007));
        inbound.extend(confirm(1003, 1003));
        // 6. Licensing, then 7. Demand Active.
        inbound.extend(data_ind(&license_wrapped));
        inbound.extend(data_ind(&demand_wrapped));

        let mut t = RdpTransport::new(MockStream::new(inbound));
        let config = EstablishConfig::new(1024, 768, "CORP", "alice", "secret");
        let session = t.establish(&config, &client_random).unwrap();

        assert_eq!(session.user_id, 1007);
        assert_eq!(session.io_channel, 1003);
        assert_eq!(session.share_id, 0x0000_1234);
        assert_eq!(session.server_channel, 1002);
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
