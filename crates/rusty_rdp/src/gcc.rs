//! GCC (ITU-T T.124) Conference Create exchange and the RDP settings blocks.
//!
//! The opaque `user_data` carried inside the MCS [`ConnectInitial`] /
//! [`ConnectResponse`] ([`crate::mcs`]) is a T.124 *Conference Create
//! Request* / *Response*: a small PER-encoded envelope wrapping a sequence of
//! typed RDP settings blocks.
//!
//! ```text
//! ConferenceCreateRequest  (PER, "Duca" key)  ── client ──▶
//!     └─ TS_UD_CS_CORE / CS_SECURITY / CS_NET / CS_CLUSTER
//!                          ◀── ConferenceCreateResponse (PER, "McDn" key)
//!                                 └─ TS_UD_SC_CORE / SC_SECURITY / SC_NET
//! ```
//!
//! Each settings block is `TS_UD_HEADER { type: u16, length: u16 }` (the
//! length includes the four header bytes) followed by a little-endian body.
//! This module models the block types RDP relies on and preserves anything
//! else as [`UserDataBlock::Unknown`].
//!
//! [`ConnectInitial`]: crate::mcs::ConnectInitial
//! [`ConnectResponse`]: crate::mcs::ConnectResponse

use crate::cursor::{Reader, Writer};
use crate::error::{Error, Result};
use crate::per;

/// The T.124 object identifier `{0 0 20 124 0 1}` shared by both directions.
const T124_OID: [u8; 6] = [0, 0, 20, 124, 0, 1];
/// H.221 non-standard key for client-to-server user data ("Duca").
const H221_CLIENT_KEY: &[u8; 4] = b"Duca";
/// H.221 non-standard key for server-to-client user data ("McDn").
const H221_SERVER_KEY: &[u8; 4] = b"McDn";

/// Fixed byte count of a Conference Create body preceding the user data
/// octet string, used to size the `connectPDU` length.
const CCR_FIXED_OVERHEAD: usize = 14;

// TS_UD_HEADER block types (MS-RDPBCGR 2.2.1.3 / 2.2.1.4).
const CS_CORE: u16 = 0xC001;
const CS_SECURITY: u16 = 0xC002;
const CS_NET: u16 = 0xC003;
const CS_CLUSTER: u16 = 0xC004;
const SC_CORE: u16 = 0x0C01;
const SC_SECURITY: u16 = 0x0C02;
const SC_NET: u16 = 0x0C03;

/// RDP 5.0+ version tag for the core data `version` field.
pub const RDP_VERSION_5_PLUS: u32 = 0x0008_0004;
/// `colorDepth` value `RNS_UD_COLOR_8BPP`.
pub const RNS_UD_COLOR_8BPP: u16 = 0xCA01;
/// Default `SASSequence` value `RNS_UD_SAS_DEL`.
pub const RNS_UD_SAS_DEL: u16 = 0xAA03;

/// `encryptionMethods` flag: 40-bit RC4.
pub const ENCRYPTION_METHOD_40BIT: u32 = 0x0000_0001;
/// `encryptionMethods` flag: 128-bit RC4.
pub const ENCRYPTION_METHOD_128BIT: u32 = 0x0000_0002;
/// `encryptionMethods` flag: 56-bit RC4.
pub const ENCRYPTION_METHOD_56BIT: u32 = 0x0000_0008;
/// `encryptionMethods` flag: FIPS 140-1.
pub const ENCRYPTION_METHOD_FIPS: u32 = 0x0000_0010;

/// `encryptionLevel` value: all data protected using the maximum key
/// strength both client and server support.
pub const ENCRYPTION_LEVEL_CLIENT_COMPATIBLE: u32 = 0x0000_0002;

// ---------------------------------------------------------------------------
// Conference Create Request / Response envelope
// ---------------------------------------------------------------------------

/// Encode a client's Conference Create Request wrapping the concatenated
/// client settings blocks in `user_data`.
pub fn encode_conference_create_request(user_data: &[u8]) -> Result<Vec<u8>> {
    let mut w = Writer::new();
    // ConnectData: Key = object identifier.
    per::write_choice(&mut w, 0);
    per::write_object_identifier(&mut w, &T124_OID)?;
    // ConnectData::connectPDU as an OCTET STRING length.
    per::write_length(&mut w, user_data.len() + CCR_FIXED_OVERHEAD)?;
    // ConferenceCreateRequest
    per::write_choice(&mut w, 0);
    per::write_selection(&mut w, 0x08);
    per::write_numeric_string(&mut w, b"1", 1)?; // ConferenceName::numeric = "1"
    per::write_padding(&mut w, 1);
    // UserData SET OF: one element, h221NonStandard.
    per::write_number_of_sets(&mut w, 1);
    per::write_choice(&mut w, 0xC0);
    per::write_octet_string(&mut w, H221_CLIENT_KEY, 4)?;
    per::write_octet_string(&mut w, user_data, 0)?;
    Ok(w.into_vec())
}

