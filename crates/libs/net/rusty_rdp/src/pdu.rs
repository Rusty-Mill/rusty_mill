//! Share Control and Share Data headers (MS-RDPBCGR 2.2.8.1.1.1).
//!
//! Once the connection is up, almost every RDP PDU is wrapped in a **Share
//! Control Header**. Data PDUs additionally carry a **Share Data Header**,
//! which embeds a Share Control Header and adds the share id, stream id, and
//! the `pduType2` sub-type that identifies the payload (synchronize, control,
//! input, font list, update, …).
//!
//! ```text
//! Share Control Header (6 bytes)
//!   totalLength u16 | pduType u16 | pduSource u16
//!
//! Share Data Header (18 bytes = control + 12)
//!   ...control (pduType = DATAPDU)... | shareId u32 | pad u8 | streamId u8
//!   | uncompressedLength u16 | pduType2 u8 | compressedType u8
//!   | compressedLength u16
//! ```
//!
//! These headers sit inside the MCS Send Data user payload, so they are just
//! byte codecs here — framing them onto the wire is [`crate::mcs`]'s job.

use crate::cursor::{Reader, Writer};
use crate::error::{Error, Result};

/// `TS_PROTOCOL_VERSION` (1) shifted into the high bits of `pduType`.
const TS_PROTOCOL_VERSION: u16 = 0x0010;

/// Length of a Share Control Header in bytes.
pub const SHARE_CONTROL_HEADER_LEN: usize = 6;
/// Length of a full Share Data Header (control + 12) in bytes.
pub const SHARE_DATA_HEADER_LEN: usize = 18;

// PDUTYPE — the Share Control Header `pduType` low nibble (2.2.8.1.1.1.1).
/// Demand Active PDU (server capabilities).
pub const PDUTYPE_DEMANDACTIVEPDU: u16 = 1;
/// Confirm Active PDU (client capabilities).
pub const PDUTYPE_CONFIRMACTIVEPDU: u16 = 3;
/// Deactivate All PDU.
pub const PDUTYPE_DEACTIVATEALLPDU: u16 = 6;
/// Data PDU (carries a Share Data Header).
pub const PDUTYPE_DATAPDU: u16 = 7;
/// Server Redirection Packet.
pub const PDUTYPE_SERVER_REDIR_PKT: u16 = 10;

// PDUTYPE2 — the Share Data Header sub-type (2.2.8.1.1.1.2).
/// Update PDU (graphics / pointer updates).
pub const PDUTYPE2_UPDATE: u8 = 2;
/// Control PDU (cooperate / grant control).
pub const PDUTYPE2_CONTROL: u8 = 20;
/// Pointer PDU.
pub const PDUTYPE2_POINTER: u8 = 27;
/// Input event PDU.
pub const PDUTYPE2_INPUT: u8 = 28;
/// Synchronize PDU.
pub const PDUTYPE2_SYNCHRONIZE: u8 = 31;
/// Refresh Rect PDU.
pub const PDUTYPE2_REFRESH_RECT: u8 = 33;
/// Suppress Output PDU.
pub const PDUTYPE2_SUPPRESS_OUTPUT: u8 = 35;
/// Font List PDU.
pub const PDUTYPE2_FONTLIST: u8 = 39;
/// Font Map PDU.
pub const PDUTYPE2_FONTMAP: u8 = 40;
/// Set Error Info PDU (server disconnect reason).
pub const PDUTYPE2_SET_ERROR_INFO_PDU: u8 = 47;

// Stream identifiers for the Share Data Header.
/// `STREAM_LOW` priority.
pub const STREAM_LOW: u8 = 1;
/// `STREAM_MED` priority.
pub const STREAM_MED: u8 = 2;
/// `STREAM_HI` priority.
pub const STREAM_HI: u8 = 4;

/// A decoded Share Control Header.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct ShareControlHeader {
    /// The `PDUTYPE_*` value (protocol-version bits already stripped).
    pub pdu_type: u16,
    /// The sender's MCS user channel id.
    pub pdu_source: u16,
}

impl ShareControlHeader {
    /// Encode a Share Control PDU wrapping `payload`.
    pub fn encode(pdu_type: u16, pdu_source: u16, payload: &[u8]) -> Result<Vec<u8>> {
        let total = SHARE_CONTROL_HEADER_LEN + payload.len();
        if total > u16::MAX as usize {
            return Err(Error::Overflow {
                field: "share control totalLength",
            });
        }
        let mut w = Writer::with_capacity(total);
        w.write_u16_le(total as u16);
        w.write_u16_le(pdu_type | TS_PROTOCOL_VERSION);
        w.write_u16_le(pdu_source);
        w.write_bytes(payload);
        Ok(w.into_vec())
    }

    /// Decode a Share Control PDU, returning the header and its payload slice.
    pub fn decode(buf: &[u8]) -> Result<(ShareControlHeader, &[u8])> {
        let mut r = Reader::new(buf);
        let total_length = r.read_u16_le()? as usize;
        if total_length < SHARE_CONTROL_HEADER_LEN || total_length > buf.len() {
            return Err(Error::InvalidLength {
                field: "share control totalLength",
                length: total_length,
            });
        }
        let pdu_type = r.read_u16_le()? & 0x000F;
        let pdu_source = r.read_u16_le()?;
        let payload = &buf[SHARE_CONTROL_HEADER_LEN..total_length];
        Ok((
            ShareControlHeader {
                pdu_type,
                pdu_source,
            },
            payload,
        ))
    }
}

