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
//! **Not yet implemented:** the Device I/O Request/Response exchange
//! (`PAKID_CORE_DEVICE_IOREQUEST`/`IOCOMPLETION`) that carries the actual
//! file/printer/port/smart-card operations — this is the bulk of the
//! protocol's remaining surface and a substantial follow-up in its own
//! right — and `PAKID_CORE_DEVICELIST_REMOVE`.

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
}