/// Decode a Conference Create Request, returning the wrapped client settings
/// blocks.
pub fn decode_conference_create_request(buf: &[u8]) -> Result<Vec<u8>> {
    let mut r = Reader::new(buf);
    per::read_choice(&mut r)?;
    expect_t124_oid(&mut r)?;
    let _connect_pdu_len = per::read_length(&mut r)?;
    per::read_choice(&mut r)?;
    per::read_selection(&mut r)?;
    per::read_numeric_string(&mut r, 1)?;
    per::read_padding(&mut r, 1)?;
    per::read_number_of_sets(&mut r)?;
    per::read_choice(&mut r)?;
    expect_h221_key(&mut r, H221_CLIENT_KEY)?;
    Ok(per::read_octet_string(&mut r, 0)?.to_vec())
}

/// Encode a server's Conference Create Response wrapping the server settings
/// blocks in `user_data`. `node_id` is the server's MCS node (UserId).
pub fn encode_conference_create_response(node_id: u16, user_data: &[u8]) -> Result<Vec<u8>> {
    let mut w = Writer::new();
    per::write_choice(&mut w, 0);
    per::write_object_identifier(&mut w, &T124_OID)?;
    per::write_length(&mut w, user_data.len() + CCR_FIXED_OVERHEAD)?;
    // ConferenceCreateResponse
    per::write_choice(&mut w, 0x14);
    per::write_integer16(&mut w, node_id, 1001)?; // nodeID
    per::write_integer(&mut w, 1)?; // tag
    per::write_enumerated(&mut w, 0); // result = success
    per::write_number_of_sets(&mut w, 1);
    per::write_choice(&mut w, 0xC0);
    per::write_octet_string(&mut w, H221_SERVER_KEY, 4)?;
    per::write_octet_string(&mut w, user_data, 0)?;
    Ok(w.into_vec())
}

/// Decode a Conference Create Response, returning `(node_id, server settings
/// blocks)`.
pub fn decode_conference_create_response(buf: &[u8]) -> Result<(u16, Vec<u8>)> {
    let mut r = Reader::new(buf);
    per::read_choice(&mut r)?;
    expect_t124_oid(&mut r)?;
    let _connect_pdu_len = per::read_length(&mut r)?;
    per::read_choice(&mut r)?;
    let node_id = per::read_integer16(&mut r, 1001)?;
    let _tag = per::read_integer(&mut r)?;
    let _result = per::read_enumerated(&mut r)?;
    per::read_number_of_sets(&mut r)?;
    per::read_choice(&mut r)?;
    expect_h221_key(&mut r, H221_SERVER_KEY)?;
    let user_data = per::read_octet_string(&mut r, 0)?.to_vec();
    Ok((node_id, user_data))
}

fn expect_t124_oid(r: &mut Reader<'_>) -> Result<()> {
    let oid = per::read_object_identifier(r)?;
    if oid != T124_OID {
        return Err(Error::InvalidValue {
            field: "GCC t124 OID",
            value: format!("{oid:?}"),
        });
    }
    Ok(())
}

fn expect_h221_key(r: &mut Reader<'_>, key: &[u8; 4]) -> Result<()> {
    let actual = per::read_octet_string(r, 4)?;
    if actual != key {
        return Err(Error::InvalidValue {
            field: "GCC H.221 key",
            value: format!("{actual:02X?}"),
        });
    }
    Ok(())
}

// ---------------------------------------------------------------------------
// User data blocks
// ---------------------------------------------------------------------------

/// One `TS_UD_*` settings block from the Conference Create user data.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum UserDataBlock {
    /// `TS_UD_CS_CORE` — client core settings.
    ClientCore(ClientCoreData),
    /// `TS_UD_CS_SEC` — client security settings.
    ClientSecurity(ClientSecurityData),
    /// `TS_UD_CS_NET` — client virtual channel definitions.
    ClientNetwork(ClientNetworkData),
    /// `TS_UD_CS_CLUSTER` — client cluster/redirection settings.
    ClientCluster(ClientClusterData),
    /// `TS_UD_SC_CORE` — server core settings.
    ServerCore(ServerCoreData),
    /// `TS_UD_SC_SEC1` — server security settings (crypto + certificate).
    ServerSecurity(ServerSecurityData),
    /// `TS_UD_SC_NET` — server channel assignments.
    ServerNetwork(ServerNetworkData),
    /// Any block type this crate does not yet model, kept verbatim.
    Unknown {
        /// The `TS_UD_HEADER` type field.
        block_type: u16,
        /// The raw block body (without the four header bytes).
        data: Vec<u8>,
    },
}

