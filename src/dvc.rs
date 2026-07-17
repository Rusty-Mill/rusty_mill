//! Dynamic Virtual Channel PDU framing (MS-RDPEDYC), std-only.
//!
//! Static virtual channels (the ones [`crate::gcc::ClientNetworkData`]
//! registers at connect time) are few and fixed for the life of a session.
//! Almost everything built on top of RDP since Windows Vista — the RDPGFX
//! graphics pipeline (the prerequisite for RemoteFX/AVC420/AVC444), clipboard,
//! audio, drive/USB/smartcard/printer redirection — instead opens a *dynamic*
//! virtual channel by name, multiplexed inside one reserved static channel
//! named `"DRDYNVC"`. This module is the wire codec for that framing: a
//! caller decodes the PDUs coming off the `DRDYNVC` static channel and
//! dispatches by name (e.g. `"Microsoft::Windows::RDS::Graphics"` for RDPGFX)
//! to whatever DVC-based protocol is layered on top.
//!
//! ## PDU header
//!
//! Every PDU starts with one header byte: `Cmd` (bits 7-4) identifies the PDU
//! type, `Sp` (bits 3-2) is PDU-specific (unused, or `Pri`/priority class),
//! and `cbId` (bits 1-0) gives the width of the `ChannelId` field that
//! follows (1/2/4 bytes). [`DataFirstPdu`] reuses the same two low bits as
//! `Len`, the width of its `Length` prefix.
//!
//! ## What's implemented
//!
//! The core create/data/close/capability negotiation PDUs:
//! [`CreateRequestPdu`] / [`CreateResponsePdu`], [`DataFirstPdu`] /
//! [`DataPdu`], [`ClosePdu`], and the version 1/2/3 capability exchange
//! ([`CapsRequest`] / [`CapabilitiesResponsePdu`]).
//!
//! **Not yet implemented:** the compressed variants (`DATA_FIRST_COMPRESSED`
//! / `DATA_COMPRESSED`, which need the RDP8 bulk compressor) and the
//! soft-sync PDUs (session-reconnect optimization, version 3 only). A message
//! larger than one PDU can carry must be split across [`DataFirstPdu`] +
//! [`DataPdu`]s by the caller; [`max_data_len`] gives the per-PDU budget.

use crate::cursor::{Reader, Writer};
use crate::error::{Error, Result};

/// The static virtual channel name that carries all dynamic-channel traffic —
/// request this via [`crate::net::EstablishConfig::extra_channels`] to enable
/// DVC-based protocols (RDPGFX, redirection).
pub const DRDYNVC_CHANNEL_NAME: &str = "DRDYNVC";

// Cmd values (MS-RDPEDYC 2.2, the header's high nibble).
/// `DYNVC_CREATE_REQ` / `DYNVC_CREATE_RSP`.
pub const CMD_CREATE: u8 = 0x01;
/// `DYNVC_DATA_FIRST`.
pub const CMD_DATA_FIRST: u8 = 0x02;
/// `DYNVC_DATA`.
pub const CMD_DATA: u8 = 0x03;
/// `DYNVC_CLOSE`.
pub const CMD_CLOSE: u8 = 0x04;
/// `DYNVC_CAPS_*` (request or response).
pub const CMD_CAPABILITY: u8 = 0x05;

/// `DYNVC_CAPS_VERSION1`.
pub const CAPS_VERSION1: u16 = 0x0001;
/// `DYNVC_CAPS_VERSION2`.
pub const CAPS_VERSION2: u16 = 0x0002;
/// `DYNVC_CAPS_VERSION3`.
pub const CAPS_VERSION3: u16 = 0x0003;

/// The maximum size, in bytes, of one framed DVC PDU on the wire
/// (MS-RDPEDYC 2.2.3.1).
const MAX_PDU_LEN: usize = 1600;

