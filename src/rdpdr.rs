//! Device Redirection (MS-RDPEFS), std-only.
//!
//! Device redirection (drives, printers, ports, smart cards) rides on the
//! static virtual channel named `"rdpdr"` (registered via
//! [`crate::net::EstablishConfig::extra_channels`] and framed by
//! [`crate::vchan`], like [`crate::cliprdr`]/[`crate::rdpsnd`]). Unlike
//! those protocols' `CLIPRDR_HEADER`/`SNDPROLOG`, [`RdpdrHeader`] carries
//! no length field of its own — a PDU's boundary is whatever one
//! `vchan`-reassembled message the channel delivered, so every `decode`
//! here consumes its entire input buffer.
//!
//! ## Initialization sequence (MS-RDPEFS 1.3.2)
//!
//! 1. Server sends [`ServerAnnounceRequestPdu`].
//! 2. Client sends [`ClientAnnounceReplyPdu`], then [`ClientNameRequestPdu`].
//! 3. Server sends [`ServerClientIdConfirmPdu`].
//! 4. Server sends [`ServerCoreCapabilityPdu`]; client replies with
//!    [`ClientCoreCapabilityPdu`].
//! 5. Client sends [`ClientDeviceListAnnouncePdu`], listing the devices
//!    it's redirecting; the server answers each with a
//!    [`ServerDeviceAnnounceResponsePdu`].
//! 6. Server may send [`ServerUserLoggedOnPdu`] once the user session is up.
//!
//! ## What's implemented
//!
//! The full core initialization handshake and capability negotiation:
//! [`RdpdrHeader`], [`ServerAnnounceRequestPdu`], [`ClientAnnounceReplyPdu`],
//! [`ServerClientIdConfirmPdu`], [`ClientNameRequestPdu`],
//! [`ServerCoreCapabilityPdu`]/[`ClientCoreCapabilityPdu`] (with
//! [`GeneralCapsSet`] typed, other capability types preserved raw),
//! [`ClientDeviceListAnnouncePdu`]/[`DeviceAnnounce`],
//! [`ServerDeviceAnnounceResponsePdu`], and [`ServerUserLoggedOnPdu`].
//!
//! The Device I/O Request/Response exchange (`PAKID_CORE_DEVICE_IOREQUEST`/
//! `IOCOMPLETION`, section 2.2.1.4/2.2.1.5) that carries the actual
//! file/port/smart-card operations, via the shared [`DeviceIoRequest`]/
//! [`DeviceIoResponse`] headers and the request/response pair for each of
//! the five major functions common to every redirected device type:
//! [`DeviceCreateRequestPdu`]/[`DeviceCreateResponsePdu`] (`IRP_MJ_CREATE`),
//! [`DeviceCloseRequestPdu`]/[`DeviceCloseResponsePdu`] (`IRP_MJ_CLOSE`),
//! [`DeviceReadRequestPdu`]/[`DeviceReadResponsePdu`] (`IRP_MJ_READ`),
//! [`DeviceWriteRequestPdu`]/[`DeviceWriteResponsePdu`] (`IRP_MJ_WRITE`), and
//! [`DeviceControlRequestPdu`]/[`DeviceControlResponsePdu`]
//! (`IRP_MJ_DEVICE_CONTROL`, the generic IOCTL/FSCTL carrier smart-card and
//! port redirection ride on).
//!
//! **Not yet implemented:** the filesystem-specific major functions —
//! `IRP_MJ_QUERY_INFORMATION`/`SET_INFORMATION`,
//! `IRP_MJ_QUERY_VOLUME_INFORMATION`/`SET_VOLUME_INFORMATION`,
//! `IRP_MJ_DIRECTORY_CONTROL` (directory listing/change notification), and
//! `IRP_MJ_LOCK_CONTROL` — and `PAKID_CORE_DEVICELIST_REMOVE`.

use crate::cursor::{Reader, Writer};
use crate::error::{Error, Result};

/// The static virtual channel name device redirection registers under.
pub const RDPDR_CHANNEL_NAME: &str = "rdpdr";

// Component values (RDPDR_HEADER's Component field).
/// `RDPDR_CTYP_CORE` — the device redirector core component.
pub const RDPDR_CTYP_CORE: u16 = 0x4472;
/// `RDPDR_CTYP_PRN` — the printing component.
pub const RDPDR_CTYP_PRN: u16 = 0x5052;

// PacketId values (RDPDR_HEADER's PacketId field).
const PAKID_CORE_SERVER_ANNOUNCE: u16 = 0x496E;
const PAKID_CORE_CLIENTID_CONFIRM: u16 = 0x4343;
const PAKID_CORE_CLIENT_NAME: u16 = 0x434E;
const PAKID_CORE_DEVICELIST_ANNOUNCE: u16 = 0x4441;
const PAKID_CORE_DEVICE_REPLY: u16 = 0x6472;
const PAKID_CORE_SERVER_CAPABILITY: u16 = 0x5350;
const PAKID_CORE_CLIENT_CAPABILITY: u16 = 0x4350;
const PAKID_CORE_USER_LOGGEDON: u16 = 0x554C;
const PAKID_CORE_DEVICE_IOREQUEST: u16 = 0x4952;
const PAKID_CORE_DEVICE_IOCOMPLETION: u16 = 0x4943;

// MajorFunction values (DR_DEVICE_IOREQUEST's MajorFunction field).
/// `IRP_MJ_CREATE` — create/open request.
pub const IRP_MJ_CREATE: u32 = 0x0000_0000;
/// `IRP_MJ_CLOSE` — close request.
pub const IRP_MJ_CLOSE: u32 = 0x0000_0002;
/// `IRP_MJ_READ` — read request.
pub const IRP_MJ_READ: u32 = 0x0000_0003;
/// `IRP_MJ_WRITE` — write request.
pub const IRP_MJ_WRITE: u32 = 0x0000_0004;
/// `IRP_MJ_QUERY_INFORMATION` — query file information request. Not yet
/// implemented.
pub const IRP_MJ_QUERY_INFORMATION: u32 = 0x0000_0005;
/// `IRP_MJ_SET_INFORMATION` — set file information request. Not yet
/// implemented.
pub const IRP_MJ_SET_INFORMATION: u32 = 0x0000_0006;
/// `IRP_MJ_QUERY_VOLUME_INFORMATION` — query volume information request.
/// Not yet implemented.
pub const IRP_MJ_QUERY_VOLUME_INFORMATION: u32 = 0x0000_000A;
/// `IRP_MJ_SET_VOLUME_INFORMATION` — set volume information request. Not
/// yet implemented.
pub const IRP_MJ_SET_VOLUME_INFORMATION: u32 = 0x0000_000B;
/// `IRP_MJ_DIRECTORY_CONTROL` — directory control request (query
/// directory / notify change directory, distinguished by
/// [`DeviceIoRequest::minor_function`]). Not yet implemented.
pub const IRP_MJ_DIRECTORY_CONTROL: u32 = 0x0000_000C;
/// `IRP_MJ_DEVICE_CONTROL` — device control (IOCTL/FSCTL) request.
pub const IRP_MJ_DEVICE_CONTROL: u32 = 0x0000_000E;
/// `IRP_MJ_LOCK_CONTROL` — file lock control request. Not yet implemented.
pub const IRP_MJ_LOCK_CONTROL: u32 = 0x0000_0011;

// MinorFunction values, valid only when MajorFunction is
// IRP_MJ_DIRECTORY_CONTROL.
/// `IRP_MN_QUERY_DIRECTORY` — query directory request.
pub const IRP_MN_QUERY_DIRECTORY: u32 = 0x0000_0001;
/// `IRP_MN_NOTIFY_CHANGE_DIRECTORY` — notify change directory request.
pub const IRP_MN_NOTIFY_CHANGE_DIRECTORY: u32 = 0x0000_0002;

// DeviceType values (DEVICE_ANNOUNCE's DeviceType field).
/// `RDPDR_DTYP_SERIAL`.
pub const RDPDR_DTYP_SERIAL: u32 = 0x0000_0001;
/// `RDPDR_DTYP_PARALLEL`.
pub const RDPDR_DTYP_PARALLEL: u32 = 0x0000_0002;
/// `RDPDR_DTYP_PRINT`.
pub const RDPDR_DTYP_PRINT: u32 = 0x0000_0004;
/// `RDPDR_DTYP_FILESYSTEM`.
pub const RDPDR_DTYP_FILESYSTEM: u32 = 0x0000_0008;
/// `RDPDR_DTYP_SMARTCARD`.
pub const RDPDR_DTYP_SMARTCARD: u32 = 0x0000_0020;

/// `RDPDR_HEADER` — the 4-byte shared header at the start of every RDPDR
/// message. Unlike the other channel protocols in this crate, it carries
/// no length field: a PDU's extent is the whole buffer handed to a
/// `decode` function (one `vchan`-reassembled channel message).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct RdpdrHeader {
    /// `RDPDR_CTYP_CORE` or `RDPDR_CTYP_PRN`.
    pub component: u16,
    /// Identifies the packet's function.
    pub packet_id: u16,
}

impl RdpdrHeader {
    fn encode(&self, w: &mut Writer) {
        w.write_u16_le(self.component);
        w.write_u16_le(self.packet_id);
    }

    fn decode(r: &mut Reader<'_>) -> Result<RdpdrHeader> {
        Ok(RdpdrHeader {
            component: r.read_u16_le()?,
            packet_id: r.read_u16_le()?,
        })
    }
}

/// Peek the `(Component, PacketId)` of an encoded PDU without consuming it,
/// to pick the right decoder.
pub fn decode_header(buf: &[u8]) -> Result<RdpdrHeader> {
    let mut r = Reader::new(buf);
    RdpdrHeader::decode(&mut r)
}