impl UserDataBlock {
    /// Encode this block, including its `TS_UD_HEADER`, into `w`.
    pub fn encode(&self, w: &mut Writer) -> Result<()> {
        let (block_type, body) = match self {
            UserDataBlock::ClientCore(d) => (CS_CORE, d.encode_body()),
            UserDataBlock::ClientSecurity(d) => (CS_SECURITY, d.encode_body()),
            UserDataBlock::ClientNetwork(d) => (CS_NET, d.encode_body()),
            UserDataBlock::ClientCluster(d) => (CS_CLUSTER, d.encode_body()),
            UserDataBlock::ServerCore(d) => (SC_CORE, d.encode_body()),
            UserDataBlock::ServerSecurity(d) => (SC_SECURITY, d.encode_body()),
            UserDataBlock::ServerNetwork(d) => (SC_NET, d.encode_body()),
            UserDataBlock::Unknown { block_type, data } => (*block_type, data.clone()),
        };
        write_block_header(w, block_type, body.len())?;
        w.write_bytes(&body);
        Ok(())
    }

    fn parse(block_type: u16, body: &[u8]) -> Result<UserDataBlock> {
        Ok(match block_type {
            CS_CORE => UserDataBlock::ClientCore(ClientCoreData::decode(body)?),
            CS_SECURITY => UserDataBlock::ClientSecurity(ClientSecurityData::decode(body)?),
            CS_NET => UserDataBlock::ClientNetwork(ClientNetworkData::decode(body)?),
            CS_CLUSTER => UserDataBlock::ClientCluster(ClientClusterData::decode(body)?),
            SC_CORE => UserDataBlock::ServerCore(ServerCoreData::decode(body)?),
            SC_SECURITY => UserDataBlock::ServerSecurity(ServerSecurityData::decode(body)?),
            SC_NET => UserDataBlock::ServerNetwork(ServerNetworkData::decode(body)?),
            other => UserDataBlock::Unknown {
                block_type: other,
                data: body.to_vec(),
            },
        })
    }
}

/// Encode a sequence of settings blocks into a single user-data buffer.
pub fn encode_user_data(blocks: &[UserDataBlock]) -> Result<Vec<u8>> {
    let mut w = Writer::new();
    for block in blocks {
        block.encode(&mut w)?;
    }
    Ok(w.into_vec())
}

/// Parse a user-data buffer into its settings blocks.
pub fn parse_user_data(buf: &[u8]) -> Result<Vec<UserDataBlock>> {
    let mut r = Reader::new(buf);
    let mut blocks = Vec::new();
    while r.remaining() >= 4 {
        let block_type = r.read_u16_le()?;
        let length = r.read_u16_le()? as usize;
        if length < 4 {
            return Err(Error::InvalidLength {
                field: "TS_UD_HEADER length",
                length,
            });
        }
        let body = r.read_bytes(length - 4)?;
        blocks.push(UserDataBlock::parse(block_type, body)?);
    }
    Ok(blocks)
}

fn write_block_header(w: &mut Writer, block_type: u16, body_len: usize) -> Result<()> {
    let total = body_len + 4;
    if total > u16::MAX as usize {
        return Err(Error::Overflow {
            field: "TS_UD_HEADER length",
        });
    }
    w.write_u16_le(block_type);
    w.write_u16_le(total as u16);
    Ok(())
}

// ---------------------------------------------------------------------------
// TS_UD_CS_CORE
// ---------------------------------------------------------------------------

/// `TS_UD_CS_CORE` (MS-RDPBCGR 2.2.1.3.2): the client's core settings.
///
/// The trailing optional fields are a contiguous prefix: on decode they are
/// read while bytes remain, and on encode they are written in order up to the
/// first `None`.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ClientCoreData {
    /// Supported RDP version (`RDP_VERSION_5_PLUS`).
    pub version: u32,
    /// Requested desktop width in pixels.
    pub desktop_width: u16,
    /// Requested desktop height in pixels.
    pub desktop_height: u16,
    /// Legacy `colorDepth` (`RNS_UD_COLOR_*`).
    pub color_depth: u16,
    /// Secure access sequence (`RNS_UD_SAS_DEL`).
    pub sas_sequence: u16,
    /// Keyboard layout (locale id, e.g. `0x0409`).
    pub keyboard_layout: u32,
    /// Client build number.
    pub client_build: u32,
    /// Client host name (stored as up to 15 characters).
    pub client_name: String,
    /// Keyboard type.
    pub keyboard_type: u32,
    /// Keyboard subtype.
    pub keyboard_subtype: u32,
    /// Number of function keys.
    pub keyboard_function_key: u32,
    /// IME file name (usually empty).
    pub ime_file_name: String,
    /// Optional `postBeta2ColorDepth`.
    pub post_beta2_color_depth: Option<u16>,
    /// Optional `clientProductId`.
    pub client_product_id: Option<u16>,
    /// Optional `serialNumber`.
    pub serial_number: Option<u32>,
    /// Optional `highColorDepth`.
    pub high_color_depth: Option<u16>,
    /// Optional `supportedColorDepths` bitmask.
    pub supported_color_depths: Option<u16>,
    /// Optional `earlyCapabilityFlags`.
    pub early_capability_flags: Option<u16>,
    /// Optional `clientDigProductId` (up to 31 characters).
    pub client_dig_product_id: Option<String>,
    /// Optional `connectionType`.
    pub connection_type: Option<u8>,
    /// Optional `serverSelectedProtocol` (echoes the X.224 negotiation).
    pub server_selected_protocol: Option<u32>,
}