/// The largest `Data` payload that fits in one PDU whose header (everything
/// before the data) is `header_len` bytes — `1600 - header_len`, per the
/// [`DataFirstPdu`]/[`DataPdu`] length rule. Used to size fragments when
/// splitting a message larger than one PDU across `DataFirst` + `Data` PDUs.
pub fn max_data_len(header_len: usize) -> usize {
    MAX_PDU_LEN.saturating_sub(header_len)
}

/// Pick the narrowest `cbId`/`Len` code (0/1/2) that fits `value`, and the
/// byte width it denotes.
fn narrowest_width(value: u32) -> (u8, usize) {
    if value <= u8::MAX as u32 {
        (0x00, 1)
    } else if value <= u16::MAX as u32 {
        (0x01, 2)
    } else {
        (0x02, 4)
    }
}

/// Byte width denoted by a `cbId`/`Len` code (0/1/2 → 1/2/4).
fn width_of(code: u8, field: &'static str) -> Result<usize> {
    match code {
        0x00 => Ok(1),
        0x01 => Ok(2),
        0x02 => Ok(4),
        other => Err(Error::InvalidValue {
            field,
            value: format!("0x{other:02X}"),
        }),
    }
}

fn write_header(w: &mut Writer, cmd: u8, sp: u8, cbid: u8) {
    w.write_u8((cmd << 4) | (sp << 2) | cbid);
}

/// The decoded header byte: `(cmd, sp, cbid)`.
fn read_header(r: &mut Reader<'_>) -> Result<(u8, u8, u8)> {
    let b = r.read_u8()?;
    Ok(((b >> 4) & 0x0F, (b >> 2) & 0x03, b & 0x03))
}

/// Peek the `Cmd` nibble of a PDU without consuming it, to route to the
/// right decoder (`Cmd` alone does not disambiguate a request from a
/// response — the caller's connection role does, exactly as with the
/// `Create` and `Capability` PDUs below).
pub fn peek_cmd(buf: &[u8]) -> Result<u8> {
    let first = buf.first().ok_or(Error::UnexpectedEof {
        needed: 1,
        available: 0,
    })?;
    Ok((first >> 4) & 0x0F)
}

fn write_variable(w: &mut Writer, width: usize, value: u32) {
    match width {
        1 => w.write_u8(value as u8),
        2 => w.write_u16_le(value as u16),
        _ => w.write_u32_le(value),
    }
}

fn read_variable(r: &mut Reader<'_>, width: usize) -> Result<u32> {
    match width {
        1 => Ok(r.read_u8()? as u32),
        2 => Ok(r.read_u16_le()? as u32),
        _ => r.read_u32_le(),
    }
}

/// Read a null-terminated ANSI string, consuming the terminator.
fn read_c_string(r: &mut Reader<'_>) -> Result<String> {
    let rest = r.peek_remaining();
    let end = rest
        .iter()
        .position(|&b| b == 0)
        .ok_or(Error::UnexpectedEof {
            needed: 1,
            available: rest.len(),
        })?;
    let bytes = r.read_bytes(end)?.to_vec();
    r.skip(1)?; // the terminator
    String::from_utf8(bytes).map_err(|_| Error::InvalidValue {
        field: "DVC ChannelName",
        value: "not valid UTF-8/ANSI".to_string(),
    })
}

fn write_c_string(w: &mut Writer, s: &str) {
    w.write_bytes(s.as_bytes());
    w.write_u8(0);
}

// ---------------------------------------------------------------------------
// Create
// ---------------------------------------------------------------------------

/// `DYNVC_CREATE_REQ` — sent by the server to ask the client to open a named
/// dynamic channel.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct CreateRequestPdu {
    /// Server-assigned channel id, unique within the static channel.
    pub channel_id: u32,
    /// The DVC-based protocol's registered name (e.g.
    /// `"Microsoft::Windows::RDS::Graphics"` for RDPGFX).
    pub channel_name: String,
}