fn wrap(packet_id: u16, body: &[u8]) -> Vec<u8> {
    let mut w = Writer::with_capacity(4 + body.len());
    RdpdrHeader {
        component: RDPDR_CTYP_CORE,
        packet_id,
    }
    .encode(&mut w);
    w.write_bytes(body);
    w.into_vec()
}

fn unwrap<'a>(buf: &'a [u8], expected_packet_id: u16) -> Result<Reader<'a>> {
    let mut r = Reader::new(buf);
    let header = RdpdrHeader::decode(&mut r)?;
    if header.component != RDPDR_CTYP_CORE {
        return Err(Error::InvalidValue {
            field: "RDPDR_HEADER Component",
            value: format!("0x{:04X}", header.component),
        });
    }
    if header.packet_id != expected_packet_id {
        return Err(Error::InvalidValue {
            field: "RDPDR_HEADER PacketId",
            value: format!(
                "0x{:04X} (expected 0x{:04X})",
                header.packet_id, expected_packet_id
            ),
        });
    }
    Ok(r)
}

/// `DR_CORE_SERVER_ANNOUNCE_REQ` — the first message of the protocol,
/// sent by the server.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct ServerAnnounceRequestPdu {
    /// MUST be `0x0001`.
    pub version_major: u16,
    /// Server minor version.
    pub version_minor: u16,
    /// A unique ID the server generates for this connection.
    pub client_id: u32,
}

impl ServerAnnounceRequestPdu {
    /// Encode to bytes.
    pub fn encode(&self) -> Vec<u8> {
        let mut body = Writer::new();
        body.write_u16_le(self.version_major);
        body.write_u16_le(self.version_minor);
        body.write_u32_le(self.client_id);
        wrap(PAKID_CORE_SERVER_ANNOUNCE, body.as_slice())
    }

    /// Decode from bytes.
    pub fn decode(buf: &[u8]) -> Result<ServerAnnounceRequestPdu> {
        let mut r = unwrap(buf, PAKID_CORE_SERVER_ANNOUNCE)?;
        Ok(ServerAnnounceRequestPdu {
            version_major: r.read_u16_le()?,
            version_minor: r.read_u16_le()?,
            client_id: r.read_u32_le()?,
        })
    }
}

/// `DR_CORE_CLIENT_ANNOUNCE_RSP` — the client's reply to
/// [`ServerAnnounceRequestPdu`].
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct ClientAnnounceReplyPdu {
    /// MUST be `0x0001`.
    pub version_major: u16,
    /// Client minor version.
    pub version_minor: u16,
    /// Echoes the server's `client_id`, or a client-chosen unique ID.
    pub client_id: u32,
}

impl ClientAnnounceReplyPdu {
    /// Encode to bytes.
    pub fn encode(&self) -> Vec<u8> {
        let mut body = Writer::new();
        body.write_u16_le(self.version_major);
        body.write_u16_le(self.version_minor);
        body.write_u32_le(self.client_id);
        wrap(PAKID_CORE_CLIENTID_CONFIRM, body.as_slice())
    }

    /// Decode from bytes.
    pub fn decode(buf: &[u8]) -> Result<ClientAnnounceReplyPdu> {
        let mut r = unwrap(buf, PAKID_CORE_CLIENTID_CONFIRM)?;
        Ok(ClientAnnounceReplyPdu {
            version_major: r.read_u16_le()?,
            version_minor: r.read_u16_le()?,
            client_id: r.read_u32_le()?,
        })
    }
}

/// `DR_CORE_SERVER_CLIENTID_CONFIRM` — the server's confirmation of the
/// `client_id` from [`ClientAnnounceReplyPdu`]. Identical wire shape to
/// that PDU (both use `PAKID_CORE_CLIENTID_CONFIRM`); which one a given
/// buffer is follows from the connection role, not the bytes.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct ServerClientIdConfirmPdu {
    /// MUST be `0x0001`.
    pub version_major: u16,
    /// Server minor version.
    pub version_minor: u16,
    /// Confirms the `client_id` sent by the client.
    pub client_id: u32,
}

impl ServerClientIdConfirmPdu {
    /// Encode to bytes.
    pub fn encode(&self) -> Vec<u8> {
        let mut body = Writer::new();
        body.write_u16_le(self.version_major);
        body.write_u16_le(self.version_minor);
        body.write_u32_le(self.client_id);
        wrap(PAKID_CORE_CLIENTID_CONFIRM, body.as_slice())
    }

    /// Decode from bytes.
    pub fn decode(buf: &[u8]) -> Result<ServerClientIdConfirmPdu> {
        let mut r = unwrap(buf, PAKID_CORE_CLIENTID_CONFIRM)?;
        Ok(ServerClientIdConfirmPdu {
            version_major: r.read_u16_le()?,
            version_minor: r.read_u16_le()?,
            client_id: r.read_u32_le()?,
        })
    }
}

/// `DR_CORE_CLIENT_NAME_REQ` — the client announces its machine name.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ClientNameRequestPdu {
    /// The client's computer name.
    pub computer_name: String,
}

impl ClientNameRequestPdu {
    /// Encode to bytes (always as Unicode; `CodePage` is fixed at 0).
    pub fn encode(&self) -> Vec<u8> {
        let mut name_bytes = Writer::new();
        for u in self.computer_name.encode_utf16() {
            name_bytes.write_u16_le(u);
        }
        name_bytes.write_u16_le(0); // NUL terminator

        let mut body = Writer::new();
        body.write_u32_le(1); // UnicodeFlag
        body.write_u32_le(0); // CodePage
        body.write_u32_le(name_bytes.len() as u32);
        body.write_bytes(name_bytes.as_slice());
        wrap(PAKID_CORE_CLIENT_NAME, body.as_slice())
    }

    /// Decode from bytes.
    pub fn decode(buf: &[u8]) -> Result<ClientNameRequestPdu> {
        let mut r = unwrap(buf, PAKID_CORE_CLIENT_NAME)?;
        let unicode = r.read_u32_le()? & 1 != 0;
        let _code_page = r.read_u32_le()?;
        let len = r.read_u32_le()? as usize;
        let raw = r.read_bytes(len)?;
        let computer_name = if unicode {
            let units: Vec<u16> = raw
                .chunks_exact(2)
                .map(|c| u16::from_le_bytes([c[0], c[1]]))
                .collect();
            let end = units.iter().position(|&u| u == 0).unwrap_or(units.len());
            String::from_utf16_lossy(&units[..end])
        } else {
            let end = raw.iter().position(|&b| b == 0).unwrap_or(raw.len());
            String::from_utf8_lossy(&raw[..end]).into_owned()
        };
        Ok(ClientNameRequestPdu { computer_name })
    }
}

// CapabilityType values (CAPABILITY_HEADER's CapabilityType field).
const CAP_GENERAL_TYPE: u16 = 0x0001;

const GENERAL_CAPABILITY_VERSION_01: u32 = 0x0000_0001;
const GENERAL_CAPABILITY_VERSION_02: u32 = 0x0000_0002;

/// `GENERAL_CAPS_SET` — non-device-specific capabilities.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct GeneralCapsSet {
    /// Bitmask of supported I/O request major functions.
    pub io_code1: u32,
    /// Bitmask of `RDPDR_DEVICE_REMOVE_PDUS`/`CLIENT_DISPLAY_NAME_PDU`/
    /// `USER_LOGGEDON_PDU`.
    pub extended_pdu: u32,
    /// Bitmask of `ENABLE_ASYNCIO` and reserved bits.
    pub extra_flags1: u32,
    /// Number of special devices to redirect before user logon
    /// (`GENERAL_CAPABILITY_VERSION_02` only).
    pub special_type_device_cap: Option<u32>,
}

impl GeneralCapsSet {
    fn encode_into(&self, w: &mut Writer, version_major: u16, version_minor: u16) {
        let version = if self.special_type_device_cap.is_some() {
            GENERAL_CAPABILITY_VERSION_02
        } else {
            GENERAL_CAPABILITY_VERSION_01
        };
        let len = if self.special_type_device_cap.is_some() {
            36
        } else {
            32
        };
        w.write_u16_le(CAP_GENERAL_TYPE);
        w.write_u16_le(len);
        w.write_u32_le(version);
        w.write_u32_le(0); // osType, ignored
        w.write_u32_le(0); // osVersion, unused
        w.write_u16_le(version_major);
        w.write_u16_le(version_minor);
        w.write_u32_le(self.io_code1);
        w.write_u32_le(0); // ioCode2, reserved
        w.write_u32_le(self.extended_pdu);
        w.write_u32_le(self.extra_flags1);
        w.write_u32_le(0); // extraFlags2, reserved
        if let Some(special) = self.special_type_device_cap {
            w.write_u32_le(special);
        }
    }

    fn decode_from(r: &mut Reader<'_>, version: u32) -> Result<GeneralCapsSet> {
        let _os_type = r.read_u32_le()?;
        let _os_version = r.read_u32_le()?;
        let _protocol_major = r.read_u16_le()?;
        let _protocol_minor = r.read_u16_le()?;
        let io_code1 = r.read_u32_le()?;
        let _io_code2 = r.read_u32_le()?;
        let extended_pdu = r.read_u32_le()?;
        let extra_flags1 = r.read_u32_le()?;
        let _extra_flags2 = r.read_u32_le()?;
        let special_type_device_cap = if version >= GENERAL_CAPABILITY_VERSION_02 {
            Some(r.read_u32_le()?)
        } else {
            None
        };
        Ok(GeneralCapsSet {
            io_code1,
            extended_pdu,
            extra_flags1,
            special_type_device_cap,
        })
    }
}

