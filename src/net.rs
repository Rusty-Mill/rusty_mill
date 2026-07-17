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
//! For the enhanced-security (TLS/CredSSP) path, the X.224 negotiation runs on
//! the raw TCP connection, the stream is then upgraded to TLS, and
//! [`RdpTransport::new_enhanced`] + [`RdpTransport::establish_enhanced`] drive
//! the rest of the sequence with the RDP security layer switched off — no
//! Security Exchange and no RC4, since TLS provides confidentiality. This
//! module stays dependency-free by being generic over the stream: bring any
//! TLS implementation (or, with the optional `tls` feature, use
//! `crate::tls::connect_tls`).
//!
//! Once active, [`RdpTransport::recv_event`] reads server updates — accepting
//! both slow-path (TPKT) and fast-path framing transparently — and
//! [`RdpTransport::send_input`] sends keyboard/mouse events over the compact
//! fast-path input path. Everything here stays on the standard library, so the
//! crate remains dependency-free.
//!
//! Static virtual channels beyond the required I/O channel — e.g. `"DRDYNVC"`,
//! which carries [`crate::dvc`]'s dynamic-channel traffic (RDPGFX,
//! redirection protocols) — are opt in: list them in
//! [`EstablishConfig::extra_channels`], look up the id the server granted with
//! [`RdpSession::channel_id`], and [`RdpTransport::recv_event`] reassembles
//! their chunked traffic (MS-RDPBCGR 2.2.6.1, [`crate::vchan`]) into
//! [`RdpEvent::ChannelData`] alongside the usual display/input events;
//! [`RdpTransport::send_channel_data`] is the outbound counterpart.

use std::collections::HashMap;
use std::io::{self, Read, Write};

use crate::capabilities::{client_capability_sets, ConfirmActive, DemandActive};
use crate::client_info::{ClientInfo, INFO_UNICODE};
use crate::finalization::client_finalization_sequence;
use crate::finalization::FinalizationPdu;
use crate::gcc::{
    self, ChannelDef, ClientClusterData, ClientCoreData, ClientNetworkData, ClientSecurityData,
    ServerNetworkData, UserDataBlock, ENCRYPTION_METHOD_128BIT, ENCRYPTION_METHOD_40BIT,
};
use crate::input::InputEvent;
use crate::license::LicensePdu;
use crate::mcs::{ConnectInitial, ConnectResponse, DomainPdu, McsResult, MCS_GLOBAL_CHANNEL_ID};
use crate::nego::{Negotiation, SecurityProtocols};
use crate::output::{BitmapData, PaletteUpdate, UpdatePdu};
use crate::pdu::{
    ShareControlHeader, ShareDataHeader, PDUTYPE2_CONTROL, PDUTYPE2_FONTMAP, PDUTYPE2_POINTER,
    PDUTYPE2_SYNCHRONIZE, PDUTYPE2_UPDATE, PDUTYPE_DEACTIVATEALLPDU, PDUTYPE_DEMANDACTIVEPDU,
};
use crate::pointer::PointerUpdate;
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
    /// Additional static virtual channels to request beyond the required I/O
    /// channel — e.g. [`crate::dvc::DRDYNVC_CHANNEL_NAME`] to enable dynamic
    /// virtual channels (RDPGFX, redirection protocols). Empty by default;
    /// the assigned channel ids come back on [`RdpSession::channel_id`].
    pub extra_channels: Vec<ChannelDef>,
}

impl EstablishConfig {
    /// Build a config for the given desktop size and credentials, defaulting
    /// to 16bpp, a `rusty-rdp` client name, and no extra virtual channels.
    pub fn new(width: u16, height: u16, domain: &str, username: &str, password: &str) -> Self {
        EstablishConfig {
            desktop_width: width,
            desktop_height: height,
            bits_per_pixel: 16,
            domain: domain.to_string(),
            username: username.to_string(),
            password: password.to_string(),
            client_name: "rusty-rdp".to_string(),
            extra_channels: Vec::new(),
        }
    }
}

/// A session that reached the "active" state after [`RdpTransport::establish`].
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct RdpSession {
    /// The client's assigned MCS `UserId`.
    pub user_id: u16,
    /// The MCS I/O channel carrying the main RDP data.
    pub io_channel: u16,
    /// The share id assigned by the server's Demand Active PDU.
    pub share_id: u32,
    /// The server's MCS channel id (the Demand Active `pduSource`).
    pub server_channel: u16,
    /// MCS channel ids assigned to [`EstablishConfig::extra_channels`], keyed
    /// by the channel name that was requested. A name absent from this map
    /// means the server did not grant that channel.
    pub channels: HashMap<String, u16>,
}

impl RdpSession {
    /// Look up the MCS channel id assigned to a requested static virtual
    /// channel by name (e.g. [`crate::dvc::DRDYNVC_CHANNEL_NAME`]).
    pub fn channel_id(&self, name: &str) -> Option<u16> {
        self.channels.get(name).copied()
    }
}