impl CreateRequestPdu {
    /// Encode to bytes.
    pub fn encode(&self) -> Vec<u8> {
        let (cbid, width) = narrowest_width(self.channel_id);
        let mut w = Writer::new();
        write_header(&mut w, CMD_CREATE, 0, cbid);
        write_variable(&mut w, width, self.channel_id);
        write_c_string(&mut w, &self.channel_name);
        w.into_vec()
    }

    /// Decode from bytes.
    pub fn decode(buf: &[u8]) -> Result<CreateRequestPdu> {
        let mut r = Reader::new(buf);
        let (cmd, _sp, cbid) = read_header(&mut r)?;
        expect_cmd(cmd, CMD_CREATE)?;
        let channel_id = read_variable(&mut r, width_of(cbid, "DVC cbId")?)?;
        let channel_name = read_c_string(&mut r)?;
        Ok(CreateRequestPdu {
            channel_id,
            channel_name,
        })
    }
}

/// `DYNVC_CREATE_RSP` — the client's reply indicating whether the channel was
/// opened. `creation_status` is an HRESULT: zero or positive is success.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct CreateResponsePdu {
    /// The channel id echoed from the [`CreateRequestPdu`].
    pub channel_id: u32,
    /// HRESULT status code.
    pub creation_status: i32,
}

impl CreateResponsePdu {
    /// Encode to bytes.
    pub fn encode(&self) -> Vec<u8> {
        let (cbid, width) = narrowest_width(self.channel_id);
        let mut w = Writer::new();
        write_header(&mut w, CMD_CREATE, 0, cbid);
        write_variable(&mut w, width, self.channel_id);
        w.write_u32_le(self.creation_status as u32);
        w.into_vec()
    }

    /// Decode from bytes.
    pub fn decode(buf: &[u8]) -> Result<CreateResponsePdu> {
        let mut r = Reader::new(buf);
        let (cmd, _sp, cbid) = read_header(&mut r)?;
        expect_cmd(cmd, CMD_CREATE)?;
        let channel_id = read_variable(&mut r, width_of(cbid, "DVC cbId")?)?;
        let creation_status = r.read_u32_le()? as i32;
        Ok(CreateResponsePdu {
            channel_id,
            creation_status,
        })
    }

    /// `true` when `creation_status` indicates success (zero or positive).
    pub fn succeeded(&self) -> bool {
        self.creation_status >= 0
    }
}

// ---------------------------------------------------------------------------
// Data
// ---------------------------------------------------------------------------

/// `DYNVC_DATA_FIRST` — the first fragment of a message too large for one PDU.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct DataFirstPdu {
    /// The target channel.
    pub channel_id: u32,
    /// The full reassembled message length.
    pub total_length: u32,
    /// This PDU's chunk of the message.
    pub data: Vec<u8>,
}

impl DataFirstPdu {
    /// Encode to bytes.
    pub fn encode(&self) -> Vec<u8> {
        let (cbid, cbid_width) = narrowest_width(self.channel_id);
        let (len_code, len_width) = narrowest_width(self.total_length);
        let mut w = Writer::new();
        write_header(&mut w, CMD_DATA_FIRST, len_code, cbid);
        write_variable(&mut w, cbid_width, self.channel_id);
        write_variable(&mut w, len_width, self.total_length);
        w.write_bytes(&self.data);
        w.into_vec()
    }

    /// Decode from bytes.
    pub fn decode(buf: &[u8]) -> Result<DataFirstPdu> {
        let mut r = Reader::new(buf);
        let (cmd, len_code, cbid) = read_header(&mut r)?;
        expect_cmd(cmd, CMD_DATA_FIRST)?;
        let channel_id = read_variable(&mut r, width_of(cbid, "DVC cbId")?)?;
        let total_length = read_variable(&mut r, width_of(len_code, "DVC Len")?)?;
        let data = r.peek_remaining().to_vec();
        Ok(DataFirstPdu {
            channel_id,
            total_length,
            data,
        })
    }
}