/// One entry of a Server Core Capability Request / Client Core Capability
/// Response's capability array. Only the General Capability Set is
/// interpreted; any other type (Printer/Port/Drive/Smart Card) is
/// preserved raw.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum CapabilitySet {
    /// `CAP_GENERAL_TYPE`.
    General(GeneralCapsSet),
    /// Any other `CapabilityType`, preserved as raw capability data
    /// (header included, for simplicity of round-tripping unknown sets).
    Other {
        /// The unrecognized `CapabilityType`.
        cap_type: u16,
        /// The set's `Version` field.
        version: u32,
        /// The raw bytes following `Version`.
        data: Vec<u8>,
    },
}

impl CapabilitySet {
    fn encode_into(&self, w: &mut Writer, version_major: u16, version_minor: u16) {
        match self {
            CapabilitySet::General(g) => g.encode_into(w, version_major, version_minor),
            CapabilitySet::Other {
                cap_type,
                version,
                data,
            } => {
                w.write_u16_le(*cap_type);
                w.write_u16_le((8 + data.len()) as u16);
                w.write_u32_le(*version);
                w.write_bytes(data);
            }
        }
    }

    fn decode_from(r: &mut Reader<'_>) -> Result<CapabilitySet> {
        let cap_type = r.read_u16_le()?;
        let length = r.read_u16_le()? as usize;
        let version = r.read_u32_le()?;
        let data_len = length.checked_sub(8).ok_or(Error::InvalidLength {
            field: "CAPABILITY_HEADER CapabilityLength",
            length,
        })?;
        if cap_type == CAP_GENERAL_TYPE {
            let start = r.position();
            let set = GeneralCapsSet::decode_from(r, version)?;
            let consumed = r.position() - start;
            r.skip(data_len.saturating_sub(consumed))?;
            Ok(CapabilitySet::General(set))
        } else {
            let data = r.read_bytes(data_len)?.to_vec();
            Ok(CapabilitySet::Other {
                cap_type,
                version,
                data,
            })
        }
    }
}

fn encode_capability_sets(
    sets: &[CapabilitySet],
    version_major: u16,
    version_minor: u16,
) -> Vec<u8> {
    let mut body = Writer::new();
    body.write_u16_le(sets.len() as u16);
    body.write_u16_le(0); // Padding
    for set in sets {
        set.encode_into(&mut body, version_major, version_minor);
    }
    body.into_vec()
}

fn decode_capability_sets(r: &mut Reader<'_>) -> Result<Vec<CapabilitySet>> {
    let count = r.read_u16_le()?;
    let _padding = r.read_u16_le()?;
    let mut sets = Vec::with_capacity(count as usize);
    for _ in 0..count {
        sets.push(CapabilitySet::decode_from(r)?);
    }
    Ok(sets)
}

/// `DR_CORE_CAPABILITY_REQ` (sent by the server, `PAKID_CORE_SERVER_CAPABILITY`).
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ServerCoreCapabilityPdu {
    /// The offered capability sets.
    pub sets: Vec<CapabilitySet>,
}

impl ServerCoreCapabilityPdu {
    /// Encode to bytes. `version_major`/`version_minor` fill in any
    /// [`GeneralCapsSet`]'s protocol version fields.
    pub fn encode(&self, version_major: u16, version_minor: u16) -> Vec<u8> {
        wrap(
            PAKID_CORE_SERVER_CAPABILITY,
            &encode_capability_sets(&self.sets, version_major, version_minor),
        )
    }

    /// Decode from bytes.
    pub fn decode(buf: &[u8]) -> Result<ServerCoreCapabilityPdu> {
        let mut r = unwrap(buf, PAKID_CORE_SERVER_CAPABILITY)?;
        Ok(ServerCoreCapabilityPdu {
            sets: decode_capability_sets(&mut r)?,
        })
    }
}

/// `DR_CORE_CLIENT_CAPABILITY_RSP` (sent by the client, `PAKID_CORE_CLIENT_CAPABILITY`).
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ClientCoreCapabilityPdu {
    /// The accepted capability sets.
    pub sets: Vec<CapabilitySet>,
}

impl ClientCoreCapabilityPdu {
    /// Encode to bytes. `version_major`/`version_minor` fill in any
    /// [`GeneralCapsSet`]'s protocol version fields.
    pub fn encode(&self, version_major: u16, version_minor: u16) -> Vec<u8> {
        wrap(
            PAKID_CORE_CLIENT_CAPABILITY,
            &encode_capability_sets(&self.sets, version_major, version_minor),
        )
    }

    /// Decode from bytes.
    pub fn decode(buf: &[u8]) -> Result<ClientCoreCapabilityPdu> {
        let mut r = unwrap(buf, PAKID_CORE_CLIENT_CAPABILITY)?;
        Ok(ClientCoreCapabilityPdu {
            sets: decode_capability_sets(&mut r)?,
        })
    }
}

/// `DEVICE_ANNOUNCE` — describes one redirected device.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct DeviceAnnounce {
    /// One of the `RDPDR_DTYP_*` constants.
    pub device_type: u32,
    /// Client-assigned unique ID for this device.
    pub device_id: u32,
    /// The device's name as it appears on the client, at most 7 characters
    /// (the 8th byte is always the NUL terminator on the wire).
    pub dos_name: String,
    /// Device-type-specific data (e.g. the file system device's root path).
    pub data: Vec<u8>,
}

impl DeviceAnnounce {
    fn encode_into(&self, w: &mut Writer) {
        w.write_u32_le(self.device_type);
        w.write_u32_le(self.device_id);
        let mut name_bytes = self.dos_name.as_bytes().to_vec();
        name_bytes.truncate(7);
        name_bytes.resize(8, 0);
        w.write_bytes(&name_bytes);
        w.write_u32_le(self.data.len() as u32);
        w.write_bytes(&self.data);
    }

    fn decode_from(r: &mut Reader<'_>) -> Result<DeviceAnnounce> {
        let device_type = r.read_u32_le()?;
        let device_id = r.read_u32_le()?;
        let name_bytes = r.read_bytes(8)?;
        let end = name_bytes.iter().position(|&b| b == 0).unwrap_or(8);
        let dos_name = String::from_utf8_lossy(&name_bytes[..end]).into_owned();
        let data_len = r.read_u32_le()? as usize;
        let data = r.read_bytes(data_len)?.to_vec();
        Ok(DeviceAnnounce {
            device_type,
            device_id,
            dos_name,
            data,
        })
    }
}

/// `DR_CORE_DEVICELIST_ANNOUNCE_REQ` — the client announces the devices it
/// is redirecting.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ClientDeviceListAnnouncePdu {
    /// The announced devices.
    pub devices: Vec<DeviceAnnounce>,
}

impl ClientDeviceListAnnouncePdu {
    /// Encode to bytes.
    pub fn encode(&self) -> Vec<u8> {
        let mut body = Writer::new();
        body.write_u32_le(self.devices.len() as u32);
        for d in &self.devices {
            d.encode_into(&mut body);
        }
        wrap(PAKID_CORE_DEVICELIST_ANNOUNCE, body.as_slice())
    }

    /// Decode from bytes.
    pub fn decode(buf: &[u8]) -> Result<ClientDeviceListAnnouncePdu> {
        let mut r = unwrap(buf, PAKID_CORE_DEVICELIST_ANNOUNCE)?;
        let count = r.read_u32_le()?;
        let mut devices = Vec::with_capacity(count as usize);
        for _ in 0..count {
            devices.push(DeviceAnnounce::decode_from(&mut r)?);
        }
        Ok(ClientDeviceListAnnouncePdu { devices })
    }
}

/// `DR_CORE_DEVICE_ANNOUNCE_RSP` — the server's per-device reply to a
/// [`ClientDeviceListAnnouncePdu`] entry.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct ServerDeviceAnnounceResponsePdu {
    /// Echoes a `device_id` from the announce list.
    pub device_id: u32,
    /// An NTSTATUS code; `0` (`STATUS_SUCCESS`) on success.
    pub result_code: u32,
}

impl ServerDeviceAnnounceResponsePdu {
    /// Encode to bytes.
    pub fn encode(&self) -> Vec<u8> {
        let mut body = Writer::new();
        body.write_u32_le(self.device_id);
        body.write_u32_le(self.result_code);
        wrap(PAKID_CORE_DEVICE_REPLY, body.as_slice())
    }

    /// Decode from bytes.
    pub fn decode(buf: &[u8]) -> Result<ServerDeviceAnnounceResponsePdu> {
        let mut r = unwrap(buf, PAKID_CORE_DEVICE_REPLY)?;
        Ok(ServerDeviceAnnounceResponsePdu {
            device_id: r.read_u32_le()?,
            result_code: r.read_u32_le()?,
        })
    }

    /// `true` when `result_code` is `STATUS_SUCCESS` (0).
    pub fn succeeded(&self) -> bool {
        self.result_code == 0
    }
}

/// `DR_CORE_USER_LOGGEDON` — sent by the server once the user's session is
/// active; carries no data beyond the header.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub struct ServerUserLoggedOnPdu;

impl ServerUserLoggedOnPdu {
    /// Encode to bytes.
    pub fn encode(&self) -> Vec<u8> {
        wrap(PAKID_CORE_USER_LOGGEDON, &[])
    }

    /// Decode from bytes.
    pub fn decode(buf: &[u8]) -> Result<ServerUserLoggedOnPdu> {
        unwrap(buf, PAKID_CORE_USER_LOGGEDON)?;
        Ok(ServerUserLoggedOnPdu)
    }
}