/// A server-to-client event read after the session is active.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum RdpEvent {
    /// A bitmap update: one or more pixel rectangles.
    Bitmap(Vec<BitmapData>),
    /// A palette update (8bpp color table).
    Palette(PaletteUpdate),
    /// A pointer/cursor update (shape, position, or system cursor).
    Pointer(PointerUpdate),
    /// An update-synchronize marker.
    UpdateSynchronize,
    /// Raw drawing orders (not decoded here).
    Orders(Vec<u8>),
    /// A server connection-finalization PDU (synchronize / control / font).
    Finalization(FinalizationPdu),
    /// The server asked to deactivate the share (a reactivation may follow).
    DeactivateAll,
    /// A reassembled message on a static virtual channel other than the I/O
    /// channel (MS-RDPBCGR 2.2.6.1) — e.g. dynamic-channel traffic on
    /// [`crate::dvc::DRDYNVC_CHANNEL_NAME`], decodable with [`crate::dvc`].
    ChannelData {
        /// The MCS channel id the data arrived on (see
        /// [`RdpSession::channel_id`] to map this back to a channel name).
        channel_id: u16,
        /// The reassembled message.
        data: Vec<u8>,
    },
    /// A share PDU this driver does not model.
    Other {
        /// The Share Control `pduType`.
        pdu_type: u16,
        /// The Share Data `pduType2`, when the PDU is a Data PDU.
        pdu_type2: Option<u8>,
    },
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
    /// Events decoded from a fast-path PDU that bundled several updates,
    /// waiting to be returned one at a time by [`RdpTransport::recv_event`].
    pending: std::collections::VecDeque<RdpEvent>,
    /// `true` when the stream already provides encryption (TLS/CredSSP), so
    /// the RDP security layer is disabled: no Security Exchange, no RC4, and
    /// data PDUs carry no Basic Security Header (MS-RDPBCGR 5.4). Only the
    /// Client Info and licensing PDUs keep a header in this mode.
    enhanced: bool,
    /// The MCS I/O channel id, once known (set during channel setup). Slow-path
    /// traffic on any other joined channel is virtual-channel data, not a
    /// Share Control/Data PDU.
    io_channel: Option<u16>,
    /// Per-channel reassembly state for static virtual channel traffic
    /// (MS-RDPBCGR 2.2.6.1), keyed by MCS channel id.
    channel_reassemblers: HashMap<u16, crate::vchan::Reassembler>,
}

impl<S: Read + Write> RdpTransport<S> {
    /// Wrap a connected stream that speaks standard RDP security (the RDP
    /// security layer encrypts PDUs itself).
    pub fn new(stream: S) -> Self {
        RdpTransport {
            stream,
            session: None,
            pending: std::collections::VecDeque::new(),
            enhanced: false,
            io_channel: None,
            channel_reassemblers: HashMap::new(),
        }
    }