/// `DYNVC_DATA` — a complete message, or a non-first fragment.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct DataPdu {
    /// The target channel.
    pub channel_id: u32,
    /// The message, or this fragment of one.
    pub data: Vec<u8>,
}

impl DataPdu {
    /// Encode to bytes.
    pub fn encode(&self) -> Vec<u8> {
        let (cbid, width) = narrowest_width(self.channel_id);
        let mut w = Writer::new();
        write_header(&mut w, CMD_DATA, 0, cbid);
        write_variable(&mut w, width, self.channel_id);
        w.write_bytes(&self.data);
        w.into_vec()
    }

    /// Decode from bytes.
    pub fn decode(buf: &[u8]) -> Result<DataPdu> {
        let mut r = Reader::new(buf);
        let (cmd, _sp, cbid) = read_header(&mut r)?;
        expect_cmd(cmd, CMD_DATA)?;
        let channel_id = read_variable(&mut r, width_of(cbid, "DVC cbId")?)?;
        let data = r.peek_remaining().to_vec();
        Ok(DataPdu { channel_id, data })
    }
}

// ---------------------------------------------------------------------------
// Close
// ---------------------------------------------------------------------------

/// `DYNVC_CLOSE` — sent by either side to tear down a channel; the same PDU
/// shape serves as both the close request and its acknowledgement.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct ClosePdu {
    /// The channel being closed.
    pub channel_id: u32,
}

impl ClosePdu {
    /// Encode to bytes.
    pub fn encode(&self) -> Vec<u8> {
        let (cbid, width) = narrowest_width(self.channel_id);
        let mut w = Writer::new();
        write_header(&mut w, CMD_CLOSE, 0, cbid);
        write_variable(&mut w, width, self.channel_id);
        w.into_vec()
    }

    /// Decode from bytes.
    pub fn decode(buf: &[u8]) -> Result<ClosePdu> {
        let mut r = Reader::new(buf);
        let (cmd, _sp, cbid) = read_header(&mut r)?;
        expect_cmd(cmd, CMD_CLOSE)?;
        let channel_id = read_variable(&mut r, width_of(cbid, "DVC cbId")?)?;
        Ok(ClosePdu { channel_id })
    }
}

// ---------------------------------------------------------------------------
// Capabilities
// ---------------------------------------------------------------------------

/// `DYNVC_CAPS_VERSION1/2/3` — the server's capability announcement that
/// opens the DVC manager handshake. Versions 2 and 3 add the same
/// `PriorityCharge0..3` bandwidth-allocation fields; version 3 additionally
/// signals support for the compressed data PDUs (not implemented here).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum CapsRequest {
    /// `DYNVC_CAPS_VERSION1`.
    V1,
    /// `DYNVC_CAPS_VERSION2`.
    V2 {
        /// Per-priority-class bandwidth charges (see MS-RDPEDYC 2.2.1.1.2).
        priority_charges: [u16; 4],
    },
    /// `DYNVC_CAPS_VERSION3`.
    V3 {
        /// Per-priority-class bandwidth charges.
        priority_charges: [u16; 4],
    },
}

impl CapsRequest {
    /// The `Version` field value for this variant.
    pub fn version(&self) -> u16 {
        match self {
            CapsRequest::V1 => CAPS_VERSION1,
            CapsRequest::V2 { .. } => CAPS_VERSION2,
            CapsRequest::V3 { .. } => CAPS_VERSION3,
        }
    }

    /// Encode to bytes.
    pub fn encode(&self) -> Vec<u8> {
        let mut w = Writer::new();
        write_header(&mut w, CMD_CAPABILITY, 0, 0);
        w.write_u8(0); // Pad
        w.write_u16_le(self.version());
        if let CapsRequest::V2 { priority_charges } | CapsRequest::V3 { priority_charges } = self {
            for charge in priority_charges {
                w.write_u16_le(*charge);
            }
        }
        w.into_vec()
    }

