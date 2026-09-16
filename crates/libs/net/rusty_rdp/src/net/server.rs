//! The SERVER-driving half of the connection sequence: [`RdpTransport::accept`]
//! and everything only it reaches — Connection Confirm, the GCC/MCS
//! Connect-Response, channel-join confirmation, Client Info receipt, the
//! "no license required" response, and Demand/Confirm Active. Framing
//! primitives shared with the client side live in [`super::framing`].

use super::*;

/// Server-side encryption parameters for [`RdpTransport::accept`]. Supply
/// this (via [`AcceptConfig::encryption`]) to negotiate real encrypted
/// standard RDP security instead of `encryptionLevel = 0`.
#[derive(Debug, Clone)]
pub struct AcceptEncryption {
    /// The server's RSA public key, embedded in the certificate
    /// [`accept`](RdpTransport::accept) sends the client (signed with
    /// [`crate::security::ts_signing_key`] via
    /// [`crate::security::encode_proprietary_certificate`]).
    pub public_key: RsaPublicKey,
    /// The matching private key, used to decrypt the client's Security
    /// Exchange PDU.
    pub private_key: RsaPrivateKey,
    /// 32 bytes of server randomness, mixed into session-key derivation
    /// alongside the client's random. Caller-supplied, like
    /// [`RdpTransport::establish`]'s `client_random` — this crate does not
    /// generate randomness itself.
    pub server_random: [u8; RANDOM_LEN],
}

/// Settings for [`RdpTransport::accept`].
#[derive(Debug, Clone)]
pub struct AcceptConfig {
    /// Desktop width in pixels, advertised in the Demand Active PDU.
    pub desktop_width: u16,
    /// Desktop height in pixels, advertised in the Demand Active PDU.
    pub desktop_height: u16,
    /// Session color depth (bits per pixel), advertised in the Demand
    /// Active PDU.
    pub bits_per_pixel: u16,
    /// `sourceDescriptor` advertised in the Demand Active PDU.
    pub source_descriptor: Vec<u8>,
    /// When `Some`, drive real encrypted standard security (RSA exchange +
    /// RC4) instead of `encryptionLevel = 0`. `None` (the default from
    /// [`AcceptConfig::new`]) keeps the original unencrypted-only behavior.
    pub encryption: Option<AcceptEncryption>,
}

impl AcceptConfig {
    /// Build a config for the given desktop size, defaulting to 16bpp, the
    /// conventional `b"RDP\0"` source descriptor, and no encryption.
    pub fn new(width: u16, height: u16) -> Self {
        AcceptConfig {
            desktop_width: width,
            desktop_height: height,
            bits_per_pixel: 16,
            source_descriptor: b"RDP\0".to_vec(),
            encryption: None,
        }
    }
}

/// A client accepted by [`RdpTransport::accept`].
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct AcceptedClient {
    /// The `UserId` [`RdpTransport::accept`] assigned the client.
    pub user_id: u16,
    /// The MCS I/O channel carrying the main RDP data.
    pub io_channel: u16,
    /// The share id this server assigned in its Demand Active PDU.
    pub share_id: u32,
    /// MCS channel ids granted to the client's requested static virtual
    /// channels, keyed by channel name.
    pub channels: HashMap<String, u16>,
    /// The client's logon data (domain/user/password). Encrypted in transit
    /// only when [`AcceptConfig::encryption`] was set; with no encryption
    /// configured this travelled the wire in the clear — do not use
    /// `accept` unencrypted over an untrusted network.
    pub client_info: ClientInfo,
}

impl<S: Read + Write> RdpTransport<S> {
    // --- Server-side connection sequence (RdpTransport::accept) ----------

    /// The pseudo `UserId` [`RdpTransport::accept`] uses as the server's own
    /// MCS identity when it originates a Send Data Indication or a Share
    /// Control/Data PDU — distinct from any client `UserId`, matching the
    /// convention real servers use (and this module's tests already assume).
    const SERVER_MCS_ID: u16 = MCS_GLOBAL_CHANNEL_ID - 1;

    /// The `UserId` [`RdpTransport::accept`] assigns the (single) client it
    /// accepts.
    const ACCEPTED_CLIENT_USER_ID: u16 = MCS_BASE_CHANNEL_ID + 6;

    /// The share id [`RdpTransport::accept`] assigns in its Demand Active
    /// PDU.
    const ACCEPTED_SHARE_ID: u32 = 0x0001_00EA;