/// A decoded Share Data Header (the fields beyond the embedded control
/// header).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct ShareDataHeader {
    /// The share identifier assigned in the Demand Active PDU.
    pub share_id: u32,
    /// Stream priority (`STREAM_*`).
    pub stream_id: u8,
    /// Uncompressed payload length as carried on the wire.
    pub uncompressed_length: u16,
    /// The `PDUTYPE2_*` sub-type of the payload.
    pub pdu_type2: u8,
    /// Compression type (0 when uncompressed).
    pub compressed_type: u8,
    /// Compressed length (0 when uncompressed).
    pub compressed_length: u16,
}

impl ShareDataHeader {
    /// Build a header for an uncompressed `payload_len`-byte payload of the
    /// given sub-type, using low stream priority.
    ///
    /// `uncompressed_length` is set to include the 18-byte Share Data Header,
    /// per MS-RDPBCGR 2.2.8.1.1.1.2.
    pub fn new(share_id: u32, pdu_type2: u8, payload_len: usize) -> Self {
        ShareDataHeader {
            share_id,
            stream_id: STREAM_LOW,
            uncompressed_length: (payload_len + SHARE_DATA_HEADER_LEN) as u16,
            pdu_type2,
            compressed_type: 0,
            compressed_length: 0,
        }
    }

    /// Encode a Data PDU: this Share Data Header wrapping `payload`, inside a
    /// Share Control Header from `pdu_source`.
    pub fn encode(&self, pdu_source: u16, payload: &[u8]) -> Result<Vec<u8>> {
        let mut inner = Writer::with_capacity(12 + payload.len());
        inner.write_u32_le(self.share_id);
        inner.write_u8(0); // pad1
        inner.write_u8(self.stream_id);
        inner.write_u16_le(self.uncompressed_length);
        inner.write_u8(self.pdu_type2);
        inner.write_u8(self.compressed_type);
        inner.write_u16_le(self.compressed_length);
        inner.write_bytes(payload);
        ShareControlHeader::encode(PDUTYPE_DATAPDU, pdu_source, inner.as_slice())
    }

    /// Decode a Data PDU, returning `(pdu_source, header, payload)`.
    ///
    /// Returns [`Error::InvalidValue`] if the control header is not a Data
    /// PDU.
    pub fn decode(buf: &[u8]) -> Result<(u16, ShareDataHeader, &[u8])> {
        let (control, body) = ShareControlHeader::decode(buf)?;
        if control.pdu_type != PDUTYPE_DATAPDU {
            return Err(Error::InvalidValue {
                field: "share control pduType",
                value: control.pdu_type.to_string(),
            });
        }
        let mut r = Reader::new(body);
        let share_id = r.read_u32_le()?;
        let _pad1 = r.read_u8()?;
        let stream_id = r.read_u8()?;
        let uncompressed_length = r.read_u16_le()?;
        let pdu_type2 = r.read_u8()?;
        let compressed_type = r.read_u8()?;
        let compressed_length = r.read_u16_le()?;
        let payload = r.peek_remaining();
        Ok((
            control.pdu_source,
            ShareDataHeader {
                share_id,
                stream_id,
                uncompressed_length,
                pdu_type2,
                compressed_type,
                compressed_length,
            },
            payload,
        ))
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn share_control_roundtrip() {
        let payload = [0xAA, 0xBB, 0xCC];
        let bytes = ShareControlHeader::encode(PDUTYPE_DEMANDACTIVEPDU, 1002, &payload).unwrap();
        // totalLength = 6 + 3 = 9; pduType = 1 | 0x10 = 0x11.
        assert_eq!(&bytes[..6], &[0x09, 0x00, 0x11, 0x00, 0xEA, 0x03]);
        let (hdr, decoded) = ShareControlHeader::decode(&bytes).unwrap();
        assert_eq!(hdr.pdu_type, PDUTYPE_DEMANDACTIVEPDU);
        assert_eq!(hdr.pdu_source, 1002);
        assert_eq!(decoded, &payload);
    }

    #[test]
    fn share_data_roundtrip() {
        let payload = [0x01, 0x02, 0x03, 0x04];
        let hdr = ShareDataHeader::new(0x0001_0000, PDUTYPE2_SYNCHRONIZE, payload.len());
        let bytes = hdr.encode(1007, &payload).unwrap();
        // Outer control header marks a DATAPDU.
        let (control, _) = ShareControlHeader::decode(&bytes).unwrap();
        assert_eq!(control.pdu_type, PDUTYPE_DATAPDU);

        let (source, decoded_hdr, decoded_payload) = ShareDataHeader::decode(&bytes).unwrap();
        assert_eq!(source, 1007);
        assert_eq!(decoded_hdr, hdr);
        assert_eq!(decoded_payload, &payload);
        assert_eq!(decoded_hdr.pdu_type2, PDUTYPE2_SYNCHRONIZE);
        // uncompressedLength includes the 18-byte header.
        assert_eq!(
            decoded_hdr.uncompressed_length as usize,
            payload.len() + SHARE_DATA_HEADER_LEN
        );
    }

    #[test]
    fn decode_rejects_non_data_pdu() {
        let bytes = ShareControlHeader::encode(PDUTYPE_CONFIRMACTIVEPDU, 1007, &[0; 12]).unwrap();
        assert!(matches!(
            ShareDataHeader::decode(&bytes).unwrap_err(),
            Error::InvalidValue {
                field: "share control pduType",
                ..
            }
        ));
    }

    #[test]
    fn decode_rejects_bogus_total_length() {
        // totalLength claims 100 bytes but only 6 are present.
        let bytes = [0x64, 0x00, 0x11, 0x00, 0xEA, 0x03];
        assert!(matches!(
            ShareControlHeader::decode(&bytes).unwrap_err(),
            Error::InvalidLength { .. }
        ));
    }
}