    /// Decode from bytes.
    pub fn decode(buf: &[u8]) -> Result<CapsRequest> {
        let mut r = Reader::new(buf);
        let (cmd, _sp, _cbid) = read_header(&mut r)?;
        expect_cmd(cmd, CMD_CAPABILITY)?;
        let _pad = r.read_u8()?;
        let version = r.read_u16_le()?;
        match version {
            CAPS_VERSION1 => Ok(CapsRequest::V1),
            CAPS_VERSION2 | CAPS_VERSION3 => {
                let mut charges = [0u16; 4];
                for charge in charges.iter_mut() {
                    *charge = r.read_u16_le()?;
                }
                Ok(if version == CAPS_VERSION2 {
                    CapsRequest::V2 {
                        priority_charges: charges,
                    }
                } else {
                    CapsRequest::V3 {
                        priority_charges: charges,
                    }
                })
            }
            other => Err(Error::InvalidValue {
                field: "DYNVC_CAPS Version",
                value: format!("0x{other:04X}"),
            }),
        }
    }
}

/// `DYNVC_CAPS_RSP` — the client's acknowledgement of the highest version
/// level it supports.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct CapabilitiesResponsePdu {
    /// The acknowledged `DYNVC_CAPS_VERSION*` value.
    pub version: u16,
}

impl CapabilitiesResponsePdu {
    /// Encode to bytes.
    pub fn encode(&self) -> Vec<u8> {
        let mut w = Writer::new();
        write_header(&mut w, CMD_CAPABILITY, 0, 0);
        w.write_u8(0); // Pad
        w.write_u16_le(self.version);
        w.into_vec()
    }

    /// Decode from bytes.
    pub fn decode(buf: &[u8]) -> Result<CapabilitiesResponsePdu> {
        let mut r = Reader::new(buf);
        let (cmd, _sp, _cbid) = read_header(&mut r)?;
        expect_cmd(cmd, CMD_CAPABILITY)?;
        let _pad = r.read_u8()?;
        let version = r.read_u16_le()?;
        Ok(CapabilitiesResponsePdu { version })
    }
}