    /// Read a Connection Request and reply with a Connection Confirm.
    /// `accept` only speaks standard RDP security, so this always selects
    /// [`SecurityProtocols::RDP`] regardless of what the client offered.
    fn accept_negotiate(&mut self) -> io::Result<()> {
        let request = self.read_tpkt()?;
        let pdu = match X224::decode(&request).map_err(to_io)? {
            X224::ConnectionRequest(pdu) => pdu,
            other => {
                return Err(protocol_error(format!(
                    "expected Connection Request, got {other:?}"
                )))
            }
        };
        let negotiation = match pdu.negotiation {
            Some(Negotiation::Request { .. }) => Some(Negotiation::Response {
                flags: 0,
                selected: SecurityProtocols::RDP,
            }),
            // A client that skipped negotiation entirely gets a bare
            // Connection Confirm; `negotiate()` treats that as standard RDP.
            _ => None,
        };
        let confirm = X224::ConnectionConfirm(ConnectionPdu {
            negotiation,
            ..Default::default()
        })
        .to_vec()
        .map_err(to_io)?;
        self.write_tpkt(&confirm)
    }

    /// Read a Connection Request and reply selecting `protocol` if the
    /// client offered it, else reply with an `RDP_NEG_FAILURE` (`code`) and
    /// return an error. Shared by [`RdpTransport::accept_negotiate_ssl`] and
    /// [`RdpTransport::accept_negotiate_hybrid`].
    #[cfg(feature = "tls")]
    fn accept_negotiate_select(
        &mut self,
        protocol: SecurityProtocols,
        code: NegFailureCode,
        unmet_msg: &str,
    ) -> io::Result<()> {
        let request = self.read_tpkt()?;
        let pdu = match X224::decode(&request).map_err(to_io)? {
            X224::ConnectionRequest(pdu) => pdu,
            other => {
                return Err(protocol_error(format!(
                    "expected Connection Request, got {other:?}"
                )))
            }
        };
        let offered = match pdu.negotiation {
            Some(Negotiation::Request { protocols, .. }) => protocols,
            _ => SecurityProtocols::RDP,
        };
        if !offered.contains(protocol) {
            let failure = X224::ConnectionConfirm(ConnectionPdu {
                negotiation: Some(Negotiation::Failure { code }),
                ..Default::default()
            })
            .to_vec()
            .map_err(to_io)?;
            self.write_tpkt(&failure)?;
            return Err(protocol_error(unmet_msg));
        }
        let confirm = X224::ConnectionConfirm(ConnectionPdu {
            negotiation: Some(Negotiation::Response {
                flags: 0,
                selected: protocol,
            }),
            ..Default::default()
        })
        .to_vec()
        .map_err(to_io)?;
        self.write_tpkt(&confirm)
    }

    /// Read a Connection Request and reply selecting
    /// [`SecurityProtocols::SSL`], for a TLS-upgrading server entry point
    /// (see `crate::tls::accept_tls`). Errors (after telling the client via
    /// an `RDP_NEG_FAILURE`) if the client didn't offer TLS.
    #[cfg(feature = "tls")]
    pub(crate) fn accept_negotiate_ssl(&mut self) -> io::Result<()> {
        self.accept_negotiate_select(
            SecurityProtocols::SSL,
            NegFailureCode::SslRequiredByServer,
            "client did not offer TLS security, but accept_tls requires it",
        )
    }

    /// Read a Connection Request and reply selecting
    /// [`SecurityProtocols::HYBRID`], for a CredSSP/NLA-accepting server
    /// entry point (see `crate::tls::accept_tls_nla`). Errors (after telling
    /// the client via an `RDP_NEG_FAILURE`) if the client didn't offer it.
    ///
    /// `HYBRID_EX`'s Early User Authorization Result PDU is not sent — this
    /// selects (and `accept_tls_nla` only speaks) the base `HYBRID`
    /// protocol.
    #[cfg(feature = "tls")]
    pub(crate) fn accept_negotiate_hybrid(&mut self) -> io::Result<()> {
        self.accept_negotiate_select(
            SecurityProtocols::HYBRID,
            NegFailureCode::HybridRequiredByServer,
            "client did not offer CredSSP/NLA, but accept_tls_nla requires it",
        )
    }