/// `DR_DEVICE_IOREQUEST` — the 24-byte header embedded in every server
/// request on a specific device (section 2.2.1.4). Unlike the other RDPDR
/// messages, its `PacketId` is always `PAKID_CORE_DEVICE_IOREQUEST`
/// regardless of the operation; [`major_function`](Self::major_function)
/// is what distinguishes a create from a read, a write, and so on.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct DeviceIoRequest {
    /// Matches the `DeviceId` from the [`DeviceAnnounce`] this request
    /// targets.
    pub device_id: u32,
    /// A unique ID retrieved from the [`DeviceCreateResponsePdu`] that
    /// opened the file/handle this request operates on. Meaningless (and
    /// conventionally `0`) on the [`DeviceCreateRequestPdu`] itself, since
    /// no file is open yet.
    pub file_id: u32,
    /// A unique ID for this request, echoed in the matching
    /// [`DeviceIoResponse`]; reusable once that response is received.
    pub completion_id: u32,
    /// One of the `IRP_MJ_*` constants, identifying the request function.
    pub major_function: u32,
    /// Valid only when `major_function` is [`IRP_MJ_DIRECTORY_CONTROL`];
    /// one of the `IRP_MN_*` constants. `0` for every other major
    /// function.
    pub minor_function: u32,
}

impl DeviceIoRequest {
    fn encode_into(&self, w: &mut Writer) {
        RdpdrHeader {
            component: RDPDR_CTYP_CORE,
            packet_id: PAKID_CORE_DEVICE_IOREQUEST,
        }
        .encode(w);
        w.write_u32_le(self.device_id);
        w.write_u32_le(self.file_id);
        w.write_u32_le(self.completion_id);
        w.write_u32_le(self.major_function);
        w.write_u32_le(self.minor_function);
    }

    fn decode_from(r: &mut Reader<'_>) -> Result<DeviceIoRequest> {
        let header = RdpdrHeader::decode(r)?;
        if header.component != RDPDR_CTYP_CORE {
            return Err(Error::InvalidValue {
                field: "RDPDR_HEADER Component",
                value: format!("0x{:04X}", header.component),
            });
        }
        if header.packet_id != PAKID_CORE_DEVICE_IOREQUEST {
            return Err(Error::InvalidValue {
                field: "RDPDR_HEADER PacketId",
                value: format!(
                    "0x{:04X} (expected 0x{PAKID_CORE_DEVICE_IOREQUEST:04X})",
                    header.packet_id
                ),
            });
        }
        Ok(DeviceIoRequest {
            device_id: r.read_u32_le()?,
            file_id: r.read_u32_le()?,
            completion_id: r.read_u32_le()?,
            major_function: r.read_u32_le()?,
            minor_function: r.read_u32_le()?,
        })
    }

    fn expect_major_function(&self, expected: u32) -> Result<()> {
        if self.major_function != expected {
            return Err(Error::InvalidValue {
                field: "DR_DEVICE_IOREQUEST MajorFunction",
                value: format!("0x{:08X} (expected 0x{expected:08X})", self.major_function),
            });
        }
        Ok(())
    }
}

/// Peek the [`DeviceIoRequest`] header of an encoded Device I/O Request PDU
/// without consuming the buffer, to route on `major_function` before
/// picking the matching `DeviceXxxRequestPdu::decode`.
pub fn decode_device_io_request(buf: &[u8]) -> Result<DeviceIoRequest> {
    let mut r = Reader::new(buf);
    DeviceIoRequest::decode_from(&mut r)
}

/// `DR_DEVICE_IOCOMPLETION` — the 16-byte header embedded in every client
/// response to a [`DeviceIoRequest`] (section 2.2.1.5). Matched to its
/// request by `completion_id`; there is exactly one response per request.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct DeviceIoResponse {
    /// Matches the `device_id` of the corresponding request.
    pub device_id: u32,
    /// Matches the `completion_id` of the corresponding request.
    pub completion_id: u32,
    /// An NTSTATUS code: `0` (`STATUS_SUCCESS`) on success.
    pub io_status: u32,
}

impl DeviceIoResponse {
    fn encode_into(&self, w: &mut Writer) {
        RdpdrHeader {
            component: RDPDR_CTYP_CORE,
            packet_id: PAKID_CORE_DEVICE_IOCOMPLETION,
        }
        .encode(w);
        w.write_u32_le(self.device_id);
        w.write_u32_le(self.completion_id);
        w.write_u32_le(self.io_status);
    }

    fn decode_from(r: &mut Reader<'_>) -> Result<DeviceIoResponse> {
        let header = RdpdrHeader::decode(r)?;
        if header.component != RDPDR_CTYP_CORE {
            return Err(Error::InvalidValue {
                field: "RDPDR_HEADER Component",
                value: format!("0x{:04X}", header.component),
            });
        }
        if header.packet_id != PAKID_CORE_DEVICE_IOCOMPLETION {
            return Err(Error::InvalidValue {
                field: "RDPDR_HEADER PacketId",
                value: format!(
                    "0x{:04X} (expected 0x{PAKID_CORE_DEVICE_IOCOMPLETION:04X})",
                    header.packet_id
                ),
            });
        }
        Ok(DeviceIoResponse {
            device_id: r.read_u32_le()?,
            completion_id: r.read_u32_le()?,
            io_status: r.read_u32_le()?,
        })
    }

    /// `true` when `io_status` is `STATUS_SUCCESS` (0).
    pub fn succeeded(&self) -> bool {
        self.io_status == 0
    }
}

/// Peek the [`DeviceIoResponse`] header of an encoded Device I/O Response
/// PDU without consuming the buffer. Unlike the request side, the wire
/// format carries no indication of which major function this completes —
/// a caller must track that itself, keyed on `completion_id`.
pub fn decode_device_io_response(buf: &[u8]) -> Result<DeviceIoResponse> {
    let mut r = Reader::new(buf);
    DeviceIoResponse::decode_from(&mut r)
}

/// `DR_CREATE_REQ` — a create/open request (`IRP_MJ_CREATE`). What it
/// creates or opens depends on the target device's type.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct DeviceCreateRequestPdu {
    /// Target device.
    pub device_id: u32,
    /// Matched back in [`DeviceIoResponse::completion_id`].
    pub completion_id: u32,
    /// Requested access level (`MS-SMB2` 2.2.13 `DesiredAccess`).
    pub desired_access: u32,
    /// Initial allocation size for the file, if created.
    pub allocation_size: u64,
    /// File attributes (`MS-SMB2` 2.2.13 `FileAttributes`).
    pub file_attributes: u32,
    /// Sharing mode (`MS-SMB2` 2.2.13 `ShareAccess`).
    pub shared_access: u32,
    /// Action to take if the file already exists (`MS-SMB2` 2.2.13
    /// `CreateDisposition`). Ports and other non-filesystem devices
    /// require `FILE_OPEN` (`0x0000_0001`).
    pub create_disposition: u32,
    /// Options for creating the file (`MS-SMB2` 2.2.13 `CreateOptions`).
    pub create_options: u32,
    /// The path being created/opened.
    pub path: String,
}

impl DeviceCreateRequestPdu {
    /// Encode to bytes.
    pub fn encode(&self) -> Vec<u8> {
        let mut path_units: Vec<u16> = self.path.encode_utf16().collect();
        path_units.push(0); // NUL terminator
        let path_len_bytes = (path_units.len() * 2) as u32;

        let mut w = Writer::new();
        DeviceIoRequest {
            device_id: self.device_id,
            file_id: 0,
            completion_id: self.completion_id,
            major_function: IRP_MJ_CREATE,
            minor_function: 0,
        }
        .encode_into(&mut w);
        w.write_u32_le(self.desired_access);
        w.write_u64_le(self.allocation_size);
        w.write_u32_le(self.file_attributes);
        w.write_u32_le(self.shared_access);
        w.write_u32_le(self.create_disposition);
        w.write_u32_le(self.create_options);
        w.write_u32_le(path_len_bytes);
        for u in path_units {
            w.write_u16_le(u);
        }
        w.into_vec()
    }

    /// Decode from bytes.
    pub fn decode(buf: &[u8]) -> Result<DeviceCreateRequestPdu> {
        let mut r = Reader::new(buf);
        let io_request = DeviceIoRequest::decode_from(&mut r)?;
        io_request.expect_major_function(IRP_MJ_CREATE)?;
        let desired_access = r.read_u32_le()?;
        let allocation_size = r.read_u64_le()?;
        let file_attributes = r.read_u32_le()?;
        let shared_access = r.read_u32_le()?;
        let create_disposition = r.read_u32_le()?;
        let create_options = r.read_u32_le()?;
        let path_len = r.read_u32_le()? as usize;
        let raw = r.read_bytes(path_len)?;
        let units: Vec<u16> = raw
            .chunks_exact(2)
            .map(|c| u16::from_le_bytes([c[0], c[1]]))
            .collect();
        let end = units.iter().position(|&u| u == 0).unwrap_or(units.len());
        let path = String::from_utf16_lossy(&units[..end]);
        Ok(DeviceCreateRequestPdu {
            device_id: io_request.device_id,
            completion_id: io_request.completion_id,
            desired_access,
            allocation_size,
            file_attributes,
            shared_access,
            create_disposition,
            create_options,
            path,
        })
    }
}

/// `FILE_SUPERSEDED` — a new file was created, or an existing one
/// superseded/overwritten per `FILE_SUPERSEDE`/`FILE_OPEN`/`FILE_CREATE`/
/// `FILE_OVERWRITE`.
pub const FILE_SUPERSEDED: u8 = 0x00;
/// `FILE_OPENED` — an existing file was opened (`FILE_OPEN_IF`).
pub const FILE_OPENED: u8 = 0x01;
/// `FILE_OVERWRITTEN` — an existing file was overwritten
/// (`FILE_OVERWRITE_IF`).
pub const FILE_OVERWRITTEN: u8 = 0x03;