impl ClientCoreData {
    /// Build core data for the given desktop size with conventional defaults
    /// (RDP 5+, US keyboard, 8bpp legacy / 16bpp high color).
    pub fn new(desktop_width: u16, desktop_height: u16, client_name: &str) -> Self {
        ClientCoreData {
            version: RDP_VERSION_5_PLUS,
            desktop_width,
            desktop_height,
            color_depth: RNS_UD_COLOR_8BPP,
            sas_sequence: RNS_UD_SAS_DEL,
            keyboard_layout: 0x0409,
            client_build: 2600,
            client_name: client_name.to_string(),
            keyboard_type: 4,
            keyboard_subtype: 0,
            keyboard_function_key: 12,
            ime_file_name: String::new(),
            post_beta2_color_depth: Some(RNS_UD_COLOR_8BPP),
            client_product_id: Some(1),
            serial_number: Some(0),
            high_color_depth: Some(0x0010),
            supported_color_depths: Some(0x000F),
            early_capability_flags: Some(0x0001),
            client_dig_product_id: Some(String::new()),
            connection_type: Some(0),
            server_selected_protocol: Some(0),
        }
    }

    fn encode_body(&self) -> Vec<u8> {
        let mut w = Writer::new();
        w.write_u32_le(self.version);
        w.write_u16_le(self.desktop_width);
        w.write_u16_le(self.desktop_height);
        w.write_u16_le(self.color_depth);
        w.write_u16_le(self.sas_sequence);
        w.write_u32_le(self.keyboard_layout);
        w.write_u32_le(self.client_build);
        write_utf16le_fixed(&mut w, &self.client_name, 32);
        w.write_u32_le(self.keyboard_type);
        w.write_u32_le(self.keyboard_subtype);
        w.write_u32_le(self.keyboard_function_key);
        write_utf16le_fixed(&mut w, &self.ime_file_name, 64);

        // Optional tail — contiguous prefix, stop at the first absent field.
        'opt: {
            let Some(v) = self.post_beta2_color_depth else {
                break 'opt;
            };
            w.write_u16_le(v);
            let Some(v) = self.client_product_id else {
                break 'opt;
            };
            w.write_u16_le(v);
            let Some(v) = self.serial_number else {
                break 'opt;
            };
            w.write_u32_le(v);
            let Some(v) = self.high_color_depth else {
                break 'opt;
            };
            w.write_u16_le(v);
            let Some(v) = self.supported_color_depths else {
                break 'opt;
            };
            w.write_u16_le(v);
            let Some(v) = self.early_capability_flags else {
                break 'opt;
            };
            w.write_u16_le(v);
            let Some(id) = &self.client_dig_product_id else {
                break 'opt;
            };
            write_utf16le_fixed(&mut w, id, 64);
            let Some(v) = self.connection_type else {
                break 'opt;
            };
            w.write_u8(v);
            w.write_u8(0); // pad1octet
            let Some(v) = self.server_selected_protocol else {
                break 'opt;
            };
            w.write_u32_le(v);
        }
        w.into_vec()
    }

    fn decode(body: &[u8]) -> Result<ClientCoreData> {
        let mut r = Reader::new(body);
        let version = r.read_u32_le()?;
        let desktop_width = r.read_u16_le()?;
        let desktop_height = r.read_u16_le()?;
        let color_depth = r.read_u16_le()?;
        let sas_sequence = r.read_u16_le()?;
        let keyboard_layout = r.read_u32_le()?;
        let client_build = r.read_u32_le()?;
        let client_name = read_utf16le_fixed(r.read_bytes(32)?);
        let keyboard_type = r.read_u32_le()?;
        let keyboard_subtype = r.read_u32_le()?;
        let keyboard_function_key = r.read_u32_le()?;
        let ime_file_name = read_utf16le_fixed(r.read_bytes(64)?);

        let post_beta2_color_depth = opt_u16(&mut r)?;
        let client_product_id = opt_u16(&mut r)?;
        let serial_number = opt_u32(&mut r)?;
        let high_color_depth = opt_u16(&mut r)?;
        let supported_color_depths = opt_u16(&mut r)?;
        let early_capability_flags = opt_u16(&mut r)?;
        let client_dig_product_id = if r.remaining() >= 64 {
            Some(read_utf16le_fixed(r.read_bytes(64)?))
        } else {
            None
        };
        let connection_type = if r.remaining() >= 1 {
            let ct = r.read_u8()?;
            let _pad = if r.remaining() >= 1 { r.read_u8()? } else { 0 };
            Some(ct)
        } else {
            None
        };
        let server_selected_protocol = opt_u32(&mut r)?;

        Ok(ClientCoreData {
            version,
            desktop_width,
            desktop_height,
            color_depth,
            sas_sequence,
            keyboard_layout,
            client_build,
            client_name,
            keyboard_type,
            keyboard_subtype,
            keyboard_function_key,
            ime_file_name,
            post_beta2_color_depth,
            client_product_id,
            serial_number,
            high_color_depth,
            supported_color_depths,
            early_capability_flags,
            client_dig_product_id,
            connection_type,
            server_selected_protocol,
        })
    }
}