    /// Read the client's `Connect-Initial` and return its decoded GCC
    /// settings blocks.
    fn accept_read_connect_initial(&mut self) -> io::Result<Vec<UserDataBlock>> {
        let payload = self.read_x224_data()?;
        let connect_initial = ConnectInitial::decode(&payload).map_err(to_io)?;
        let client_ud =
            gcc::decode_conference_create_request(&connect_initial.user_data).map_err(to_io)?;
        gcc::parse_user_data(&client_ud).map_err(to_io)
    }

    /// Send a `Connect-Response` wrapping `server_blocks`.
    fn accept_send_connect_response(&mut self, server_blocks: &[UserDataBlock]) -> io::Result<()> {
        let user_data = gcc::encode_user_data(server_blocks).map_err(to_io)?;
        let ccrsp = gcc::encode_conference_create_response(Self::SERVER_MCS_ID, &user_data)
            .map_err(to_io)?;
        let response = ConnectResponse {
            result: McsResult::Successful,
            called_connect_id: 0,
            domain_parameters: DomainParameters::client_target(),
            user_data: ccrsp,
        };
        self.write_x224_data(&response.to_vec())
    }

    /// Consume the client's `ErectDomainRequest`, answer its
    /// `AttachUserRequest` with the assigned `UserId`, then answer each of
    /// the `2 + granted_channels.len()` `ChannelJoinRequest`s (the client's
    /// own user channel, the I/O channel, and each granted virtual channel,
    /// in that order — the sequence [`join_all_channels`](Self::join_all_channels)
    /// drives client-side) with a `ChannelJoinConfirm`. Returns the assigned
    /// `UserId`.
    fn accept_join_channels(
        &mut self,
        io_channel: u16,
        granted_channels: &[u16],
    ) -> io::Result<u16> {
        let erect = self.read_x224_data()?;
        match DomainPdu::decode(&erect).map_err(to_io)? {
            DomainPdu::ErectDomainRequest { .. } => {}
            other => {
                return Err(protocol_error(format!(
                    "expected Erect Domain Request, got {other:?}"
                )))
            }
        }

        let attach = self.read_x224_data()?;
        match DomainPdu::decode(&attach).map_err(to_io)? {
            DomainPdu::AttachUserRequest => {}
            other => {
                return Err(protocol_error(format!(
                    "expected Attach User Request, got {other:?}"
                )))
            }
        }
        let user_id = Self::ACCEPTED_CLIENT_USER_ID;
        let attach_confirm = DomainPdu::AttachUserConfirm {
            result: McsResult::Successful,
            initiator: Some(user_id),
        }
        .to_vec()
        .map_err(to_io)?;
        self.write_x224_data(&attach_confirm)?;

        let expected_joins = 2 + granted_channels.len();
        for _ in 0..expected_joins {
            let req = self.read_x224_data()?;
            let (initiator, channel_id) = match DomainPdu::decode(&req).map_err(to_io)? {
                DomainPdu::ChannelJoinRequest {
                    initiator,
                    channel_id,
                } => (initiator, channel_id),
                other => {
                    return Err(protocol_error(format!(
                        "expected Channel Join Request, got {other:?}"
                    )))
                }
            };
            let join_confirm = DomainPdu::ChannelJoinConfirm {
                result: McsResult::Successful,
                initiator,
                requested: channel_id,
                channel_id: Some(channel_id),
            }
            .to_vec()
            .map_err(to_io)?;
            self.write_x224_data(&join_confirm)?;
        }

        self.io_channel = Some(io_channel);
        Ok(user_id)
    }

    /// Send `data` on `channel_id` as a Send Data Indication from
    /// `initiator` — the server-role counterpart of
    /// [`send_data`](Self::send_data).
    fn send_data_indication(
        &mut self,
        initiator: u16,
        channel_id: u16,
        data: &[u8],
    ) -> io::Result<()> {
        let pdu = DomainPdu::SendDataIndication {
            initiator,
            channel_id,
            user_data: data,
        }
        .to_vec()
        .map_err(to_io)?;
        self.write_x224_data(&pdu)
    }

    /// Receive one Send Data Request, returning `(initiator, channel_id,
    /// data)` — the server-role counterpart of
    /// [`recv_data`](Self::recv_data).
    fn recv_data_request(&mut self) -> io::Result<(u16, u16, Vec<u8>)> {
        let response = self.read_x224_data()?;
        match DomainPdu::decode(&response).map_err(to_io)? {
            DomainPdu::SendDataRequest {
                initiator,
                channel_id,
                user_data,
            } => Ok((initiator, channel_id, user_data.to_vec())),
            other => Err(protocol_error(format!(
                "expected Send Data Request, got {other:?}"
            ))),
        }
    }