/// `DR_CREATE_RSP` — the response to a [`DeviceCreateRequestPdu`].
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct DeviceCreateResponsePdu {
    /// Echoes the request's device.
    pub device_id: u32,
    /// Echoes the request's `completion_id`.
    pub completion_id: u32,
    /// An NTSTATUS code; `0` (`STATUS_SUCCESS`) on success.
    pub io_status: u32,
    /// A unique ID for the created file object, reused after the matching
    /// [`DeviceCloseResponsePdu`] is sent. Meaningless when `io_status`
    /// indicates failure.
    pub file_id: u32,
    /// One of `FILE_SUPERSEDED`/`FILE_OPENED`/`FILE_OVERWRITTEN`,
    /// determined by the request's `create_disposition`. `None` when
    /// omitted on the wire, which the spec says a receiver must treat as
    /// `FILE_SUPERSEDED`.
    pub information: Option<u8>,
}

impl DeviceCreateResponsePdu {
    /// Encode to bytes.
    pub fn encode(&self) -> Vec<u8> {
        let mut w = Writer::new();
        DeviceIoResponse {
            device_id: self.device_id,
            completion_id: self.completion_id,
            io_status: self.io_status,
        }
        .encode_into(&mut w);
        w.write_u32_le(self.file_id);
        if let Some(information) = self.information {
            w.write_u8(information);
        }
        w.into_vec()
    }

    /// Decode from bytes.
    pub fn decode(buf: &[u8]) -> Result<DeviceCreateResponsePdu> {
        let mut r = Reader::new(buf);
        let io_completion = DeviceIoResponse::decode_from(&mut r)?;
        let file_id = r.read_u32_le()?;
        let information = if r.is_empty() {
            None
        } else {
            Some(r.read_u8()?)
        };
        Ok(DeviceCreateResponsePdu {
            device_id: io_completion.device_id,
            completion_id: io_completion.completion_id,
            io_status: io_completion.io_status,
            file_id,
            information,
        })
    }

    /// `true` when `io_status` is `STATUS_SUCCESS` (0).
    pub fn succeeded(&self) -> bool {
        self.io_status == 0
    }
}

/// `DR_CLOSE_REQ` — a close request (`IRP_MJ_CLOSE`) for a file opened by a
/// [`DeviceCreateRequestPdu`].
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct DeviceCloseRequestPdu {
    /// Target device.
    pub device_id: u32,
    /// The file being closed, from [`DeviceCreateResponsePdu::file_id`].
    pub file_id: u32,
    /// Matched back in [`DeviceIoResponse::completion_id`].
    pub completion_id: u32,
}

impl DeviceCloseRequestPdu {
    /// Encode to bytes.
    pub fn encode(&self) -> Vec<u8> {
        let mut w = Writer::new();
        DeviceIoRequest {
            device_id: self.device_id,
            file_id: self.file_id,
            completion_id: self.completion_id,
            major_function: IRP_MJ_CLOSE,
            minor_function: 0,
        }
        .encode_into(&mut w);
        w.write_bytes(&[0u8; 32]); // Padding
        w.into_vec()
    }

    /// Decode from bytes.
    pub fn decode(buf: &[u8]) -> Result<DeviceCloseRequestPdu> {
        let mut r = Reader::new(buf);
        let io_request = DeviceIoRequest::decode_from(&mut r)?;
        io_request.expect_major_function(IRP_MJ_CLOSE)?;
        r.skip(32)?; // Padding
        Ok(DeviceCloseRequestPdu {
            device_id: io_request.device_id,
            file_id: io_request.file_id,
            completion_id: io_request.completion_id,
        })
    }
}

/// `DR_CLOSE_RSP` — the response to a [`DeviceCloseRequestPdu`].
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct DeviceCloseResponsePdu {
    /// Echoes the request's device.
    pub device_id: u32,
    /// Echoes the request's `completion_id`.
    pub completion_id: u32,
    /// An NTSTATUS code; `0` (`STATUS_SUCCESS`) on success.
    pub io_status: u32,
}

impl DeviceCloseResponsePdu {
    /// Encode to bytes.
    pub fn encode(&self) -> Vec<u8> {
        let mut w = Writer::new();
        DeviceIoResponse {
            device_id: self.device_id,
            completion_id: self.completion_id,
            io_status: self.io_status,
        }
        .encode_into(&mut w);
        w.write_u32_le(0); // Padding
        w.into_vec()
    }

    /// Decode from bytes.
    pub fn decode(buf: &[u8]) -> Result<DeviceCloseResponsePdu> {
        let mut r = Reader::new(buf);
        let io_completion = DeviceIoResponse::decode_from(&mut r)?;
        r.skip(4)?; // Padding
        Ok(DeviceCloseResponsePdu {
            device_id: io_completion.device_id,
            completion_id: io_completion.completion_id,
            io_status: io_completion.io_status,
        })
    }

    /// `true` when `io_status` is `STATUS_SUCCESS` (0).
    pub fn succeeded(&self) -> bool {
        self.io_status == 0
    }
}

/// `DR_READ_REQ` — a read request (`IRP_MJ_READ`).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct DeviceReadRequestPdu {
    /// Target device.
    pub device_id: u32,
    /// The file being read, from [`DeviceCreateResponsePdu::file_id`].
    pub file_id: u32,
    /// Matched back in [`DeviceIoResponse::completion_id`].
    pub completion_id: u32,
    /// Maximum number of bytes to read.
    pub length: u32,
    /// File offset to read from.
    pub offset: u64,
}

impl DeviceReadRequestPdu {
    /// Encode to bytes.
    pub fn encode(&self) -> Vec<u8> {
        let mut w = Writer::new();
        DeviceIoRequest {
            device_id: self.device_id,
            file_id: self.file_id,
            completion_id: self.completion_id,
            major_function: IRP_MJ_READ,
            minor_function: 0,
        }
        .encode_into(&mut w);
        w.write_u32_le(self.length);
        w.write_u64_le(self.offset);
        w.write_bytes(&[0u8; 20]); // Padding
        w.into_vec()
    }

    /// Decode from bytes.
    pub fn decode(buf: &[u8]) -> Result<DeviceReadRequestPdu> {
        let mut r = Reader::new(buf);
        let io_request = DeviceIoRequest::decode_from(&mut r)?;
        io_request.expect_major_function(IRP_MJ_READ)?;
        let length = r.read_u32_le()?;
        let offset = r.read_u64_le()?;
        r.skip(20)?; // Padding
        Ok(DeviceReadRequestPdu {
            device_id: io_request.device_id,
            file_id: io_request.file_id,
            completion_id: io_request.completion_id,
            length,
            offset,
        })
    }
}

/// `DR_READ_RSP` — the response to a [`DeviceReadRequestPdu`], carrying the
/// data read.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct DeviceReadResponsePdu {
    /// Echoes the request's device.
    pub device_id: u32,
    /// Echoes the request's `completion_id`.
    pub completion_id: u32,
    /// An NTSTATUS code; `0` (`STATUS_SUCCESS`) on success.
    pub io_status: u32,
    /// The bytes read.
    pub data: Vec<u8>,
}

impl DeviceReadResponsePdu {
    /// Encode to bytes.
    pub fn encode(&self) -> Vec<u8> {
        let mut w = Writer::with_capacity(20 + self.data.len());
        DeviceIoResponse {
            device_id: self.device_id,
            completion_id: self.completion_id,
            io_status: self.io_status,
        }
        .encode_into(&mut w);
        w.write_u32_le(self.data.len() as u32);
        w.write_bytes(&self.data);
        w.into_vec()
    }

    /// Decode from bytes.
    pub fn decode(buf: &[u8]) -> Result<DeviceReadResponsePdu> {
        let mut r = Reader::new(buf);
        let io_completion = DeviceIoResponse::decode_from(&mut r)?;
        let length = r.read_u32_le()? as usize;
        let data = r.read_bytes(length)?.to_vec();
        Ok(DeviceReadResponsePdu {
            device_id: io_completion.device_id,
            completion_id: io_completion.completion_id,
            io_status: io_completion.io_status,
            data,
        })
    }
}

/// `DR_WRITE_REQ` — a write request (`IRP_MJ_WRITE`).
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct DeviceWriteRequestPdu {
    /// Target device.
    pub device_id: u32,
    /// The file being written, from [`DeviceCreateResponsePdu::file_id`].
    pub file_id: u32,
    /// Matched back in [`DeviceIoResponse::completion_id`].
    pub completion_id: u32,
    /// File offset to write at.
    pub offset: u64,
    /// The bytes to write.
    pub data: Vec<u8>,
}

impl DeviceWriteRequestPdu {
    /// Encode to bytes.
    pub fn encode(&self) -> Vec<u8> {
        let mut w = Writer::with_capacity(56 + self.data.len());
        DeviceIoRequest {
            device_id: self.device_id,
            file_id: self.file_id,
            completion_id: self.completion_id,
            major_function: IRP_MJ_WRITE,
            minor_function: 0,
        }
        .encode_into(&mut w);
        w.write_u32_le(self.data.len() as u32);
        w.write_u64_le(self.offset);
        w.write_bytes(&[0u8; 20]); // Padding
        w.write_bytes(&self.data);
        w.into_vec()
    }

    /// Decode from bytes.
    pub fn decode(buf: &[u8]) -> Result<DeviceWriteRequestPdu> {
        let mut r = Reader::new(buf);
        let io_request = DeviceIoRequest::decode_from(&mut r)?;
        io_request.expect_major_function(IRP_MJ_WRITE)?;
        let length = r.read_u32_le()? as usize;
        let offset = r.read_u64_le()?;
        r.skip(20)?; // Padding
        let data = r.read_bytes(length)?.to_vec();
        Ok(DeviceWriteRequestPdu {
            device_id: io_request.device_id,
            file_id: io_request.file_id,
            completion_id: io_request.completion_id,
            offset,
            data,
        })
    }
}

/// `DR_WRITE_RSP` — the response to a [`DeviceWriteRequestPdu`].
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct DeviceWriteResponsePdu {
    /// Echoes the request's device.
    pub device_id: u32,
    /// Echoes the request's `completion_id`.
    pub completion_id: u32,
    /// An NTSTATUS code; `0` (`STATUS_SUCCESS`) on success.
    pub io_status: u32,
    /// The number of bytes actually written.
    pub length: u32,
}

