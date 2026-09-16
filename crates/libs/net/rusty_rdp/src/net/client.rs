//! The CLIENT-driving half of the connection sequence: [`RdpTransport::establish`]
//! / [`RdpTransport::establish_enhanced`] and everything only they reach —
//! X.224 negotiation, MCS connect/channel-join, the security exchange, and
//! the encrypted Client Info / activation wait. Framing primitives shared
//! with the server side live in [`super::framing`].

use super::*;

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

impl<S: Read + Write> RdpTransport<S> {
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
    pub(super) fn send_share(
        &mut self,
        user_id: u16,
        channel_id: u16,
        share: &[u8],
    ) -> io::Result<()> {
        let mut session = self.session.take();
        let result = match session.as_mut() {
            Some(s) => self.send_secure(Some(s), user_id, channel_id, 0, share),
            None => self.send_data(user_id, channel_id, share),
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
}