// ---------------------------------------------------------------------------
// TS_UD_CS_SEC
// ---------------------------------------------------------------------------

/// `TS_UD_CS_SEC` (MS-RDPBCGR 2.2.1.3.3): the client's security settings.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct ClientSecurityData {
    /// Supported `encryptionMethods` bitmask.
    pub encryption_methods: u32,
    /// `extEncryptionMethods` (French-locale variant, usually 0).
    pub ext_encryption_methods: u32,
}

impl ClientSecurityData {
    fn encode_body(&self) -> Vec<u8> {
        let mut w = Writer::new();
        w.write_u32_le(self.encryption_methods);
        w.write_u32_le(self.ext_encryption_methods);
        w.into_vec()
    }

    fn decode(body: &[u8]) -> Result<ClientSecurityData> {
        let mut r = Reader::new(body);
        Ok(ClientSecurityData {
            encryption_methods: r.read_u32_le()?,
            ext_encryption_methods: r.read_u32_le()?,
        })
    }
}

// ---------------------------------------------------------------------------
// TS_UD_CS_NET
// ---------------------------------------------------------------------------

// CHANNEL_OPTION_* flags (MS-RDPBCGR 2.2.1.3.4.1).
/// The channel is initialized (always set in practice; required by the spec).
pub const CHANNEL_OPTION_INITIALIZED: u32 = 0x8000_0000;
/// Traffic on this channel should be encrypted when standard RDP security is
/// in use.
pub const CHANNEL_OPTION_ENCRYPT_RDP: u32 = 0x4000_0000;
/// Server-to-client traffic on this channel should be encrypted.
pub const CHANNEL_OPTION_ENCRYPT_SC: u32 = 0x2000_0000;
/// Client-to-server traffic on this channel should be encrypted.
pub const CHANNEL_OPTION_ENCRYPT_CS: u32 = 0x1000_0000;
/// High priority bandwidth class.
pub const CHANNEL_OPTION_PRI_HIGH: u32 = 0x0800_0000;
/// Medium priority bandwidth class.
pub const CHANNEL_OPTION_PRI_MED: u32 = 0x0400_0000;
/// Low priority bandwidth class.
pub const CHANNEL_OPTION_PRI_LOW: u32 = 0x0200_0000;
/// The channel's data should be compressed.
pub const CHANNEL_OPTION_COMPRESS_RDP: u32 = 0x0080_0000;

/// A single virtual-channel definition in `TS_UD_CS_NET`.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ChannelDef {
    /// Channel name (up to 8 bytes ASCII, e.g. "rdpdr", "cliprdr").
    pub name: String,
    /// `CHANNEL_OPTION_*` flags.
    pub options: u32,
}

/// `TS_UD_CS_NET` (MS-RDPBCGR 2.2.1.3.4): requested virtual channels.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ClientNetworkData {
    /// The requested channels, in the order the server will assign IDs.
    pub channels: Vec<ChannelDef>,
}

impl ClientNetworkData {
    fn encode_body(&self) -> Vec<u8> {
        let mut w = Writer::new();
        w.write_u32_le(self.channels.len() as u32);
        for ch in &self.channels {
            let mut name = [0u8; 8];
            for (slot, byte) in name.iter_mut().zip(ch.name.bytes()) {
                *slot = byte;
            }
            w.write_bytes(&name);
            w.write_u32_le(ch.options);
        }
        w.into_vec()
    }