impl DeviceWriteResponsePdu {
    /// Encode to bytes.
    pub fn encode(&self) -> Vec<u8> {
        let mut w = Writer::new();
        DeviceIoResponse {
            device_id: self.device_id,
            completion_id: self.completion_id,
            io_status: self.io_status,
        }
        .encode_into(&mut w);
        w.write_u32_le(self.length);
        w.write_u8(0); // Padding
        w.into_vec()
    }

    /// Decode from bytes.
    pub fn decode(buf: &[u8]) -> Result<DeviceWriteResponsePdu> {
        let mut r = Reader::new(buf);
        let io_completion = DeviceIoResponse::decode_from(&mut r)?;
        let length = r.read_u32_le()?;
        r.skip(1)?; // Padding
        Ok(DeviceWriteResponsePdu {
            device_id: io_completion.device_id,
            completion_id: io_completion.completion_id,
            io_status: io_completion.io_status,
            length,
        })
    }

    /// `true` when `io_status` is `STATUS_SUCCESS` (0).
    pub fn succeeded(&self) -> bool {
        self.io_status == 0
    }
}

/// `DR_CONTROL_REQ` — a device control request (`IRP_MJ_DEVICE_CONTROL`):
/// the generic IOCTL/FSCTL carrier that smart-card redirection (MS-RDPESC)
/// and serial/parallel port redirection (MS-RDPESP) ride on entirely, and
/// that filesystem redirection uses for `FSCTL_*` operations.
/// `io_control_code` and the buffer contents are device-specific and kept
/// opaque here.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct DeviceControlRequestPdu {
    /// Target device.
    pub device_id: u32,
    /// The file/handle this operates on, from
    /// [`DeviceCreateResponsePdu::file_id`].
    pub file_id: u32,
    /// Matched back in [`DeviceIoResponse::completion_id`].
    pub completion_id: u32,
    /// Maximum number of bytes expected in the response's `output_buffer`.
    pub output_buffer_length: u32,
    /// The device-specific IOCTL/FSCTL code.
    pub io_control_code: u32,
    /// Device-specific input, opaque to this codec.
    pub input_buffer: Vec<u8>,
}

impl DeviceControlRequestPdu {
    /// Encode to bytes.
    pub fn encode(&self) -> Vec<u8> {
        let mut w = Writer::with_capacity(56 + self.input_buffer.len());
        DeviceIoRequest {
            device_id: self.device_id,
            file_id: self.file_id,
            completion_id: self.completion_id,
            major_function: IRP_MJ_DEVICE_CONTROL,
            minor_function: 0,
        }
        .encode_into(&mut w);
        w.write_u32_le(self.output_buffer_length);
        w.write_u32_le(self.input_buffer.len() as u32);
        w.write_u32_le(self.io_control_code);
        w.write_bytes(&[0u8; 20]); // Padding
        w.write_bytes(&self.input_buffer);
        w.into_vec()
    }

    /// Decode from bytes.
    pub fn decode(buf: &[u8]) -> Result<DeviceControlRequestPdu> {
        let mut r = Reader::new(buf);
        let io_request = DeviceIoRequest::decode_from(&mut r)?;
        io_request.expect_major_function(IRP_MJ_DEVICE_CONTROL)?;
        let output_buffer_length = r.read_u32_le()?;
        let input_buffer_length = r.read_u32_le()? as usize;
        let io_control_code = r.read_u32_le()?;
        r.skip(20)?; // Padding
        let input_buffer = r.read_bytes(input_buffer_length)?.to_vec();
        Ok(DeviceControlRequestPdu {
            device_id: io_request.device_id,
            file_id: io_request.file_id,
            completion_id: io_request.completion_id,
            output_buffer_length,
            io_control_code,
            input_buffer,
        })
    }
}

/// `DR_CONTROL_RSP` — the response to a [`DeviceControlRequestPdu`].
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct DeviceControlResponsePdu {
    /// Echoes the request's device.
    pub device_id: u32,
    /// Echoes the request's `completion_id`.
    pub completion_id: u32,
    /// An NTSTATUS code; `0` (`STATUS_SUCCESS`) on success.
    pub io_status: u32,
    /// Device-specific output, opaque to this codec. Empty on failure.
    pub output_buffer: Vec<u8>,
}

impl DeviceControlResponsePdu {
    /// Encode to bytes.
    pub fn encode(&self) -> Vec<u8> {
        let mut w = Writer::with_capacity(20 + self.output_buffer.len());
        DeviceIoResponse {
            device_id: self.device_id,
            completion_id: self.completion_id,
            io_status: self.io_status,
        }
        .encode_into(&mut w);
        w.write_u32_le(self.output_buffer.len() as u32);
        w.write_bytes(&self.output_buffer);
        w.into_vec()
    }

    /// Decode from bytes.
    pub fn decode(buf: &[u8]) -> Result<DeviceControlResponsePdu> {
        let mut r = Reader::new(buf);
        let io_completion = DeviceIoResponse::decode_from(&mut r)?;
        let output_buffer_length = r.read_u32_le()? as usize;
        let output_buffer = r.read_bytes(output_buffer_length)?.to_vec();
        Ok(DeviceControlResponsePdu {
            device_id: io_completion.device_id,
            completion_id: io_completion.completion_id,
            io_status: io_completion.io_status,
            output_buffer,
        })
    }