    /// Wrap a stream that already provides encryption (TLS/CredSSP).
    ///
    /// Use this after the X.224 negotiation on the raw TCP connection has
    /// selected an enhanced-security protocol and the stream has been upgraded
    /// (e.g. wrapped in TLS). The RDP security layer is left off:
    /// [`RdpTransport::establish_enhanced`] skips the Security Exchange and no
    /// PDU is RC4-encrypted.
    pub fn new_enhanced(stream: S) -> Self {
        RdpTransport {
            stream,
            session: None,
            pending: std::collections::VecDeque::new(),
            enhanced: true,
            io_channel: None,
            channel_reassemblers: HashMap::new(),
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

    /// Send a Share Data / Share Control PDU (or any other channel payload)
    /// on `channel_id`, encrypting it with the stored session when active —
    /// exactly how the I/O channel and other static virtual channels both
    /// carry traffic once encryption is negotiated.
    fn send_share(&mut self, user_id: u16, channel_id: u16, share: &[u8]) -> io::Result<()> {
        let mut session = self.session.take();
        let result = match session.as_mut() {
            Some(s) => self.send_secure(Some(s), user_id, channel_id, 0, share),
            None => self.send_data(user_id, channel_id, share),
        };
        self.session = session;
        result
    }

    /// Send `data` on virtual channel `channel_id`, chunking it per
    /// MS-RDPBCGR 2.2.6.1 (`crate::vchan::chunk`) and encrypting each chunk
    /// with the stored session when active. Use the id from
    /// [`RdpSession::channel_id`] for a channel requested via
    /// [`EstablishConfig::extra_channels`].
    pub fn send_channel_data(
        &mut self,
        user_id: u16,
        channel_id: u16,
        data: &[u8],
    ) -> io::Result<()> {
        for chunk in crate::vchan::chunk(data, crate::vchan::DEFAULT_CHUNK_SIZE) {
            self.send_share(user_id, channel_id, &chunk)?;
        }
        Ok(())
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

    /// Build the client GCC settings blocks for the connection.
    ///
    /// `selected_protocol` echoes the X.224 negotiation into the client core
    /// data (0 for standard RDP, [`SecurityProtocols::SSL`] etc. for enhanced
    /// security). `encryption_methods` advertises the RDP-layer ciphers we
    /// accept — always 0 under TLS, where the RDP security layer is off.
    fn build_client_blocks(
        config: &EstablishConfig,
        selected_protocol: u32,
        encryption_methods: u32,
    ) -> Vec<UserDataBlock> {
        let mut core = ClientCoreData::new(
            config.desktop_width,
            config.desktop_height,
            &config.client_name,
        );
        core.server_selected_protocol = Some(selected_protocol);
        vec![
            UserDataBlock::ClientCore(core),
            UserDataBlock::ClientSecurity(ClientSecurityData {
                encryption_methods,
                ext_encryption_methods: 0,
            }),
            UserDataBlock::ClientNetwork(ClientNetworkData {
                channels: config.extra_channels.clone(),
            }),
            UserDataBlock::ClientCluster(ClientClusterData {
                flags: 0x0D,
                redirected_session_id: 0,
            }),
        ]
    }

    /// Zip the requested [`EstablishConfig::extra_channels`] names against the
    /// server's granted channel ids (in the same order), skipping any the
    /// server did not grant (id `0`).
    fn build_channel_map(
        config: &EstablishConfig,
        virtual_channels: &[u16],
    ) -> HashMap<String, u16> {
        config
            .extra_channels
            .iter()
            .zip(virtual_channels)
            .filter(|(_, &id)| id != 0)
            .map(|(def, &id)| (def.name.clone(), id))
            .collect()
    }

    /// Run the MCS domain setup: erect the domain, attach a user, and join the
    /// user, I/O, and any virtual channels advertised in `server_blocks`.
    /// Returns `(user_id, io_channel, granted_virtual_channel_ids)` — the
    /// third element is in the same order as [`EstablishConfig::extra_channels`]
    /// was requested, with `0` marking a channel the server did not grant.
    fn join_all_channels(
        &mut self,
        server_blocks: &[UserDataBlock],
    ) -> io::Result<(u16, u16, Vec<u16>)> {
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

        self.erect_domain()?;
        let user_id = self.attach_user()?;
        self.join_channel(user_id, user_id)?;
        self.join_channel(user_id, io_channel)?;
        for &vc in virtual_channels.iter().filter(|&&id| id != 0) {
            self.join_channel(user_id, vc)?;
        }
        self.io_channel = Some(io_channel);
        Ok((user_id, io_channel, virtual_channels))
    }

    /// Run the shared tail of the connection sequence: licensing, capability
    /// exchange, and the client finalization PDUs. Returns the active session.
    fn activate(
        &mut self,
        config: &EstablishConfig,
        user_id: u16,
        io_channel: u16,
        channels: HashMap<String, u16>,
    ) -> io::Result<RdpSession> {
        // Licensing + capability exchange: read until Demand Active.
        let (share_id, server_channel) = self.await_activation(io_channel)?;

        // Confirm Active with our capability sets.
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

        // Client connection-finalization sequence.
        for pdu in client_finalization_sequence(server_channel) {
            let bytes = pdu.encode(share_id, user_id).map_err(to_io)?;
            self.send_share(user_id, io_channel, &bytes)?;
        }

        Ok(RdpSession {
            user_id,
            io_channel,
            share_id,
            server_channel,
            channels,
        })
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
    /// a TLS/CredSSP selection or an unencrypted session returns an error. For
    /// the TLS path see [`RdpTransport::establish_enhanced`].
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

        // 2. MCS connect with our client settings (both RDP ciphers on offer).
        let client_blocks = Self::build_client_blocks(
            config,
            0,
            ENCRYPTION_METHOD_40BIT | ENCRYPTION_METHOD_128BIT,
        );
        let server_blocks = self.mcs_connect(&client_blocks)?;
        let crypto = server_crypto(&server_blocks)?.ok_or_else(|| {
            protocol_error(
                "server selected no encryption; establish requires standard RDP security",
            )
        })?;

        // 3. Channel setup.
        let (user_id, io_channel, virtual_channels) = self.join_all_channels(&server_blocks)?;
        let channels = Self::build_channel_map(config, &virtual_channels);

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

        // 6-8. Licensing, capabilities, finalization.
        self.activate(config, user_id, io_channel, channels)
    }

    /// Drive the connection sequence over a stream that already provides
    /// encryption (TLS/CredSSP) and return the active session.
    ///
    /// The X.224 negotiation must already have happened on the raw TCP
    /// connection and selected `selected` (an enhanced-security protocol);
    /// this transport must wrap the *upgraded* (e.g. TLS) stream — build it
    /// with [`RdpTransport::new_enhanced`]. Compared to [`establish`], the RDP
    /// security layer is off: there is no Security Exchange, no client random,
    /// and no RC4 — TLS carries the confidentiality. Only the Client Info and
    /// licensing PDUs carry a Basic Security Header (MS-RDPBCGR 5.4).
    ///
    /// The server is expected to send its licensing PDU(s) before the Demand
    /// Active PDU, as real servers do.
    ///
    /// [`establish`]: Self::establish
    pub fn establish_enhanced(
        &mut self,
        config: &EstablishConfig,
        selected: SecurityProtocols,
    ) -> io::Result<RdpSession> {
        self.enhanced = true;
        self.session = None;

        // MCS connect: echo the selected protocol, advertise no RDP ciphers.
        let client_blocks = Self::build_client_blocks(config, selected.0, 0);
        let server_blocks = self.mcs_connect(&client_blocks)?;

        // Channel setup.
        let (user_id, io_channel, virtual_channels) = self.join_all_channels(&server_blocks)?;
        let channels = Self::build_channel_map(config, &virtual_channels);

        // Client Info (logon) in the clear under a SEC_INFO_PKT header — TLS
        // already encrypts the stream, so no RC4 session is used.
        let info = ClientInfo::new(&config.domain, &config.username, &config.password);
        self.send_client_info(None, user_id, io_channel, &info)?;

        // Licensing, capabilities, finalization.
        self.activate(config, user_id, io_channel, channels)
    }

    /// Consume licensing PDUs until the server signals a valid client, then
    /// read the Demand Active PDU. Returns `(share_id, server_channel)`.
    fn await_activation(&mut self, _io_channel: u16) -> io::Result<(u32, u16)> {
        if self.enhanced {
            return self.await_activation_enhanced();
        }
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

    /// Enhanced-security variant of [`await_activation`](Self::await_activation).
    ///
    /// Under TLS the RDP security layer is off, so only the licensing PDUs
    /// carry a Basic Security Header (`SEC_LICENSE_PKT`); the Demand Active and
    /// every later PDU are bare Share Control PDUs. We read licensing PDUs
    /// until the server signals a valid client, then read the headerless
    /// Demand Active.
    fn await_activation_enhanced(&mut self) -> io::Result<(u32, u16)> {
        let mut licensing = true;
        for _ in 0..16 {
            let (_channel, user_data) = self.recv_data()?;

            // A licensing PDU still carries a Basic Security Header.
            if licensing && user_data.len() >= 4 {
                let flags = u16::from_le_bytes([user_data[0], user_data[1]]);
                if flags & SEC_LICENSE_PKT != 0 {
                    match LicensePdu::decode(&user_data[4..]).map_err(to_io)? {
                        LicensePdu::ErrorAlert(msg) if msg.is_valid_client() => {
                            licensing = false;
                            continue;
                        }
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
            }

            // Otherwise it is a headerless Share Control PDU.
            licensing = false;
            let (control, _) = ShareControlHeader::decode(&user_data).map_err(to_io)?;
            if control.pdu_type == PDUTYPE_DEMANDACTIVEPDU {
                let (server_channel, demand) = DemandActive::decode(&user_data).map_err(to_io)?;
                return Ok((demand.share_id, server_channel));
            }
            // Ignore any other share control PDU (e.g. an early Deactivate All).
        }
        Err(protocol_error(
            "did not receive a Demand Active PDU during activation",
        ))
    }

    /// Receive and classify one server-to-client event once the session is
    /// active (after [`establish`](Self::establish)).
    ///
    /// Reads the next frame — slow-path (TPKT / X.224 / MCS / Share) or
    /// fast-path — decrypts it with the stored session, and returns a typed
    /// [`RdpEvent`]. A fast-path PDU may bundle several updates; the extras are
    /// buffered and returned by later calls. Anything not modelled comes back
    /// as [`RdpEvent::Other`] rather than an error. Slow-path traffic on a
    /// virtual channel other than the I/O channel is reassembled
    /// (MS-RDPBCGR 2.2.6.1) and, once complete, returned as
    /// [`RdpEvent::ChannelData`].
    pub fn recv_event(&mut self) -> io::Result<RdpEvent> {
        loop {
            if let Some(event) = self.pending.pop_front() {
                return Ok(event);
            }
            let mut first = [0u8; 1];
            self.stream.read_exact(&mut first)?;
            if crate::fastpath::is_fastpath(first[0]) {
                let events = self.read_fastpath_output(first[0])?;
                self.pending.extend(events);
            } else if let Some((channel_id, body)) = self.read_slowpath_share(first[0])? {
                // Route by channel once join_all_channels has told us which one
                // is the I/O channel; if that hasn't happened (e.g. a caller
                // driving recv_event without the full establish() sequence),
                // there is nothing to route by, so treat it as Share data.
                if self.io_channel.is_none() || Some(channel_id) == self.io_channel {
                    self.pending.push_back(classify_share(&body)?);
                } else if let Some(data) = self
                    .channel_reassemblers
                    .entry(channel_id)
                    .or_default()
                    .feed(&body)
                    .map_err(to_io)?
                {
                    self.pending
                        .push_back(RdpEvent::ChannelData { channel_id, data });
                }
                // Otherwise a partial chunk was buffered; keep reading.
            }
        }
    }

    /// Read the remainder of a TPKT packet whose first byte is `first`,
    /// returning the inner X.224 TPDU payload.
    fn read_tpkt_rest(&mut self, first: u8) -> io::Result<Vec<u8>> {
        let mut rest = [0u8; 3];
        self.stream.read_exact(&mut rest)?;
        let total = u16::from_be_bytes([rest[1], rest[2]]) as usize;
        if total < TPKT_HEADER_LEN {
            return Err(protocol_error("short TPKT packet"));
        }
        let mut packet = vec![0u8; total];
        packet[..TPKT_HEADER_LEN].copy_from_slice(&[first, rest[0], rest[1], rest[2]]);
        self.stream.read_exact(&mut packet[TPKT_HEADER_LEN..])?;
        let tpkt = Tpkt::decode(&packet).map_err(to_io)?;
        Ok(tpkt.payload.to_vec())
    }

    /// Read a slow-path frame and return `(channel_id, decrypted body)`, or
    /// `None` if the frame is not a Send Data Indication.
    fn read_slowpath_share(&mut self, first: u8) -> io::Result<Option<(u16, Vec<u8>)>> {
        let tpdu = self.read_tpkt_rest(first)?;
        let inner = match X224::decode(&tpdu).map_err(to_io)? {
            X224::Data(payload) => payload.to_vec(),
            _ => return Ok(None),
        };
        let (channel_id, user_data) = match DomainPdu::decode(&inner).map_err(to_io)? {
            DomainPdu::SendDataIndication {
                channel_id,
                user_data,
                ..
            } => (channel_id, user_data.to_vec()),
            _ => return Ok(None),
        };
        if self.enhanced {
            // Under TLS, data PDUs carry no Basic Security Header.
            return Ok(Some((channel_id, user_data)));
        }
        let mut session = self.session.take();
        let result = security::unwrap_pdu(session.as_mut(), &user_data)
            .map(|(_flags, body)| body)
            .map_err(to_io);
        self.session = session;
        Ok(Some((channel_id, result?)))
    }

    /// Read a fast-path output frame whose header byte is `header` and decode
    /// its updates into events.
    fn read_fastpath_output(&mut self, header: u8) -> io::Result<Vec<RdpEvent>> {
        let l1 = {
            let mut b = [0u8; 1];
            self.stream.read_exact(&mut b)?;
            b[0]
        };
        let (total, len_field) = if l1 & 0x80 != 0 {
            let mut b = [0u8; 1];
            self.stream.read_exact(&mut b)?;
            ((((l1 & 0x7F) as usize) << 8) | b[0] as usize, 2usize)
        } else {
            (l1 as usize, 1usize)
        };
        let header_len = 1 + len_field;
        if total < header_len {
            return Err(protocol_error("short fast-path PDU"));
        }
        let mut rest = vec![0u8; total - header_len];
        self.stream.read_exact(&mut rest)?;

        let encryption_flags = (header >> 6) & 0x03;
        let update_bytes = if encryption_flags & crate::fastpath::FASTPATH_ENCRYPTED != 0 {
            if rest.len() < 8 {
                return Err(protocol_error("fast-path PDU missing signature"));
            }
            let signature = rest[..8].to_vec();
            let ciphertext = &rest[8..];
            let mut session = self.session.take();
            let result = match session.as_mut() {
                Some(s) => s.decrypt(&signature, ciphertext).map_err(to_io),
                None => Err(protocol_error("encrypted fast-path PDU but no session")),
            };
            self.session = session;
            result?
        } else {
            rest
        };

        let updates = crate::fastpath::parse_output_updates(&update_bytes).map_err(to_io)?;
        Ok(updates.into_iter().map(fastpath_update_to_event).collect())
    }

    /// Send client input as a fast-path Input PDU, encrypting with the stored
    /// session when one is active.
    pub fn send_input(&mut self, events: &[InputEvent]) -> io::Result<()> {
        let (count, event_bytes) = crate::fastpath::encode_input_events(events);
        if count > u8::MAX as usize {
            return Err(protocol_error("too many input events for one PDU"));
        }

        // numberEvents rides in the header when it fits in 4 bits, else in a
        // separate byte prefixed to the (possibly encrypted) event data.
        let (num_field, mut plaintext) = if count <= 0x0F {
            (count as u8, Vec::new())
        } else {
            (0u8, vec![count as u8])
        };
        plaintext.extend_from_slice(&event_bytes);

        let mut session = self.session.take();
        let (enc_flags, body) = match session.as_mut() {
            Some(s) => {
                let (signature, ciphertext) = s.encrypt(&plaintext);
                let mut body = signature.to_vec();
                body.extend_from_slice(&ciphertext);
                (
                    crate::fastpath::FASTPATH_ENCRYPTED | crate::fastpath::FASTPATH_SECURE_CHECKSUM,
                    body,
                )
            }
            None => (0u8, plaintext),
        };
        self.session = session;

        let header = crate::fastpath::FASTPATH_ACTION | (num_field << 2) | (enc_flags << 6);
        // Total length includes the header byte, the length field, and the body.
        let base = 1 + body.len();
        let total = if base < 0x7F { base + 1 } else { base + 2 };

        let mut w = crate::cursor::Writer::new();
        w.write_u8(header);
        crate::fastpath::write_length(&mut w, total).map_err(to_io)?;
        w.write_bytes(&body);
        self.stream.write_all(w.as_slice())?;
        self.stream.flush()
    }
}

/// Classify a decrypted Share Control / Share Data PDU body into an event.
fn classify_share(body: &[u8]) -> io::Result<RdpEvent> {
    let (control, _payload) = ShareControlHeader::decode(body).map_err(to_io)?;
    match control.pdu_type {
        PDUTYPE_DEACTIVATEALLPDU => Ok(RdpEvent::DeactivateAll),
        crate::pdu::PDUTYPE_DATAPDU => {
            let (_source, header, _data) = ShareDataHeader::decode(body).map_err(to_io)?;
            match header.pdu_type2 {
                PDUTYPE2_UPDATE => {
                    let (_s, _sid, update) = UpdatePdu::decode(body).map_err(to_io)?;
                    Ok(match update {
                        UpdatePdu::Bitmap(rects) => RdpEvent::Bitmap(rects),
                        UpdatePdu::Palette(palette) => RdpEvent::Palette(palette),
                        UpdatePdu::Synchronize => RdpEvent::UpdateSynchronize,
                        UpdatePdu::Orders(data) => RdpEvent::Orders(data),
                    })
                }
                PDUTYPE2_POINTER => {
                    let (_s, _sid, pointer) = PointerUpdate::decode(body).map_err(to_io)?;
                    Ok(RdpEvent::Pointer(pointer))
                }
                PDUTYPE2_SYNCHRONIZE | PDUTYPE2_CONTROL | PDUTYPE2_FONTMAP => {
                    let (_s, _sid, fin) = FinalizationPdu::decode(body).map_err(to_io)?;
                    Ok(RdpEvent::Finalization(fin))
                }
                other => Ok(RdpEvent::Other {
                    pdu_type: control.pdu_type,
                    pdu_type2: Some(other),
                }),
            }
        }
        other => Ok(RdpEvent::Other {
            pdu_type: other,
            pdu_type2: None,
        }),
    }
}

/// Map a fast-path update to the shared [`RdpEvent`] type.
fn fastpath_update_to_event(update: crate::fastpath::FastPathUpdate) -> RdpEvent {
    use crate::fastpath::FastPathUpdate as F;
    use crate::pointer::{PointerUpdate, SYSPTR_DEFAULT, SYSPTR_NULL};
    match update {
        F::Bitmap(rects) => RdpEvent::Bitmap(rects),
        F::Palette(palette) => RdpEvent::Palette(palette),
        F::Synchronize => RdpEvent::UpdateSynchronize,
        F::PointerHidden => RdpEvent::Pointer(PointerUpdate::System(SYSPTR_NULL)),
        F::PointerDefault => RdpEvent::Pointer(PointerUpdate::System(SYSPTR_DEFAULT)),
        F::PointerPosition { x, y } => RdpEvent::Pointer(PointerUpdate::Position { x, y }),
        F::PointerColor(pointer) => RdpEvent::Pointer(PointerUpdate::Color(pointer)),
        F::PointerNew { xor_bpp, pointer } => {
            RdpEvent::Pointer(PointerUpdate::New { xor_bpp, pointer })
        }
        F::PointerCached(index) => RdpEvent::Pointer(PointerUpdate::Cached(index)),
        F::Raw { update_code, .. } => RdpEvent::Other {
            pdu_type: 0,
            pdu_type2: Some(update_code),
        },
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
    fn recv_event_decodes_encrypted_bitmap_update() {
        use crate::output::{BitmapData, UpdatePdu};
        use crate::security::{derive_session_keys, SessionKeys, RANDOM_LEN};

        // Mirror sessions, as after a security exchange.
        let client_keys = derive_session_keys(&[3u8; RANDOM_LEN], &[4u8; RANDOM_LEN], 0x02);
        let server_keys = SessionKeys {
            mac_key: client_keys.mac_key.clone(),
            encrypt_key: client_keys.decrypt_key.clone(),
            decrypt_key: client_keys.encrypt_key.clone(),
        };
        let mut server_session = Rc4Session::new(&server_keys);

        // Server encodes a bitmap update, encrypts it, frames it.
        let update = UpdatePdu::Bitmap(vec![BitmapData::uncompressed(
            2,
            3,
            1,
            1,
            16,
            vec![0x00, 0xF8],
        )]);
        let share = update.encode(0x1234, 1002).unwrap();
        let wrapped = security::wrap_pdu(Some(&mut server_session), 0, &share);
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
        t.session = Some(Rc4Session::new(&client_keys));

        match t.recv_event().unwrap() {
            RdpEvent::Bitmap(rects) => {
                assert_eq!(rects.len(), 1);
                assert_eq!(rects[0].dest_left, 2);
                assert_eq!(rects[0].dest_top, 3);
            }
            other => panic!("expected Bitmap, got {other:?}"),
        }
    }

    #[test]
    fn recv_event_decodes_encrypted_pointer_position() {
        use crate::pointer::PointerUpdate;
        use crate::security::{derive_session_keys, SessionKeys, RANDOM_LEN};

        let client_keys = derive_session_keys(&[5u8; RANDOM_LEN], &[6u8; RANDOM_LEN], 0x02);
        let server_keys = SessionKeys {
            mac_key: client_keys.mac_key.clone(),
            encrypt_key: client_keys.decrypt_key.clone(),
            decrypt_key: client_keys.encrypt_key.clone(),
        };
        let mut server_session = Rc4Session::new(&server_keys);

        let share = PointerUpdate::Position { x: 100, y: 200 }
            .encode(0x1234, 1002)
            .unwrap();
        let wrapped = security::wrap_pdu(Some(&mut server_session), 0, &share);
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
        t.session = Some(Rc4Session::new(&client_keys));
        match t.recv_event().unwrap() {
            RdpEvent::Pointer(PointerUpdate::Position { x, y }) => {
                assert_eq!((x, y), (100, 200));
            }
            other => panic!("expected Pointer position, got {other:?}"),
        }
    }

    #[test]
    fn recv_event_decodes_fastpath_pointer_position() {
        use crate::fastpath::{write_length, FASTPATH_ACTION, FASTPATH_UPDATETYPE_PTR_POSITION};

        // Build a plaintext fast-path output PDU carrying a pointer position.
        let mut updates = crate::cursor::Writer::new();
        updates.write_u8(FASTPATH_UPDATETYPE_PTR_POSITION);
        updates.write_u16_le(4); // size
        updates.write_u16_le(50);
        updates.write_u16_le(60);
        let body = updates.into_vec();

        let mut pdu = crate::cursor::Writer::new();
        pdu.write_u8(FASTPATH_ACTION); // action 0, not encrypted
        let total = 1 + 1 + body.len(); // header + 1-byte length + body
        write_length(&mut pdu, total).unwrap();
        pdu.write_bytes(&body);

        let mut t = RdpTransport::new(MockStream::new(pdu.into_vec()));
        match t.recv_event().unwrap() {
            RdpEvent::Pointer(crate::pointer::PointerUpdate::Position { x, y }) => {
                assert_eq!((x, y), (50, 60));
            }
            other => panic!("expected fast-path pointer, got {other:?}"),
        }
    }

    #[test]
    fn recv_event_decodes_encrypted_fastpath_bitmap() {
        use crate::fastpath::{
            write_length, FASTPATH_ACTION, FASTPATH_ENCRYPTED, FASTPATH_SECURE_CHECKSUM,
            FASTPATH_UPDATETYPE_BITMAP,
        };
        use crate::security::{derive_session_keys, SessionKeys, RANDOM_LEN};

        let client_keys = derive_session_keys(&[9u8; RANDOM_LEN], &[8u8; RANDOM_LEN], 0x02);
        let server_keys = SessionKeys {
            mac_key: client_keys.mac_key.clone(),
            encrypt_key: client_keys.decrypt_key.clone(),
            decrypt_key: client_keys.encrypt_key.clone(),
        };
        let mut server_session = Rc4Session::new(&server_keys);

        // Update payload: one bitmap update (numberRectangles + a 1x1 rect).
        let mut updates = crate::cursor::Writer::new();
        updates.write_u8(FASTPATH_UPDATETYPE_BITMAP);
        let mut rect = crate::cursor::Writer::new();
        rect.write_u16_le(1); // numberRectangles
        for v in [7u16, 8, 7, 8, 1, 1, 16, 0, 2] {
            rect.write_u16_le(v);
        }
        rect.write_bytes(&[0x00, 0xF8]);
        let rect = rect.into_vec();
        updates.write_u16_le(rect.len() as u16);
        updates.write_bytes(&rect);
        let plaintext = updates.into_vec();

        let (signature, ciphertext) = server_session.encrypt(&plaintext);
        let mut body = signature.to_vec();
        body.extend_from_slice(&ciphertext);

        let mut pdu = crate::cursor::Writer::new();
        let flags = FASTPATH_ENCRYPTED | FASTPATH_SECURE_CHECKSUM;
        pdu.write_u8(FASTPATH_ACTION | (flags << 6));
        // Length includes the header byte and the length field itself.
        let base = 1 + body.len();
        let total = if base < 0x7F { base + 1 } else { base + 2 };
        write_length(&mut pdu, total).unwrap();
        pdu.write_bytes(&body);

        let mut t = RdpTransport::new(MockStream::new(pdu.into_vec()));
        t.session = Some(Rc4Session::new(&client_keys));
        match t.recv_event().unwrap() {
            RdpEvent::Bitmap(rects) => {
                assert_eq!(rects[0].dest_left, 7);
                assert_eq!(rects[0].dest_top, 8);
            }
            other => panic!("expected fast-path bitmap, got {other:?}"),
        }
    }

    #[test]
    fn send_input_writes_fastpath_pdu() {
        use crate::input::InputEvent;

        let mut t = RdpTransport::new(MockStream::new(Vec::new()));
        // Unencrypted (no session): key press + release.
        t.send_input(&[InputEvent::key_press(0x1E), InputEvent::key_release(0x1E)])
            .unwrap();
        let out = t.into_inner().outbound;
        // Header: action 0, numberEvents 2 in bits 2-5 → 0x08.
        assert_eq!(out[0], 0x08);
        // length byte = total = 1 (header) + 1 (length) + 4 (two 2-byte events).
        assert_eq!(out[1], 6);
        // First event: scancode header 0x00, keycode 0x1E.
        assert_eq!(&out[2..4], &[0x00, 0x1E]);
        // Second event: RELEASE flag (0x01), keycode 0x1E.
        assert_eq!(&out[4..6], &[0x01, 0x1E]);
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
    fn establish_requests_and_maps_extra_channel() {
        // Same handshake as above, but the client also asks for DRDYNVC and
        // the server grants it a channel id; establish() should surface that
        // mapping on the returned session.
        use crate::capabilities::{CapabilitySet, DemandActive, GeneralCapabilitySet};
        use crate::gcc::{
            ServerCoreData, ServerNetworkData, ServerSecurityData, ENCRYPTION_METHOD_128BIT,
        };
        use crate::license::{LicenseErrorMessage, LicensePdu};
        use crate::mcs::DomainParameters;
        use crate::security::{derive_session_keys, SessionKeys, RANDOM_LEN};

        let client_random = [0x5Au8; RANDOM_LEN];
        let server_random = [0xA5u8; RANDOM_LEN];
        let client_keys =
            derive_session_keys(&client_random, &server_random, ENCRYPTION_METHOD_128BIT);
        let server_keys = SessionKeys {
            mac_key: client_keys.mac_key.clone(),
            encrypt_key: client_keys.decrypt_key.clone(),
            decrypt_key: client_keys.encrypt_key.clone(),
        };
        let mut server_session = Rc4Session::new(&server_keys);

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

        // The server grants the one requested extra channel as id 1004.
        let server_blocks = vec![
            UserDataBlock::ServerCore(ServerCoreData {
                version: 0x0008_0004,
                client_requested_protocols: Some(0),
                early_capability_flags: None,
            }),
            UserDataBlock::ServerNetwork(ServerNetworkData {
                io_channel_id: 1003,
                channel_ids: vec![1004],
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
        let license = LicensePdu::ErrorAlert(LicenseErrorMessage::valid_client())
            .to_vec()
            .unwrap();
        let license_wrapped =
            security::wrap_pdu(Some(&mut server_session), SEC_LICENSE_PKT, &license);
        let demand = DemandActive {
            share_id: 0x0000_1234,
            source_descriptor: b"RDP\0".to_vec(),
            capability_sets: vec![CapabilitySet::General(GeneralCapabilitySet::default())],
            session_id: 0,
        };
        let demand_bytes = demand.encode(1002).unwrap();
        let demand_wrapped = security::wrap_pdu(Some(&mut server_session), 0, &demand_bytes);

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
        inbound.extend(framed(
            X224::data(&connect_response.to_vec()).to_vec().unwrap(),
        ));
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
        // User channel, I/O channel, then the extra DRDYNVC channel.
        inbound.extend(confirm(1007, 1007));
        inbound.extend(confirm(1003, 1003));
        inbound.extend(confirm(1004, 1004));
        inbound.extend(data_ind(&license_wrapped));
        inbound.extend(data_ind(&demand_wrapped));

        let mut t = RdpTransport::new(MockStream::new(inbound));
        let mut config = EstablishConfig::new(1024, 768, "CORP", "alice", "secret");
        config.extra_channels.push(ChannelDef {
            name: crate::dvc::DRDYNVC_CHANNEL_NAME.to_string(),
            options: 0,
        });
        let session = t.establish(&config, &client_random).unwrap();

        assert_eq!(
            session.channel_id(crate::dvc::DRDYNVC_CHANNEL_NAME),
            Some(1004)
        );
        assert_eq!(session.channel_id("not-requested"), None);
    }

    #[test]
    fn recv_event_reassembles_channel_data_across_chunks() {
        // Once the I/O channel is known, traffic on any other channel is
        // virtual-channel data (MS-RDPBCGR 2.2.6.1), reassembled and surfaced
        // as RdpEvent::ChannelData only once the last chunk arrives.
        let message = b"a dynamic-channel payload that needs no encryption";
        let chunks = crate::vchan::chunk(message, 16);
        assert!(chunks.len() > 1, "test needs a fragmented message");

        let data_ind = |payload: &[u8]| -> Vec<u8> {
            framed(
                X224::data(
                    &DomainPdu::SendDataIndication {
                        initiator: 1002,
                        channel_id: 1004, // not the I/O channel
                        user_data: payload,
                    }
                    .to_vec()
                    .unwrap(),
                )
                .to_vec()
                .unwrap(),
            )
        };

        // Every Send Data Indication carries a Basic Security Header, even
        // with no session active (flags = 0, no SEC_ENCRYPT).
        let mut inbound = Vec::new();
        for c in &chunks {
            let wrapped = security::wrap_pdu(None, 0, c);
            inbound.extend(data_ind(&wrapped));
        }
        let mut t = RdpTransport::new(MockStream::new(inbound));
        t.io_channel = Some(1003);

        match t.recv_event().unwrap() {
            RdpEvent::ChannelData { channel_id, data } => {
                assert_eq!(channel_id, 1004);
                assert_eq!(data, message);
            }
            other => panic!("expected ChannelData, got {other:?}"),
        }
    }

    #[test]
    fn establish_enhanced_drives_tls_handshake() {
        // The TLS path: no Security Exchange, no RC4. Licensing carries a
        // SEC_LICENSE_PKT header; the Demand Active and everything after are
        // bare Share Control PDUs.
        use crate::capabilities::{CapabilitySet, DemandActive, GeneralCapabilitySet};
        use crate::gcc::{ServerCoreData, ServerNetworkData};
        use crate::license::{LicenseErrorMessage, LicensePdu};
        use crate::mcs::DomainParameters;

        // Server security block advertises no RDP encryption (TLS handles it).
        let server_blocks = vec![
            UserDataBlock::ServerCore(ServerCoreData {
                version: 0x0008_0004,
                client_requested_protocols: Some(SecurityProtocols::SSL.0),
                early_capability_flags: None,
            }),
            UserDataBlock::ServerNetwork(ServerNetworkData {
                io_channel_id: 1003,
                channel_ids: vec![],
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

        // Licensing PDU: valid-client alert under a plaintext SEC_LICENSE_PKT
        // header (no encryption).
        let license = LicensePdu::ErrorAlert(LicenseErrorMessage::valid_client())
            .to_vec()
            .unwrap();
        let license_wrapped = security::wrap_pdu(None, SEC_LICENSE_PKT, &license);

        // Demand Active: a bare Share Control PDU (no security header).
        let demand = DemandActive {
            share_id: 0x0000_1234,
            source_descriptor: b"RDP\0".to_vec(),
            capability_sets: vec![CapabilitySet::General(GeneralCapabilitySet::default())],
            session_id: 0,
        };
        let demand_bytes = demand.encode(1002).unwrap();

        let mut inbound = Vec::new();
        // No Connection Confirm here: negotiation already happened on raw TCP.
        // 1. MCS Connect-Response.
        inbound.extend(framed(
            X224::data(&connect_response.to_vec()).to_vec().unwrap(),
        ));
        // 2. Attach User Confirm.
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
        // 3/4. Channel join confirms.
        inbound.extend(confirm(1007, 1007));
        inbound.extend(confirm(1003, 1003));
        // 5. Licensing, then 6. Demand Active (headerless).
        inbound.extend(data_ind(&license_wrapped));
        inbound.extend(data_ind(&demand_bytes));

        let mut t = RdpTransport::new_enhanced(MockStream::new(inbound));
        let config = EstablishConfig::new(1024, 768, "CORP", "alice", "secret");
        let session = t
            .establish_enhanced(&config, SecurityProtocols::SSL)
            .unwrap();

        assert_eq!(session.user_id, 1007);
        assert_eq!(session.io_channel, 1003);
        assert_eq!(session.share_id, 0x0000_1234);
        assert_eq!(session.server_channel, 1002);

        // The client sent no Security Exchange PDU; the Client Info rode under
        // a plaintext SEC_INFO_PKT header (no SEC_ENCRYPT).
        let out = t.into_inner().outbound;
        let mut info_seen = false;
        for user_data in split_send_data_requests(&out) {
            if user_data.len() < 4 {
                continue;
            }
            let flags = u16::from_le_bytes([user_data[0], user_data[1]]);
            // No PDU carries a Security Exchange header under TLS.
            assert_eq!(flags & crate::security::SEC_EXCHANGE_PKT, 0);
            if flags & SEC_INFO_PKT != 0 {
                info_seen = true;
                // Client Info is in the clear under TLS.
                assert_eq!(flags & crate::security::SEC_ENCRYPT, 0);
            }
        }
        assert!(info_seen, "Client Info PDU was not sent");
    }

    /// Split a client's outbound byte stream into the user-data payloads of
    /// each MCS Send Data Request it carries.
    fn split_send_data_requests(mut out: &[u8]) -> Vec<Vec<u8>> {
        let mut payloads = Vec::new();
        while out.len() >= TPKT_HEADER_LEN {
            let Some(total) = Tpkt::peek_total_len(&out[..TPKT_HEADER_LEN]).unwrap() else {
                break;
            };
            if total > out.len() {
                break;
            }
            let (packet, rest) = out.split_at(total);
            out = rest;
            let tpkt = Tpkt::decode(packet).unwrap();
            if let Ok(X224::Data(mcs)) = X224::decode(tpkt.payload) {
                if let Ok(DomainPdu::SendDataRequest { user_data, .. }) = DomainPdu::decode(mcs) {
                    payloads.push(user_data.to_vec());
                }
            }
        }
        payloads
    }

    #[test]
    fn establish_enhanced_bitmap_has_no_security_header() {
        // After an enhanced-mode session, a bitmap update arrives as a bare
        // Share Data PDU with no Basic Security Header.
        use crate::output::{BitmapData, UpdatePdu};

        let update = UpdatePdu::Bitmap(vec![BitmapData::uncompressed(
            2,
            3,
            1,
            1,
            16,
            vec![0x00, 0xF8],
        )]);
        let share = update.encode(0x1234, 1002).unwrap();
        let indication = X224::data(
            &DomainPdu::SendDataIndication {
                initiator: 1002,
                channel_id: 1003,
                user_data: &share,
            }
            .to_vec()
            .unwrap(),
        )
        .to_vec()
        .unwrap();

        let mut t = RdpTransport::new_enhanced(MockStream::new(framed(indication)));
        match t.recv_event().unwrap() {
            RdpEvent::Bitmap(rects) => {
                assert_eq!(rects.len(), 1);
                assert_eq!(rects[0].dest_left, 2);
                assert_eq!(rects[0].dest_top, 3);
            }
            other => panic!("expected Bitmap, got {other:?}"),
        }
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