    fn decode(body: &[u8]) -> Result<ClientNetworkData> {
        let mut r = Reader::new(body);
        let count = r.read_u32_le()? as usize;
        let mut channels = Vec::with_capacity(count);
        for _ in 0..count {
            let name_bytes = r.read_bytes(8)?;
            let end = name_bytes.iter().position(|&b| b == 0).unwrap_or(8);
            let name = String::from_utf8_lossy(&name_bytes[..end]).into_owned();
            let options = r.read_u32_le()?;
            channels.push(ChannelDef { name, options });
        }
        Ok(ClientNetworkData { channels })
    }
}

// ---------------------------------------------------------------------------
// TS_UD_CS_CLUSTER
// ---------------------------------------------------------------------------

/// `TS_UD_CS_CLUSTER` (MS-RDPBCGR 2.2.1.3.5): cluster/redirection settings.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct ClientClusterData {
    /// `Flags` bitmask (redirection support, session-id validity).
    pub flags: u32,
    /// Redirected session id (0 when none).
    pub redirected_session_id: u32,
}

impl ClientClusterData {
    fn encode_body(&self) -> Vec<u8> {
        let mut w = Writer::new();
        w.write_u32_le(self.flags);
        w.write_u32_le(self.redirected_session_id);
        w.into_vec()
    }

    fn decode(body: &[u8]) -> Result<ClientClusterData> {
        let mut r = Reader::new(body);
        Ok(ClientClusterData {
            flags: r.read_u32_le()?,
            redirected_session_id: r.read_u32_le()?,
        })
    }
}

// ---------------------------------------------------------------------------
// TS_UD_SC_CORE
// ---------------------------------------------------------------------------

/// `TS_UD_SC_CORE` (MS-RDPBCGR 2.2.1.4.2): the server's core settings.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct ServerCoreData {
    /// Server RDP version.
    pub version: u32,
    /// Optional `clientRequestedProtocols` echo.
    pub client_requested_protocols: Option<u32>,
    /// Optional `earlyCapabilityFlags`.
    pub early_capability_flags: Option<u32>,
}

impl ServerCoreData {
    fn encode_body(&self) -> Vec<u8> {
        let mut w = Writer::new();
        w.write_u32_le(self.version);
        'opt: {
            let Some(v) = self.client_requested_protocols else {
                break 'opt;
            };
            w.write_u32_le(v);
            let Some(v) = self.early_capability_flags else {
                break 'opt;
            };
            w.write_u32_le(v);
        }
        w.into_vec()
    }

    fn decode(body: &[u8]) -> Result<ServerCoreData> {
        let mut r = Reader::new(body);
        let version = r.read_u32_le()?;
        let client_requested_protocols = opt_u32(&mut r)?;
        let early_capability_flags = opt_u32(&mut r)?;
        Ok(ServerCoreData {
            version,
            client_requested_protocols,
            early_capability_flags,
        })
    }
}

// ---------------------------------------------------------------------------
// TS_UD_SC_SEC1
// ---------------------------------------------------------------------------

/// `TS_UD_SC_SEC1` (MS-RDPBCGR 2.2.1.4.3): the server's security settings.
///
/// When both `encryption_method` and `encryption_level` are zero the server
/// omits the random and certificate, and both vectors are empty here. The
/// certificate is kept as an opaque blob for the security layer to parse.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ServerSecurityData {
    /// Selected `encryptionMethod`.
    pub encryption_method: u32,
    /// Selected `encryptionLevel`.
    pub encryption_level: u32,
    /// Server random used to derive session keys.
    pub server_random: Vec<u8>,
    /// Server certificate (opaque; parsed by the security layer).
    pub server_certificate: Vec<u8>,
}

impl ServerSecurityData {
    fn encode_body(&self) -> Vec<u8> {
        let mut w = Writer::new();
        w.write_u32_le(self.encryption_method);
        w.write_u32_le(self.encryption_level);
        if !self.server_random.is_empty() || !self.server_certificate.is_empty() {
            w.write_u32_le(self.server_random.len() as u32);
            w.write_u32_le(self.server_certificate.len() as u32);
            w.write_bytes(&self.server_random);
            w.write_bytes(&self.server_certificate);
        }
        w.into_vec()
    }

    fn decode(body: &[u8]) -> Result<ServerSecurityData> {
        let mut r = Reader::new(body);
        let encryption_method = r.read_u32_le()?;
        let encryption_level = r.read_u32_le()?;
        let (server_random, server_certificate) = if r.remaining() >= 8 {
            let random_len = r.read_u32_le()? as usize;
            let cert_len = r.read_u32_le()? as usize;
            let random = r.read_bytes(random_len)?.to_vec();
            let cert = r.read_bytes(cert_len)?.to_vec();
            (random, cert)
        } else {
            (Vec::new(), Vec::new())
        };
        Ok(ServerSecurityData {
            encryption_method,
            encryption_level,
            server_random,
            server_certificate,
        })
    }
}