    /// `true` when `io_status` is `STATUS_SUCCESS` (0).
    pub fn succeeded(&self) -> bool {
        self.io_status == 0
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn header_wire_shape() {
        let pdu = ServerUserLoggedOnPdu.encode();
        assert_eq!(pdu, vec![0x72, 0x44, 0x4C, 0x55]); // "Dr" LE, "LU" LE
        assert_eq!(
            decode_header(&pdu).unwrap(),
            RdpdrHeader {
                component: RDPDR_CTYP_CORE,
                packet_id: PAKID_CORE_USER_LOGGEDON
            }
        );
    }

    #[test]
    fn wrong_component_is_rejected() {
        let mut pdu = ServerUserLoggedOnPdu.encode();
        pdu[0] = 0x00;
        pdu[1] = 0x00;
        assert!(ServerUserLoggedOnPdu::decode(&pdu).is_err());
    }

    #[test]
    fn wrong_packet_id_is_rejected() {
        let pdu = ServerUserLoggedOnPdu.encode();
        assert!(ClientNameRequestPdu::decode(&pdu).is_err());
    }

    #[test]
    fn server_announce_roundtrip() {
        let pdu = ServerAnnounceRequestPdu {
            version_major: 1,
            version_minor: 0x0D,
            client_id: 0x1234_5678,
        };
        assert_eq!(
            ServerAnnounceRequestPdu::decode(&pdu.encode()).unwrap(),
            pdu
        );
    }

    #[test]
    fn client_announce_reply_and_server_confirm_share_shape() {
        let reply = ClientAnnounceReplyPdu {
            version_major: 1,
            version_minor: 0x0D,
            client_id: 42,
        };
        assert_eq!(
            ClientAnnounceReplyPdu::decode(&reply.encode()).unwrap(),
            reply
        );

        let confirm = ServerClientIdConfirmPdu {
            version_major: 1,
            version_minor: 0x0D,
            client_id: 42,
        };
        assert_eq!(
            ServerClientIdConfirmPdu::decode(&confirm.encode()).unwrap(),
            confirm
        );
        // Same wire shape (both PAKID_CORE_CLIENTID_CONFIRM).
        assert_eq!(reply.encode(), confirm.encode());
    }

    #[test]
    fn client_name_request_roundtrip_unicode() {
        let pdu = ClientNameRequestPdu {
            computer_name: "WORKSTATION1".to_string(),
        };
        assert_eq!(ClientNameRequestPdu::decode(&pdu.encode()).unwrap(), pdu);
    }

    #[test]
    fn client_name_request_decodes_ascii_variant() {
        let mut body = Writer::new();
        body.write_u32_le(0); // UnicodeFlag = ASCII
        body.write_u32_le(0); // CodePage
        let name = b"HOST\0";
        body.write_u32_le(name.len() as u32);
        body.write_bytes(name);
        let pdu = wrap(PAKID_CORE_CLIENT_NAME, body.as_slice());
        assert_eq!(
            ClientNameRequestPdu::decode(&pdu).unwrap(),
            ClientNameRequestPdu {
                computer_name: "HOST".to_string()
            }
        );
    }

    #[test]
    fn general_caps_set_v1_roundtrip_no_special_device_cap() {
        let sets = vec![CapabilitySet::General(GeneralCapsSet {
            io_code1: 0xFFFF,
            extended_pdu: 0x3,
            extra_flags1: 0,
            special_type_device_cap: None,
        })];
        let pdu = ServerCoreCapabilityPdu { sets };
        let decoded = ServerCoreCapabilityPdu::decode(&pdu.encode(1, 13)).unwrap();
        assert_eq!(decoded, pdu);
    }

    #[test]
    fn general_caps_set_v2_roundtrip_with_special_device_cap() {
        let sets = vec![CapabilitySet::General(GeneralCapsSet {
            io_code1: 0xFFFF,
            extended_pdu: 0x7,
            extra_flags1: 1,
            special_type_device_cap: Some(1),
        })];
        let pdu = ClientCoreCapabilityPdu { sets };
        let decoded = ClientCoreCapabilityPdu::decode(&pdu.encode(1, 13)).unwrap();
        assert_eq!(decoded, pdu);
    }

    #[test]
    fn unknown_capability_set_is_preserved_raw() {
        let sets = vec![CapabilitySet::Other {
            cap_type: 0x0004, // CAP_DRIVE_TYPE
            version: 1,
            data: vec![],
        }];
        let pdu = ServerCoreCapabilityPdu { sets };
        assert_eq!(
            ServerCoreCapabilityPdu::decode(&pdu.encode(1, 13)).unwrap(),
            pdu
        );
    }

    #[test]
    fn mixed_capability_sets_roundtrip() {
        let sets = vec![
            CapabilitySet::General(GeneralCapsSet {
                io_code1: 0xFFFF,
                extended_pdu: 0x3,
                extra_flags1: 0,
                special_type_device_cap: Some(0),
            }),
            CapabilitySet::Other {
                cap_type: 0x0002, // CAP_PRINTER_TYPE
                version: 1,
                data: vec![0xAA, 0xBB],
            },
            CapabilitySet::Other {
                cap_type: 0x0003, // CAP_PORT_TYPE
                version: 1,
                data: vec![],
            },
        ];
        let pdu = ServerCoreCapabilityPdu { sets };
        assert_eq!(
            ServerCoreCapabilityPdu::decode(&pdu.encode(1, 13)).unwrap(),
            pdu
        );
    }

    #[test]
    fn device_announce_roundtrip_filesystem() {
        let dev = DeviceAnnounce {
            device_type: RDPDR_DTYP_FILESYSTEM,
            device_id: 1,
            dos_name: "DISK1".to_string(),
            data: b"DISK1\0".to_vec(),
        };
        let mut w = Writer::new();
        dev.encode_into(&mut w);
        let mut r = Reader::new(w.as_slice());
        assert_eq!(DeviceAnnounce::decode_from(&mut r).unwrap(), dev);
    }

    #[test]
    fn device_announce_dos_name_truncated_to_seven_chars() {
        let dev = DeviceAnnounce {
            device_type: RDPDR_DTYP_SMARTCARD,
            device_id: 2,
            dos_name: "TOOLONGNAME".to_string(),
            data: vec![],
        };
        let mut w = Writer::new();
        dev.encode_into(&mut w);
        assert_eq!(w.len(), 8 + 8 + 4); // type+id, name, dataLen
        let mut r = Reader::new(w.as_slice());
        let decoded = DeviceAnnounce::decode_from(&mut r).unwrap();
        assert_eq!(decoded.dos_name, "TOOLONG");
    }

    #[test]
    fn client_device_list_announce_roundtrip_multiple() {
        let pdu = ClientDeviceListAnnouncePdu {
            devices: vec![
                DeviceAnnounce {
                    device_type: RDPDR_DTYP_FILESYSTEM,
                    device_id: 1,
                    dos_name: "DISK1".to_string(),
                    data: b"DISK1\0".to_vec(),
                },
                DeviceAnnounce {
                    device_type: RDPDR_DTYP_SMARTCARD,
                    device_id: 2,
                    dos_name: "SCARD".to_string(),
                    data: vec![],
                },
            ],
        };
        assert_eq!(
            ClientDeviceListAnnouncePdu::decode(&pdu.encode()).unwrap(),
            pdu
        );
    }

    #[test]
    fn server_device_announce_response_roundtrip_and_success() {
        let ok = ServerDeviceAnnounceResponsePdu {
            device_id: 1,
            result_code: 0,
        };
        assert!(ok.succeeded());
        assert_eq!(
            ServerDeviceAnnounceResponsePdu::decode(&ok.encode()).unwrap(),
            ok
        );

        let failed = ServerDeviceAnnounceResponsePdu {
            device_id: 1,
            result_code: 0xC000_0022, // STATUS_ACCESS_DENIED
        };
        assert!(!failed.succeeded());
        assert_eq!(
            ServerDeviceAnnounceResponsePdu::decode(&failed.encode()).unwrap(),
            failed
        );
    }

    #[test]
    fn user_logged_on_wire_shape_and_roundtrip() {
        let pdu = ServerUserLoggedOnPdu;
        assert_eq!(ServerUserLoggedOnPdu::decode(&pdu.encode()).unwrap(), pdu);
    }

    /// Simulate the full initialization handshake end to end, matching
    /// MS-RDPEFS 1.3.2: announce/confirm, client name, capability exchange,
    /// then a filesystem device announced and accepted.
    #[test]
    fn full_initialization_sequence() {
        let announce = ServerAnnounceRequestPdu {
            version_major: 1,
            version_minor: 0x0D,
            client_id: 0xAABB_CCDD,
        };
        let announce_bytes = announce.encode();
        let decoded_announce = ServerAnnounceRequestPdu::decode(&announce_bytes).unwrap();

        let reply = ClientAnnounceReplyPdu {
            version_major: decoded_announce.version_major,
            version_minor: decoded_announce.version_minor,
            client_id: decoded_announce.client_id,
        };
        assert_eq!(
            ClientAnnounceReplyPdu::decode(&reply.encode())
                .unwrap()
                .client_id,
            0xAABB_CCDD
        );

        let name_req = ClientNameRequestPdu {
            computer_name: "CLIENT-PC".to_string(),
        }
        .encode();
        assert_eq!(
            ClientNameRequestPdu::decode(&name_req)
                .unwrap()
                .computer_name,
            "CLIENT-PC"
        );

        let confirm = ServerClientIdConfirmPdu {
            version_major: 1,
            version_minor: 0x0D,
            client_id: 0xAABB_CCDD,
        }
        .encode();
        assert_eq!(
            ServerClientIdConfirmPdu::decode(&confirm)
                .unwrap()
                .client_id,
            0xAABB_CCDD
        );

        let server_caps = ServerCoreCapabilityPdu {
            sets: vec![CapabilitySet::General(GeneralCapsSet {
                io_code1: 0xFFFF,
                extended_pdu: 0x7,
                extra_flags1: 0,
                special_type_device_cap: Some(0),
            })],
        }
        .encode(1, 0x0D);
        let server_caps_decoded = ServerCoreCapabilityPdu::decode(&server_caps).unwrap();

        let client_caps = ClientCoreCapabilityPdu {
            sets: server_caps_decoded.sets,
        }
        .encode(1, 0x0D);
        assert_eq!(
            ClientCoreCapabilityPdu::decode(&client_caps)
                .unwrap()
                .sets
                .len(),
            1
        );

        let device_list = ClientDeviceListAnnouncePdu {
            devices: vec![DeviceAnnounce {
                device_type: RDPDR_DTYP_FILESYSTEM,
                device_id: 1,
                dos_name: "DISK1".to_string(),
                data: b"DISK1\0".to_vec(),
            }],
        }
        .encode();
        let devices = ClientDeviceListAnnouncePdu::decode(&device_list)
            .unwrap()
            .devices;
        assert_eq!(devices.len(), 1);

        let response = ServerDeviceAnnounceResponsePdu {
            device_id: devices[0].device_id,
            result_code: 0,
        }
        .encode();
        assert!(ServerDeviceAnnounceResponsePdu::decode(&response)
            .unwrap()
            .succeeded());

        let logged_on = ServerUserLoggedOnPdu.encode();
        assert!(ServerUserLoggedOnPdu::decode(&logged_on).is_ok());
    }

    /// MS-RDPEFS 2.2.3.3.9 "Server Drive Write Request" example: a 9-byte
    /// write of `"sfddsafsa"` at offset 0 on `FileId` 0x223.
    #[test]
    fn write_request_matches_spec_vector() {
        #[rustfmt::skip]
        let bytes: [u8; 65] = [
            0x72, 0x44, 0x52, 0x49, 0x01, 0x00, 0x00, 0x00, 0x23, 0x02, 0x00, 0x00, 0x06, 0x00, 0x00, 0x00,
            0x04, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x09, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00,
            0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00,
            0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x73, 0x66, 0x64, 0x64, 0x73, 0x61, 0x66, 0x73,
            0x61,
        ];
        let pdu = DeviceWriteRequestPdu::decode(&bytes).unwrap();
        assert_eq!(pdu.device_id, 1);
        assert_eq!(pdu.file_id, 0x223);
        assert_eq!(pdu.completion_id, 6);
        assert_eq!(pdu.offset, 0);
        assert_eq!(pdu.data, b"sfddsafsa");
        assert_eq!(pdu.encode(), bytes);
    }

    /// MS-RDPEFS "Client Drive Close Response" example.
    #[test]
    fn close_response_matches_spec_vector() {
        #[rustfmt::skip]
        let bytes: [u8; 20] = [
            0x72, 0x44, 0x43, 0x49, 0x02, 0x00, 0x00, 0x00, 0x01, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00,
            0x00, 0x00, 0x00, 0x00,
        ];
        let pdu = DeviceCloseResponsePdu::decode(&bytes).unwrap();
        assert_eq!(
            pdu,
            DeviceCloseResponsePdu {
                device_id: 2,
                completion_id: 1,
                io_status: 0,
            }
        );
        assert!(pdu.succeeded());
        assert_eq!(pdu.encode(), bytes);
    }

    /// MS-RDPEFS "Client Drive Control Response" error example
    /// (`STATUS_UNSUCCESSFUL`, no output buffer).
    #[test]
    fn control_response_error_matches_spec_vector() {
        #[rustfmt::skip]
        let bytes: [u8; 20] = [
            0x72, 0x44, 0x43, 0x49, 0x01, 0x00, 0x00, 0x00, 0x08, 0x00, 0x00, 0x00, 0x01, 0x00, 0x00, 0xC0,
            0x00, 0x00, 0x00, 0x00,
        ];
        let pdu = DeviceControlResponsePdu::decode(&bytes).unwrap();
        assert_eq!(pdu.device_id, 1);
        assert_eq!(pdu.completion_id, 8);
        assert_eq!(pdu.io_status, 0xC000_0001);
        assert!(!pdu.succeeded());
        assert!(pdu.output_buffer.is_empty());
        assert_eq!(pdu.encode(), bytes);
    }

    #[test]
    fn device_io_request_header_roundtrips_and_is_peekable() {
        let req = DeviceIoRequest {
            device_id: 1,
            file_id: 2,
            completion_id: 3,
            major_function: IRP_MJ_READ,
            minor_function: 0,
        };
        let mut w = Writer::new();
        req.encode_into(&mut w);
        assert_eq!(decode_device_io_request(w.as_slice()).unwrap(), req);
    }

    #[test]
    fn device_io_response_header_roundtrips_and_is_peekable() {
        let resp = DeviceIoResponse {
            device_id: 1,
            completion_id: 3,
            io_status: 0,
        };
        let mut w = Writer::new();
        resp.encode_into(&mut w);
        assert_eq!(decode_device_io_response(w.as_slice()).unwrap(), resp);
        assert!(resp.succeeded());
    }

    #[test]
    fn wrong_major_function_is_rejected() {
        let pdu = DeviceReadRequestPdu {
            device_id: 1,
            file_id: 1,
            completion_id: 1,
            length: 4,
            offset: 0,
        }
        .encode();
        assert!(DeviceCreateRequestPdu::decode(&pdu).is_err());
        assert!(DeviceWriteRequestPdu::decode(&pdu).is_err());
    }

    #[test]
    fn create_request_roundtrips() {
        let pdu = DeviceCreateRequestPdu {
            device_id: 1,
            completion_id: 5,
            desired_access: 0x0010_0080,
            allocation_size: 0,
            file_attributes: 0x80,
            shared_access: 0x0000_0007,
            create_disposition: 1, // FILE_OPEN
            create_options: 0x60,
            path: "\\test\\file.txt".to_string(),
        };
        assert_eq!(DeviceCreateRequestPdu::decode(&pdu.encode()).unwrap(), pdu);
    }

    #[test]
    fn create_response_roundtrips_with_and_without_information() {
        let with_info = DeviceCreateResponsePdu {
            device_id: 1,
            completion_id: 5,
            io_status: 0,
            file_id: 0x223,
            information: Some(FILE_OPENED),
        };
        assert_eq!(
            DeviceCreateResponsePdu::decode(&with_info.encode()).unwrap(),
            with_info
        );
        assert!(with_info.succeeded());

        let without_info = DeviceCreateResponsePdu {
            device_id: 1,
            completion_id: 5,
            io_status: 0,
            file_id: 0x223,
            information: None,
        };
        let encoded = without_info.encode();
        assert_eq!(encoded.len(), 20); // DeviceIoResponse(16) + FileId(4), no Information byte
        assert_eq!(
            DeviceCreateResponsePdu::decode(&encoded).unwrap(),
            without_info
        );
    }

    #[test]
    fn create_response_failure_status() {
        let pdu = DeviceCreateResponsePdu {
            device_id: 1,
            completion_id: 5,
            io_status: 0xC000_0034, // STATUS_OBJECT_NAME_NOT_FOUND
            file_id: 0,
            information: None,
        };
        assert!(!pdu.succeeded());
        assert_eq!(DeviceCreateResponsePdu::decode(&pdu.encode()).unwrap(), pdu);
    }

    #[test]
    fn close_request_roundtrips() {
        let pdu = DeviceCloseRequestPdu {
            device_id: 1,
            file_id: 0x223,
            completion_id: 6,
        };
        let encoded = pdu.encode();
        assert_eq!(encoded.len(), 56); // DeviceIoRequest(24) + Padding(32)
        assert_eq!(DeviceCloseRequestPdu::decode(&encoded).unwrap(), pdu);
    }

    #[test]
    fn read_request_roundtrips() {
        let pdu = DeviceReadRequestPdu {
            device_id: 1,
            file_id: 0x223,
            completion_id: 7,
            length: 1536,
            offset: 0x1000,
        };
        let encoded = pdu.encode();
        assert_eq!(encoded.len(), 56); // DeviceIoRequest(24) + Length(4) + Offset(8) + Padding(20)
        assert_eq!(DeviceReadRequestPdu::decode(&encoded).unwrap(), pdu);
    }

    #[test]
    fn read_response_roundtrips() {
        let pdu = DeviceReadResponsePdu {
            device_id: 1,
            completion_id: 7,
            io_status: 0,
            data: vec![0xAA; 100],
        };
        assert_eq!(DeviceReadResponsePdu::decode(&pdu.encode()).unwrap(), pdu);
    }

    #[test]
    fn read_response_empty_data_roundtrips() {
        let pdu = DeviceReadResponsePdu {
            device_id: 1,
            completion_id: 7,
            io_status: 0,
            data: vec![],
        };
        assert_eq!(DeviceReadResponsePdu::decode(&pdu.encode()).unwrap(), pdu);
    }

    #[test]
    fn write_request_roundtrips() {
        let pdu = DeviceWriteRequestPdu {
            device_id: 1,
            file_id: 0x223,
            completion_id: 8,
            offset: 0x2000,
            data: b"hello, rdpdr".to_vec(),
        };
        assert_eq!(DeviceWriteRequestPdu::decode(&pdu.encode()).unwrap(), pdu);
    }

    #[test]
    fn write_response_roundtrips() {
        let pdu = DeviceWriteResponsePdu {
            device_id: 1,
            completion_id: 8,
            io_status: 0,
            length: 12,
        };
        let encoded = pdu.encode();
        assert_eq!(encoded.len(), 21); // DeviceIoResponse(16) + Length(4) + Padding(1)
        assert_eq!(DeviceWriteResponsePdu::decode(&encoded).unwrap(), pdu);
        assert!(pdu.succeeded());
    }

    #[test]
    fn control_request_roundtrips() {
        let pdu = DeviceControlRequestPdu {
            device_id: 1,
            file_id: 0x223,
            completion_id: 9,
            output_buffer_length: 256,
            io_control_code: 0x0009_0028, // FSCTL_LOCK_VOLUME (example IOCTL)
            input_buffer: vec![0x01, 0x02, 0x03, 0x04],
        };
        assert_eq!(DeviceControlRequestPdu::decode(&pdu.encode()).unwrap(), pdu);
    }

    #[test]
    fn control_request_empty_buffer_roundtrips() {
        let pdu = DeviceControlRequestPdu {
            device_id: 1,
            file_id: 0x223,
            completion_id: 9,
            output_buffer_length: 4,
            io_control_code: 0x0009_0028,
            input_buffer: vec![],
        };
        assert_eq!(DeviceControlRequestPdu::decode(&pdu.encode()).unwrap(), pdu);
    }

    #[test]
    fn control_response_success_roundtrips() {
        let pdu = DeviceControlResponsePdu {
            device_id: 1,
            completion_id: 9,
            io_status: 0,
            output_buffer: vec![0xDE, 0xAD, 0xBE, 0xEF],
        };
        assert!(pdu.succeeded());
        assert_eq!(
            DeviceControlResponsePdu::decode(&pdu.encode()).unwrap(),
            pdu
        );
    }

    /// Simulate a full file lifecycle over the Device I/O Request/Response
    /// exchange: create, write, read back, close — matching MS-RDPEFS
    /// 2.2.1.4/2.2.1.5, on top of the handshake already covered by
    /// `full_initialization_sequence`.
    #[test]
    fn full_device_io_sequence() {
        let device_id = 1;

        // Create.
        let create_req = DeviceCreateRequestPdu {
            device_id,
            completion_id: 100,
            desired_access: 0x0012_0089,
            allocation_size: 0,
            file_attributes: 0x80,
            shared_access: 0x0000_0007,
            create_disposition: 1, // FILE_OPEN
            create_options: 0,
            path: "\\file.bin".to_string(),
        }
        .encode();
        let decoded_create = DeviceCreateRequestPdu::decode(&create_req).unwrap();
        assert_eq!(decoded_create.path, "\\file.bin");

        let create_resp = DeviceCreateResponsePdu {
            device_id,
            completion_id: decoded_create.completion_id,
            io_status: 0,
            file_id: 42,
            information: Some(FILE_OPENED),
        }
        .encode();
        let file_id = DeviceCreateResponsePdu::decode(&create_resp)
            .unwrap()
            .file_id;
        assert_eq!(file_id, 42);

        // Write.
        let write_req = DeviceWriteRequestPdu {
            device_id,
            file_id,
            completion_id: 101,
            offset: 0,
            data: b"payload".to_vec(),
        }
        .encode();
        let decoded_write = DeviceWriteRequestPdu::decode(&write_req).unwrap();
        assert_eq!(decoded_write.data, b"payload");

        let write_resp = DeviceWriteResponsePdu {
            device_id,
            completion_id: decoded_write.completion_id,
            io_status: 0,
            length: decoded_write.data.len() as u32,
        }
        .encode();
        assert!(DeviceWriteResponsePdu::decode(&write_resp)
            .unwrap()
            .succeeded());

        // Read back.
        let read_req = DeviceReadRequestPdu {
            device_id,
            file_id,
            completion_id: 102,
            length: 7,
            offset: 0,
        }
        .encode();
        let decoded_read = DeviceReadRequestPdu::decode(&read_req).unwrap();

        let read_resp = DeviceReadResponsePdu {
            device_id,
            completion_id: decoded_read.completion_id,
            io_status: 0,
            data: b"payload".to_vec(),
        }
        .encode();
        assert_eq!(
            DeviceReadResponsePdu::decode(&read_resp).unwrap().data,
            b"payload"
        );

        // Close.
        let close_req = DeviceCloseRequestPdu {
            device_id,
            file_id,
            completion_id: 103,
        }
        .encode();
        let decoded_close = DeviceCloseRequestPdu::decode(&close_req).unwrap();

        let close_resp = DeviceCloseResponsePdu {
            device_id,
            completion_id: decoded_close.completion_id,
            io_status: 0,
        }
        .encode();
        assert!(DeviceCloseResponsePdu::decode(&close_resp)
            .unwrap()
            .succeeded());
    }
}