fn expect_cmd(actual: u8, expected: u8) -> Result<()> {
    if actual != expected {
        return Err(Error::InvalidValue {
            field: "DVC Cmd",
            value: format!("0x{actual:02X} (expected 0x{expected:02X})"),
        });
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn header_byte_packs_cmd_sp_cbid() {
        // Cmd in the high nibble, Sp next, cbId the low two bits (confirmed
        // against Microsoft's WindowsProtocolTestSuites reference decoder).
        let mut w = Writer::new();
        write_header(&mut w, 0x05, 0x02, 0x01);
        assert_eq!(w.as_slice(), &[0b0101_1001]);
        let mut r = Reader::new(w.as_slice());
        assert_eq!(read_header(&mut r).unwrap(), (0x05, 0x02, 0x01));
    }

    #[test]
    fn create_request_roundtrip_all_widths() {
        for channel_id in [7u32, 300, 70_000] {
            let pdu = CreateRequestPdu {
                channel_id,
                channel_name: "Microsoft::Windows::RDS::Graphics".to_string(),
            };
            let encoded = pdu.encode();
            assert_eq!(CreateRequestPdu::decode(&encoded).unwrap(), pdu);
        }
    }

    #[test]
    fn create_request_wire_shape() {
        // channel_id = 3 fits in cbId=0 (1 byte); name is NUL-terminated ANSI.
        let pdu = CreateRequestPdu {
            channel_id: 3,
            channel_name: "AB".to_string(),
        };
        let encoded = pdu.encode();
        // header: Cmd=1, Sp=0, cbId=0 -> 0x10.
        assert_eq!(&encoded[..2], &[0x10, 0x03]);
        assert_eq!(&encoded[2..], b"AB\0");
    }

    #[test]
    fn create_response_roundtrip_and_status() {
        let ok = CreateResponsePdu {
            channel_id: 5,
            creation_status: 0,
        };
        assert!(ok.succeeded());
        assert_eq!(CreateResponsePdu::decode(&ok.encode()).unwrap(), ok);

        let failed = CreateResponsePdu {
            channel_id: 5,
            creation_status: -2147024809, // E_INVALIDARG
        };
        assert!(!failed.succeeded());
        assert_eq!(CreateResponsePdu::decode(&failed.encode()).unwrap(), failed);
    }

    #[test]
    fn data_first_and_data_roundtrip() {
        let first = DataFirstPdu {
            channel_id: 1002,
            total_length: 5000,
            data: vec![0xAB; 1590],
        };
        let encoded = first.encode();
        assert_eq!(DataFirstPdu::decode(&encoded).unwrap(), first);

        let rest = DataPdu {
            channel_id: 1002,
            data: vec![0xCD; 200],
        };
        assert_eq!(DataPdu::decode(&rest.encode()).unwrap(), rest);
    }

    #[test]
    fn data_first_wire_shape_minimal() {
        // channel_id=1 (cbId=0), total_length=10 (Len=0): both 1 byte fields.
        let pdu = DataFirstPdu {
            channel_id: 1,
            total_length: 10,
            data: vec![0x99],
        };
        let encoded = pdu.encode();
        // header: Cmd=2, Sp(Len)=0, cbId=0 -> 0x20.
        assert_eq!(encoded, vec![0x20, 0x01, 0x0A, 0x99]);
    }

    #[test]
    fn close_roundtrip() {
        let pdu = ClosePdu { channel_id: 70_000 };
        assert_eq!(ClosePdu::decode(&pdu.encode()).unwrap(), pdu);
    }

    #[test]
    fn caps_v1_wire_shape_and_roundtrip() {
        let req = CapsRequest::V1;
        let encoded = req.encode();
        // header: Cmd=5, Sp=0, cbId=0 -> 0x50; Pad=0; Version=0x0001 LE.
        assert_eq!(encoded, vec![0x50, 0x00, 0x01, 0x00]);
        assert_eq!(CapsRequest::decode(&encoded).unwrap(), req);
    }

    #[test]
    fn caps_v2_and_v3_roundtrip_with_priority_charges() {
        let charges = [936u16, 3276, 9362, 21845];
        for req in [
            CapsRequest::V2 {
                priority_charges: charges,
            },
            CapsRequest::V3 {
                priority_charges: charges,
            },
        ] {
            let encoded = req.encode();
            assert_eq!(encoded.len(), 12);
            assert_eq!(CapsRequest::decode(&encoded).unwrap(), req);
        }
    }

    #[test]
    fn capabilities_response_roundtrip() {
        for version in [CAPS_VERSION1, CAPS_VERSION2, CAPS_VERSION3] {
            let pdu = CapabilitiesResponsePdu { version };
            let encoded = pdu.encode();
            assert_eq!(encoded.len(), 4);
            assert_eq!(CapabilitiesResponsePdu::decode(&encoded).unwrap(), pdu);
        }
    }

    #[test]
    fn peek_cmd_routes_without_consuming() {
        let pdu = ClosePdu { channel_id: 9 }.encode();
        assert_eq!(peek_cmd(&pdu).unwrap(), CMD_CLOSE);
        // The full decode still works afterwards (peek doesn't consume).
        assert_eq!(ClosePdu::decode(&pdu).unwrap().channel_id, 9);
    }

    #[test]
    fn max_data_len_matches_spec_budget() {
        // A 1-byte cbId + 1-byte Len + 1-byte ChannelId + 1-byte header = 4.
        assert_eq!(max_data_len(4), 1596);
        assert_eq!(max_data_len(2000), 0);
    }

    #[test]
    fn wrong_cmd_is_rejected() {
        let close = ClosePdu { channel_id: 1 }.encode();
        assert!(DataPdu::decode(&close).is_err());
    }
}