// ---------------------------------------------------------------------------
// TS_UD_SC_NET
// ---------------------------------------------------------------------------

/// `TS_UD_SC_NET` (MS-RDPBCGR 2.2.1.4.4): the server's channel assignments.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ServerNetworkData {
    /// MCS channel id of the primary I/O channel.
    pub io_channel_id: u16,
    /// Assigned channel ids, one per client-requested virtual channel.
    pub channel_ids: Vec<u16>,
}

impl ServerNetworkData {
    fn encode_body(&self) -> Vec<u8> {
        let mut w = Writer::new();
        w.write_u16_le(self.io_channel_id);
        w.write_u16_le(self.channel_ids.len() as u16);
        for &id in &self.channel_ids {
            w.write_u16_le(id);
        }
        // Pad to a 4-byte boundary when the channel count is odd.
        if self.channel_ids.len() % 2 == 1 {
            w.write_u16_le(0);
        }
        w.into_vec()
    }

    fn decode(body: &[u8]) -> Result<ServerNetworkData> {
        let mut r = Reader::new(body);
        let io_channel_id = r.read_u16_le()?;
        let count = r.read_u16_le()? as usize;
        let mut channel_ids = Vec::with_capacity(count);
        for _ in 0..count {
            channel_ids.push(r.read_u16_le()?);
        }
        Ok(ServerNetworkData {
            io_channel_id,
            channel_ids,
        })
    }
}

// ---------------------------------------------------------------------------
// Helpers
// ---------------------------------------------------------------------------

fn opt_u16(r: &mut Reader<'_>) -> Result<Option<u16>> {
    if r.remaining() >= 2 {
        Ok(Some(r.read_u16_le()?))
    } else {
        Ok(None)
    }
}

fn opt_u32(r: &mut Reader<'_>) -> Result<Option<u32>> {
    if r.remaining() >= 4 {
        Ok(Some(r.read_u32_le()?))
    } else {
        Ok(None)
    }
}

/// Write `s` as UTF-16LE into a fixed `byte_len`-byte field, NUL-terminated
/// and zero-padded (truncating if necessary to leave room for the NUL).
fn write_utf16le_fixed(w: &mut Writer, s: &str, byte_len: usize) {
    let max_units = byte_len / 2;
    let mut written = 0usize;
    for unit in s.encode_utf16() {
        if written / 2 >= max_units.saturating_sub(1) {
            break; // leave the final unit for the NUL terminator
        }
        w.write_u16_le(unit);
        written += 2;
    }
    while written < byte_len {
        w.write_u8(0);
        written += 1;
    }
}