    /// Send `payload` on `channel_id` as a Send Data Indication from
    /// `initiator`, under a Basic Security Header, encrypting with the
    /// stored session when active — the server-role counterpart of
    /// [`send_share`](Self::send_share).
    fn send_secure_indication(
        &mut self,
        initiator: u16,
        channel_id: u16,
        base_flags: u16,
        payload: &[u8],
    ) -> io::Result<()> {
        let mut session = self.session.take();
        let wrapped = security::wrap_pdu(session.as_mut(), base_flags, payload);
        self.session = session;
        self.send_data_indication(initiator, channel_id, &wrapped)
    }

    /// Receive one Send Data Request and unwrap its Basic Security Header,
    /// decrypting with the stored session when active. Returns
    /// `(initiator, channel_id, security_flags, body)`.
    fn recv_secure_request(&mut self) -> io::Result<(u16, u16, u16, Vec<u8>)> {
        let (initiator, channel_id, data) = self.recv_data_request()?;
        let mut session = self.session.take();
        let result = security::unwrap_pdu(session.as_mut(), &data).map_err(to_io);
        self.session = session;
        let (flags, body) = result?;
        Ok((initiator, channel_id, flags, body))
    }

    /// Receive the client's Security Exchange PDU, decrypt the client
    /// random with `private_key`, derive the session keys alongside
    /// `server_random`, and store the resulting [`Rc4Session`] — the
    /// server-role counterpart of
    /// [`security_exchange`](Self::security_exchange). Never encrypted
    /// itself, per MS-RDPBCGR.
    fn accept_security_exchange(
        &mut self,
        private_key: &RsaPrivateKey,
        server_random: &[u8; RANDOM_LEN],
        encryption_method: u32,
    ) -> io::Result<()> {
        let (_initiator, _channel, data) = self.recv_data_request()?;
        let padded = security::decode_security_exchange(&data).map_err(to_io)?;
        let key_len = private_key.key_length();
        if padded.len() < key_len {
            return Err(protocol_error(format!(
                "Security Exchange PDU is {} bytes, shorter than the {key_len}-byte RSA key",
                padded.len()
            )));
        }
        let plain = private_key.decrypt(&padded[..key_len]).map_err(to_io)?;
        if plain.len() < RANDOM_LEN {
            return Err(protocol_error(format!(
                "decrypted client random is {} bytes, expected at least {RANDOM_LEN}",
                plain.len()
            )));
        }
        let mut client_random = [0u8; RANDOM_LEN];
        client_random.copy_from_slice(&plain[..RANDOM_LEN]);
        let keys = derive_session_keys(&client_random, server_random, encryption_method);
        self.session = Some(Rc4Session::new_server(&keys));
        Ok(())
    }

    /// Read the Client Info PDU and decode the client's logon data,
    /// decrypting with the stored session when encryption is active.
    fn accept_client_info(&mut self) -> io::Result<ClientInfo> {
        let (_initiator, _channel, _flags, body) = self.recv_secure_request()?;
        ClientInfo::decode(&body).map_err(to_io)
    }

    /// Send the "no license required" License Error Message. Always carries
    /// a Basic Security Header (encrypted when a session is active), like
    /// the Client Info PDU.
    fn accept_send_no_license_required(&mut self, io_channel: u16) -> io::Result<()> {
        let license = LicensePdu::ErrorAlert(LicenseErrorMessage::valid_client())
            .to_vec()
            .map_err(to_io)?;
        self.send_secure_indication(Self::SERVER_MCS_ID, io_channel, SEC_LICENSE_PKT, &license)
    }

    /// Send `payload` on `channel_id` as a Send Data Indication from
    /// `initiator`, wrapping it in a Basic Security Header and encrypting
    /// it only when the stored session is active — the server-role
    /// counterpart of [`send_share`](Self::send_share). Share Control/Data
    /// PDUs (Demand Active, Confirm Active, finalization) use this: unlike
    /// Client Info and licensing, they carry no header at all when
    /// unencrypted.
    fn send_share_indication(
        &mut self,
        initiator: u16,
        channel_id: u16,
        payload: &[u8],
    ) -> io::Result<()> {
        let mut session = self.session.take();
        let result = match session.as_mut() {
            Some(s) => {
                let wrapped = security::wrap_pdu(Some(s), 0, payload);
                self.send_data_indication(initiator, channel_id, &wrapped)
            }
            None => self.send_data_indication(initiator, channel_id, payload),
        };
        self.session = session;
        result
    }

    /// Receive one Send Data Request, stripping and decrypting its Basic
    /// Security Header only when the stored session is active — the
    /// receive-side counterpart of [`send_share_indication`](Self::send_share_indication).
    fn recv_share_request(&mut self) -> io::Result<Vec<u8>> {
        let (_initiator, _channel, data) = self.recv_data_request()?;
        let mut session = self.session.take();
        let result = match session.as_mut() {
            Some(s) => security::unwrap_pdu(Some(s), &data)
                .map(|(_flags, body)| body)
                .map_err(to_io),
            None => Ok(data),
        };
        self.session = session;
        result
    }

    /// Send the Demand Active PDU and read back the client's Confirm
    /// Active. Returns `(share_id, client_channel)`, where `client_channel`
    /// is the `pduSource` the client used — needed to target
    /// [`server_finalization_sequence`].
    fn accept_capability_exchange(
        &mut self,
        config: &AcceptConfig,
        io_channel: u16,
    ) -> io::Result<(u32, u16)> {
        let share_id = Self::ACCEPTED_SHARE_ID;
        let demand = DemandActive {
            share_id,
            source_descriptor: config.source_descriptor.clone(),
            capability_sets: server_capability_sets(
                config.desktop_width,
                config.desktop_height,
                config.bits_per_pixel,
            ),
            session_id: 0,
        };
        let demand_bytes = demand.encode(Self::SERVER_MCS_ID).map_err(to_io)?;
        self.send_share_indication(Self::SERVER_MCS_ID, io_channel, &demand_bytes)?;

        let body = self.recv_share_request()?;
        let (client_channel, confirm) = ConfirmActive::decode(&body).map_err(to_io)?;
        if confirm.share_id != share_id {
            return Err(protocol_error(format!(
                "Confirm Active echoed share id {:#x}, expected {share_id:#x}",
                confirm.share_id
            )));
        }
        Ok((share_id, client_channel))
    }

    /// Read the client's four-PDU finalization sequence and reply with the
    /// server's.
    fn accept_finalization(
        &mut self,
        share_id: u32,
        io_channel: u16,
        client_channel: u16,
    ) -> io::Result<()> {
        for _ in 0..4 {
            let body = self.recv_share_request()?;
            FinalizationPdu::decode(&body).map_err(to_io)?;
        }
        for pdu in server_finalization_sequence(client_channel) {
            let bytes = pdu.encode(share_id, Self::SERVER_MCS_ID).map_err(to_io)?;
            self.send_share_indication(Self::SERVER_MCS_ID, io_channel, &bytes)?;
        }
        Ok(())
    }

    /// Drive the entire standard-RDP connection sequence as the *server* and
    /// return the accepted client.
    ///
    /// Performs, in order: the X.224 negotiation (always selecting standard
    /// RDP security), the GCC/MCS connect, channel setup, the RSA security
    /// exchange and session-key derivation (when `config.encryption` is
    /// set), the Client Info PDU, the "no license required" response, the
    /// capability exchange (Demand → Confirm Active), and the server's
    /// connection-finalization sequence.
    ///
    /// With `config.encryption` left `None`, `accept` speaks only
    /// **unencrypted** standard RDP security (`encryptionLevel = 0`): no RSA
    /// key exchange, no RC4. Setting it drives real encrypted standard
    /// security instead — see [`AcceptEncryption`]. For TLS-upgraded
    /// connections, use `crate::tls::accept_tls` instead (which negotiates
    /// [`SecurityProtocols::SSL`] and calls the shared post-negotiation logic
    /// this method also uses); leave `config.encryption` as `None` there too,
    /// since TLS already provides confidentiality. CredSSP/NLA server-side
    /// validation is not implemented. Do not use the unencrypted mode over an
    /// untrusted network — see this crate's security note on
    /// [`crate::security`]/[`crate::crypto`].
    pub fn accept(&mut self, config: &AcceptConfig) -> io::Result<AcceptedClient> {
        self.accept_negotiate()?;
        self.accept_after_negotiate(config)
    }