/// Read a UTF-16LE string from a fixed-size field, stopping at the first NUL.
fn read_utf16le_fixed(bytes: &[u8]) -> String {
    let mut units = Vec::with_capacity(bytes.len() / 2);
    let mut i = 0;
    while i + 1 < bytes.len() {
        let unit = u16::from_le_bytes([bytes[i], bytes[i + 1]]);
        if unit == 0 {
            break;
        }
        units.push(unit);
        i += 2;
    }
    String::from_utf16_lossy(&units)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn ccr_prefix_matches_wire() {
        // With < 128 bytes of user data the lengths are single-byte, but RDP
        // never sends that little; use a padded blob to exercise the 2-byte
        // connectPDU length like a real client.
        let user_data = vec![0xEE; 200];
        let bytes = encode_conference_create_request(&user_data).unwrap();
        // ConnectData Key OID prefix is fixed and well-known.
        assert_eq!(&bytes[..7], &[0x00, 0x05, 0x00, 0x14, 0x7C, 0x00, 0x01]);
        // The "Duca" H.221 key appears verbatim.
        assert!(bytes.windows(4).any(|w| w == b"Duca"));
        let round = decode_conference_create_request(&bytes).unwrap();
        assert_eq!(round, user_data);
    }

    #[test]
    fn ccrsp_roundtrip() {
        let user_data = vec![0x11; 64];
        let bytes = encode_conference_create_response(1002, &user_data).unwrap();
        assert!(bytes.windows(4).any(|w| w == b"McDn"));
        let (node_id, round) = decode_conference_create_response(&bytes).unwrap();
        assert_eq!(node_id, 1002);
        assert_eq!(round, user_data);
    }

    #[test]
    fn client_core_roundtrip() {
        let core = ClientCoreData::new(1920, 1080, "rusty");
        let block = UserDataBlock::ClientCore(core);
        let mut w = Writer::new();
        block.encode(&mut w).unwrap();
        let parsed = parse_user_data(w.as_slice()).unwrap();
        assert_eq!(parsed, vec![block]);
    }

    #[test]
    fn client_core_header_and_name() {
        let core = ClientCoreData::new(1024, 768, "PC");
        let mut w = Writer::new();
        UserDataBlock::ClientCore(core).encode(&mut w).unwrap();
        let bytes = w.into_vec();
        // TS_UD_HEADER: type 0xC001, then length.
        assert_eq!(&bytes[..2], &[0x01, 0xC0]);
        // version field follows the 4-byte header.
        assert_eq!(&bytes[4..8], &RDP_VERSION_5_PLUS.to_le_bytes());
        // desktopWidth = 1024 = 0x0400.
        assert_eq!(&bytes[8..10], &[0x00, 0x04]);
    }

    #[test]
    fn client_network_channels_roundtrip() {
        let net = ClientNetworkData {
            channels: vec![
                ChannelDef {
                    name: "rdpdr".to_string(),
                    options: 0x8000_0000,
                },
                ChannelDef {
                    name: "cliprdr".to_string(),
                    options: 0xC000_0000,
                },
            ],
        };
        let block = UserDataBlock::ClientNetwork(net);
        let bytes = encode_user_data(std::slice::from_ref(&block)).unwrap();
        assert_eq!(parse_user_data(&bytes).unwrap(), vec![block]);
    }

    #[test]
    fn server_security_with_certificate() {
        let sec = ServerSecurityData {
            encryption_method: ENCRYPTION_METHOD_128BIT,
            encryption_level: 2,
            server_random: vec![0xAB; 32],
            server_certificate: vec![0xCD; 44],
        };
        let block = UserDataBlock::ServerSecurity(sec);
        let bytes = encode_user_data(std::slice::from_ref(&block)).unwrap();
        assert_eq!(parse_user_data(&bytes).unwrap(), vec![block]);
    }

    #[test]
    fn server_security_no_encryption_omits_random() {
        let sec = ServerSecurityData {
            encryption_method: 0,
            encryption_level: 0,
            server_random: Vec::new(),
            server_certificate: Vec::new(),
        };
        let body = sec.encode_body();
        assert_eq!(body.len(), 8); // method + level only
        let block = UserDataBlock::ServerSecurity(sec);
        let bytes = encode_user_data(std::slice::from_ref(&block)).unwrap();
        assert_eq!(parse_user_data(&bytes).unwrap(), vec![block]);
    }

    #[test]
    fn server_network_odd_count_is_padded() {
        let net = ServerNetworkData {
            io_channel_id: 1003,
            channel_ids: vec![1004],
        };
        let body = net.encode_body();
        // io(2) + count(2) + id(2) + pad(2) = 8 bytes.
        assert_eq!(body.len(), 8);
        let block = UserDataBlock::ServerNetwork(net);
        let bytes = encode_user_data(std::slice::from_ref(&block)).unwrap();
        assert_eq!(parse_user_data(&bytes).unwrap(), vec![block]);
    }

    #[test]
    fn full_client_user_data_set() {
        let blocks = vec![
            UserDataBlock::ClientCore(ClientCoreData::new(1280, 720, "rusty-rdp")),
            UserDataBlock::ClientSecurity(ClientSecurityData {
                encryption_methods: ENCRYPTION_METHOD_128BIT | ENCRYPTION_METHOD_40BIT,
                ext_encryption_methods: 0,
            }),
            UserDataBlock::ClientCluster(ClientClusterData {
                flags: 0x0D,
                redirected_session_id: 0,
            }),
        ];
        let user_data = encode_user_data(&blocks).unwrap();
        // Wrap and unwrap through the GCC envelope.
        let ccr = encode_conference_create_request(&user_data).unwrap();
        let unwrapped = decode_conference_create_request(&ccr).unwrap();
        assert_eq!(parse_user_data(&unwrapped).unwrap(), blocks);
    }

    #[test]
    fn unknown_block_preserved() {
        let mut w = Writer::new();
        // Unknown type 0xFFFF with a 3-byte body.
        w.write_u16_le(0xFFFF);
        w.write_u16_le(7);
        w.write_bytes(&[0x01, 0x02, 0x03]);
        let bytes = w.into_vec();
        let parsed = parse_user_data(&bytes).unwrap();
        assert_eq!(
            parsed,
            vec![UserDataBlock::Unknown {
                block_type: 0xFFFF,
                data: vec![0x01, 0x02, 0x03],
            }]
        );
        // Re-encoding reproduces the input.
        assert_eq!(encode_user_data(&parsed).unwrap(), bytes);
    }
}