    /// Everything [`RdpTransport::accept`] does after the X.224 negotiation:
    /// GCC/MCS connect, channel setup, the RSA security exchange (when
    /// `config.encryption` is set), the Client Info PDU, licensing, the
    /// capability exchange, and connection finalization.
    ///
    /// Shared with `crate::tls::accept_tls`, which negotiates
    /// [`SecurityProtocols::SSL`] itself (via `accept_negotiate_ssl`) and
    /// then re-wraps the stream in TLS before calling this. Under TLS,
    /// `config.encryption` should stay `None`: the resulting all-zero
    /// `encryptionLevel`/`encryptionMethod`
    /// server security data is exactly what MS-RDPBCGR requires when
    /// Enhanced RDP Security supplies confidentiality, and every
    /// session-conditional framing helper below (`send_secure_indication`,
    /// `send_share_indication`, ...) already does the right thing since
    /// `self.session` naturally stays `None`.
    pub(crate) fn accept_after_negotiate(
        &mut self,
        config: &AcceptConfig,
    ) -> io::Result<AcceptedClient> {
        let client_blocks = self.accept_read_connect_initial()?;
        let requested_channels: Vec<ChannelDef> = client_blocks
            .iter()
            .find_map(|b| match b {
                UserDataBlock::ClientNetwork(net) => Some(net.channels.clone()),
                _ => None,
            })
            .unwrap_or_default();
        let io_channel = MCS_GLOBAL_CHANNEL_ID;
        let granted_ids: Vec<u16> = (0..requested_channels.len() as u16)
            .map(|i| io_channel + 1 + i)
            .collect();

        let encryption_method = config.encryption.as_ref().map(|_| {
            let client_methods = client_blocks
                .iter()
                .find_map(|b| match b {
                    UserDataBlock::ClientSecurity(d) => Some(d.encryption_methods),
                    _ => None,
                })
                .unwrap_or(0);
            if client_methods & ENCRYPTION_METHOD_128BIT != 0 {
                ENCRYPTION_METHOD_128BIT
            } else if client_methods & ENCRYPTION_METHOD_56BIT != 0 {
                ENCRYPTION_METHOD_56BIT
            } else {
                // Falls back to 40-bit even if the client didn't offer it
                // (matching a permissive real server); enc.public_key still
                // ends up in the certificate either way.
                ENCRYPTION_METHOD_40BIT
            }
        });
        let server_security = match (&config.encryption, encryption_method) {
            (Some(enc), Some(method)) => ServerSecurityData {
                encryption_method: method,
                encryption_level: ENCRYPTION_LEVEL_CLIENT_COMPATIBLE,
                server_random: enc.server_random.to_vec(),
                server_certificate: security::encode_proprietary_certificate(&enc.public_key)
                    .map_err(to_io)?,
            },
            _ => ServerSecurityData {
                encryption_method: 0,
                encryption_level: 0,
                server_random: Vec::new(),
                server_certificate: Vec::new(),
            },
        };

        let server_blocks = vec![
            UserDataBlock::ServerCore(ServerCoreData {
                version: RDP_VERSION_5_PLUS,
                client_requested_protocols: Some(0),
                early_capability_flags: Some(0),
            }),
            UserDataBlock::ServerSecurity(server_security),
            UserDataBlock::ServerNetwork(ServerNetworkData {
                io_channel_id: io_channel,
                channel_ids: granted_ids.clone(),
            }),
        ];
        self.accept_send_connect_response(&server_blocks)?;

        let user_id = self.accept_join_channels(io_channel, &granted_ids)?;
        if let (Some(enc), Some(method)) = (&config.encryption, encryption_method) {
            self.accept_security_exchange(&enc.private_key, &enc.server_random, method)?;
        }
        let channels: HashMap<String, u16> = requested_channels
            .iter()
            .zip(&granted_ids)
            .map(|(def, &id)| (def.name.clone(), id))
            .collect();

        let client_info = self.accept_client_info()?;
        self.accept_send_no_license_required(io_channel)?;

        let (share_id, client_channel) = self.accept_capability_exchange(config, io_channel)?;
        self.accept_finalization(share_id, io_channel, client_channel)?;

        Ok(AcceptedClient {
            user_id,
            io_channel,
            share_id,
            channels,
            client_info,
        })
    }
}
