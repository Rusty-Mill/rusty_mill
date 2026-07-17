//! Clipboard Virtual Channel Extension (MS-RDPECLIP), std-only.
//!
//! Clipboard redirection rides on the static virtual channel named
//! `"cliprdr"` (registered via [`crate::net::EstablishConfig::extra_channels`]
//! and framed by [`crate::vchan`], exactly like any other static channel —
//! unlike [`crate::gfx`]/[`crate::rfx`], it does not go through a dynamic
//! channel). This module is the wire codec for the PDUs carried on it.
//!
//! ## Initialization sequence (MS-RDPECLIP 1.3.2.1)
//!
//! 1. Server sends [`CapsPdu`] (optional — absence means default capabilities).
//! 2. Server sends [`MonitorReadyPdu`].
//! 3. Client sends [`CapsPdu`] (optional, same rule).
//! 4. Client sends [`FormatListPdu`] announcing what's on its clipboard.
//! 5. Server (or client) replies [`FormatListResponsePdu`], and may later
//!    send [`FormatDataRequestPdu`] for one of the announced formats,
//!    answered with [`FormatDataResponsePdu`].
//!
//! ## What's implemented
//!
//! The core PDUs needed for text clipboard sharing: [`MonitorReadyPdu`],
//! [`CapsPdu`]/[`GeneralCapabilitySet`], [`FormatListPdu`] (the Long Format
//! Name variant only — [`CapsPdu`] always advertises
//! [`CB_USE_LONG_FORMAT_NAMES`], which this module always sets, sidestepping
//! the ambiguous Short Format Name variant), [`FormatListResponsePdu`],
//! [`FormatDataRequestPdu`], and [`FormatDataResponsePdu`].
//!
//! **Not yet implemented:** file copy/paste (`CB_FILECONTENTS_REQUEST`/
//! `RESPONSE`, `CB_LOCK_CLIPDATA`/`UNLOCK_CLIPDATA`), `CB_TEMP_DIRECTORY`,
//! and the Short Format Name variant of the Format List PDU.

use crate::cursor::{Reader, Writer};
use crate::error::{Error, Result};

/// The static virtual channel name clipboard redirection registers under.
pub const CLIPRDR_CHANNEL_NAME: &str = "cliprdr";

// msgType values (MS-RDPECLIP 2.2.1, the CLIPRDR_HEADER's msgType field).
const CB_MONITOR_READY: u16 = 0x0001;
const CB_FORMAT_LIST: u16 = 0x0002;
const CB_FORMAT_LIST_RESPONSE: u16 = 0x0003;
const CB_FORMAT_DATA_REQUEST: u16 = 0x0004;
const CB_FORMAT_DATA_RESPONSE: u16 = 0x0005;
const CB_CLIP_CAPS: u16 = 0x0007;

// msgFlags values.
const CB_RESPONSE_OK: u16 = 0x0001;
const CB_RESPONSE_FAIL: u16 = 0x0002;

/// Well-known Clipboard Format ID: plain ANSI text.
pub const CF_TEXT: u32 = 1;
/// Well-known Clipboard Format ID: plain UTF-16LE text.
pub const CF_UNICODETEXT: u32 = 13;

/// `CB_CAPSTYPE_GENERAL`, the only capability set type this module encodes
/// or interprets.
const CB_CAPSTYPE_GENERAL: u16 = 0x0001;
/// `CB_CAPS_VERSION_2`.
const CB_CAPS_VERSION_2: u32 = 0x0000_0002;

/// `CB_USE_LONG_FORMAT_NAMES` — this module always sets it when encoding a
/// [`GeneralCapabilitySet`], and always emits/expects the Long Format Name
/// variant of [`FormatListPdu`] regardless of what the peer advertises.
pub const CB_USE_LONG_FORMAT_NAMES: u32 = 0x0000_0002;
/// `CB_STREAM_FILECLIP_ENABLED`.
pub const CB_STREAM_FILECLIP_ENABLED: u32 = 0x0000_0004;
/// `CB_FILECLIP_NO_FILE_PATHS`.
pub const CB_FILECLIP_NO_FILE_PATHS: u32 = 0x0000_0008;
/// `CB_CAN_LOCK_CLIPDATA`.
pub const CB_CAN_LOCK_CLIPDATA: u32 = 0x0000_0010;
/// `CB_HUGE_FILE_SUPPORT_ENABLED`.
pub const CB_HUGE_FILE_SUPPORT_ENABLED: u32 = 0x0000_0020;

fn wrap(msg_type: u16, msg_flags: u16, body: &[u8]) -> Vec<u8> {
    let mut w = Writer::with_capacity(8 + body.len());
    w.write_u16_le(msg_type);
    w.write_u16_le(msg_flags);
    w.write_u32_le(body.len() as u32);
    w.write_bytes(body);
    w.into_vec()
}

/// Read the `CLIPRDR_HEADER`, check `msgType` matches `expected`, and
/// return `(msgFlags, body reader)`.
fn unwrap<'a>(buf: &'a [u8], expected: u16) -> Result<(u16, Reader<'a>)> {
    let mut r = Reader::new(buf);
    let msg_type = r.read_u16_le()?;
    let msg_flags = r.read_u16_le()?;
    let data_len = r.read_u32_le()? as usize;
    if msg_type != expected {
        return Err(Error::InvalidValue {
            field: "CLIPRDR_HEADER msgType",
            value: format!("0x{msg_type:04X} (expected 0x{expected:04X})"),
        });
    }
    if data_len != r.remaining() {
        return Err(Error::InvalidLength {
            field: "CLIPRDR_HEADER dataLen",
            length: data_len,
        });
    }
    Ok((msg_flags, r))
}

/// Peek the `msgType` of an encoded PDU without consuming it, to pick the
/// right decoder.
pub fn decode_msg_type(buf: &[u8]) -> Result<u16> {
    let mut r = Reader::new(buf);
    r.read_u16_le()
}

fn read_wchar_z(r: &mut Reader<'_>) -> Result<String> {
    let mut units = Vec::new();
    loop {
        let u = r.read_u16_le()?;
        if u == 0 {
            break;
        }
        units.push(u);
    }
    Ok(String::from_utf16_lossy(&units))
}

fn write_wchar_z(w: &mut Writer, s: &str) {
    for u in s.encode_utf16() {
        w.write_u16_le(u);
    }
    w.write_u16_le(0);
}

/// `CLIPRDR_MONITOR_READY` — sent by the server once initialized, after any
/// [`CapsPdu`].
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub struct MonitorReadyPdu;

impl MonitorReadyPdu {
    /// Encode to bytes.
    pub fn encode(&self) -> Vec<u8> {
        wrap(CB_MONITOR_READY, 0, &[])
    }

    /// Decode from bytes.
    pub fn decode(buf: &[u8]) -> Result<MonitorReadyPdu> {
        unwrap(buf, CB_MONITOR_READY)?;
        Ok(MonitorReadyPdu)
    }
}

/// `CLIPRDR_GENERAL_CAPABILITY` (`CB_CAPSTYPE_GENERAL`) — the only
/// capability set this module builds or interprets.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct GeneralCapabilitySet {
    /// `CB_CAPS_VERSION_1` or `CB_CAPS_VERSION_2`; informational only.
    pub version: u32,
    /// `CB_USE_LONG_FORMAT_NAMES` / `CB_STREAM_FILECLIP_ENABLED` / etc.
    pub general_flags: u32,
}

impl Default for GeneralCapabilitySet {
    fn default() -> Self {
        GeneralCapabilitySet {
            version: CB_CAPS_VERSION_2,
            general_flags: CB_USE_LONG_FORMAT_NAMES,
        }
    }
}

impl GeneralCapabilitySet {
    fn encode_into(&self, w: &mut Writer) {
        w.write_u16_le(CB_CAPSTYPE_GENERAL);
        w.write_u16_le(12); // lengthCapability: this set is always 12 bytes.
        w.write_u32_le(self.version);
        w.write_u32_le(self.general_flags);
    }
}

/// One entry of `CLIPRDR_CAPS`'s `capabilitySets` array. Only the General
/// Capability Set is interpreted; any other type is preserved raw so a
/// caller can inspect it, but this module does not otherwise act on it.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum CapabilitySet {
    /// `CB_CAPSTYPE_GENERAL`.
    General(GeneralCapabilitySet),
    /// Any other `capabilitySetType`, preserved as raw capability data.
    Other {
        /// The unrecognized `capabilitySetType`.
        set_type: u16,
        /// The raw bytes following `lengthCapability`.
        data: Vec<u8>,
    },
}

impl CapabilitySet {
    fn encode_into(&self, w: &mut Writer) {
        match self {
            CapabilitySet::General(g) => g.encode_into(w),
            CapabilitySet::Other { set_type, data } => {
                w.write_u16_le(*set_type);
                w.write_u16_le((4 + data.len()) as u16);
                w.write_bytes(data);
            }
        }
    }

    fn decode_from(r: &mut Reader<'_>) -> Result<CapabilitySet> {
        let set_type = r.read_u16_le()?;
        let length = r.read_u16_le()? as usize;
        let data_len = length.checked_sub(4).ok_or(Error::InvalidLength {
            field: "CLIPRDR_CAPS_SET lengthCapability",
            length,
        })?;
        if set_type == CB_CAPSTYPE_GENERAL && data_len == 8 {
            let version = r.read_u32_le()?;
            let general_flags = r.read_u32_le()?;
            Ok(CapabilitySet::General(GeneralCapabilitySet {
                version,
                general_flags,
            }))
        } else {
            let data = r.read_bytes(data_len)?.to_vec();
            Ok(CapabilitySet::Other { set_type, data })
        }
    }
}

/// `CLIPRDR_CAPS` — exchanges capability information. Optional on the wire;
/// an endpoint that never sends one is assumed to use the default
/// [`GeneralCapabilitySet`].
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct CapsPdu {
    /// The advertised capability sets.
    pub sets: Vec<CapabilitySet>,
}

impl Default for CapsPdu {
    fn default() -> Self {
        CapsPdu {
            sets: vec![CapabilitySet::General(GeneralCapabilitySet::default())],
        }
    }
}

impl CapsPdu {
    /// Encode to bytes.
    pub fn encode(&self) -> Vec<u8> {
        let mut body = Writer::new();
        body.write_u16_le(self.sets.len() as u16);
        body.write_u16_le(0); // pad1
        for set in &self.sets {
            set.encode_into(&mut body);
        }
        wrap(CB_CLIP_CAPS, 0, body.as_slice())
    }

    /// Decode from bytes.
    pub fn decode(buf: &[u8]) -> Result<CapsPdu> {
        let (_flags, mut r) = unwrap(buf, CB_CLIP_CAPS)?;
        let count = r.read_u16_le()?;
        let _pad1 = r.read_u16_le()?;
        let mut sets = Vec::with_capacity(count as usize);
        for _ in 0..count {
            sets.push(CapabilitySet::decode_from(&mut r)?);
        }
        Ok(CapsPdu { sets })
    }

    /// The general capability set, if present (or the spec's implied
    /// default of all-zero flags/version 1 if this PDU carries no General
    /// Capability Set at all).
    pub fn general(&self) -> GeneralCapabilitySet {
        self.sets
            .iter()
            .find_map(|s| match s {
                CapabilitySet::General(g) => Some(*g),
                _ => None,
            })
            .unwrap_or(GeneralCapabilitySet {
                version: 0,
                general_flags: 0,
            })
    }
}

/// `CLIPRDR_FORMAT_LIST` (Long Format Name variant) — announces the
/// Clipboard Format ID/name pairs available on the sender's local
/// clipboard. An empty list indicates the clipboard has been emptied.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct FormatListPdu {
    /// `(formatId, name)` pairs; `name` is empty for formats with no name
    /// (encoded on the wire as a single NUL).
    pub formats: Vec<(u32, String)>,
}

impl FormatListPdu {
    /// Encode to bytes.
    pub fn encode(&self) -> Vec<u8> {
        let mut body = Writer::new();
        for (id, name) in &self.formats {
            body.write_u32_le(*id);
            write_wchar_z(&mut body, name);
        }
        wrap(CB_FORMAT_LIST, 0, body.as_slice())
    }

    /// Decode from bytes.
    pub fn decode(buf: &[u8]) -> Result<FormatListPdu> {
        let (_flags, mut r) = unwrap(buf, CB_FORMAT_LIST)?;
        let mut formats = Vec::new();
        while !r.is_empty() {
            let id = r.read_u32_le()?;
            let name = read_wchar_z(&mut r)?;
            formats.push((id, name));
        }
        Ok(FormatListPdu { formats })
    }
}

/// `CLIPRDR_FORMAT_LIST_RESPONSE` — acknowledges a [`FormatListPdu`].
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct FormatListResponsePdu {
    /// `true` for `CB_RESPONSE_OK`, `false` for `CB_RESPONSE_FAIL`.
    pub ok: bool,
}

impl FormatListResponsePdu {
    /// Encode to bytes.
    pub fn encode(&self) -> Vec<u8> {
        let flags = if self.ok {
            CB_RESPONSE_OK
        } else {
            CB_RESPONSE_FAIL
        };
        wrap(CB_FORMAT_LIST_RESPONSE, flags, &[])
    }

    /// Decode from bytes.
    pub fn decode(buf: &[u8]) -> Result<FormatListResponsePdu> {
        let (flags, _r) = unwrap(buf, CB_FORMAT_LIST_RESPONSE)?;
        Ok(FormatListResponsePdu {
            ok: flags & CB_RESPONSE_OK != 0,
        })
    }
}

/// `CLIPRDR_FORMAT_DATA_REQUEST` — requests the data for one of the
/// formats previously announced in a [`FormatListPdu`].
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct FormatDataRequestPdu {
    /// The requested Clipboard Format ID.
    pub requested_format_id: u32,
}

impl FormatDataRequestPdu {
    /// Encode to bytes.
    pub fn encode(&self) -> Vec<u8> {
        let mut body = Writer::new();
        body.write_u32_le(self.requested_format_id);
        wrap(CB_FORMAT_DATA_REQUEST, 0, body.as_slice())
    }

    /// Decode from bytes.
    pub fn decode(buf: &[u8]) -> Result<FormatDataRequestPdu> {
        let (_flags, mut r) = unwrap(buf, CB_FORMAT_DATA_REQUEST)?;
        Ok(FormatDataRequestPdu {
            requested_format_id: r.read_u32_le()?,
        })
    }
}

/// `CLIPRDR_FORMAT_DATA_RESPONSE` — replies to a [`FormatDataRequestPdu`]
/// with the requested clipboard data (generic bytes; this module does not
/// interpret the Packed Metafile/Palette payload variants).
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct FormatDataResponsePdu {
    /// `true` for `CB_RESPONSE_OK`, `false` for `CB_RESPONSE_FAIL`.
    pub ok: bool,
    /// The requested format's data (empty when `ok` is `false`).
    pub data: Vec<u8>,
}

impl FormatDataResponsePdu {
    /// Encode to bytes.
    pub fn encode(&self) -> Vec<u8> {
        let flags = if self.ok {
            CB_RESPONSE_OK
        } else {
            CB_RESPONSE_FAIL
        };
        wrap(CB_FORMAT_DATA_RESPONSE, flags, &self.data)
    }

    /// Decode from bytes.
    pub fn decode(buf: &[u8]) -> Result<FormatDataResponsePdu> {
        let (flags, mut r) = unwrap(buf, CB_FORMAT_DATA_RESPONSE)?;
        let data = r.read_bytes(r.remaining())?.to_vec();
        Ok(FormatDataResponsePdu {
            ok: flags & CB_RESPONSE_OK != 0,
            data,
        })
    }

    /// Decode `data` as UTF-16LE text (`CF_UNICODETEXT`), stripping one
    /// trailing NUL terminator if present.
    pub fn as_unicode_text(&self) -> String {
        let mut units: Vec<u16> = self
            .data
            .chunks_exact(2)
            .map(|c| u16::from_le_bytes([c[0], c[1]]))
            .collect();
        if units.last() == Some(&0) {
            units.pop();
        }
        String::from_utf16_lossy(&units)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn monitor_ready_wire_shape_and_roundtrip() {
        let pdu = MonitorReadyPdu;
        assert_eq!(
            pdu.encode(),
            vec![0x01, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00]
        );
        assert_eq!(MonitorReadyPdu::decode(&pdu.encode()).unwrap(), pdu);
    }

    #[test]
    fn decode_msg_type_reads_without_consuming() {
        let pdu = MonitorReadyPdu.encode();
        assert_eq!(decode_msg_type(&pdu).unwrap(), CB_MONITOR_READY);
    }

    #[test]
    fn wrong_msg_type_is_rejected() {
        let pdu = MonitorReadyPdu.encode();
        assert!(FormatListPdu::decode(&pdu).is_err());
    }

    #[test]
    fn truncated_data_len_is_rejected() {
        let mut pdu = MonitorReadyPdu.encode();
        pdu[4] = 5; // claim 5 bytes of body that aren't there
        assert!(MonitorReadyPdu::decode(&pdu).is_err());
    }

    #[test]
    fn caps_pdu_default_roundtrip() {
        let pdu = CapsPdu::default();
        let decoded = CapsPdu::decode(&pdu.encode()).unwrap();
        assert_eq!(decoded, pdu);
        assert_eq!(decoded.general().general_flags, CB_USE_LONG_FORMAT_NAMES);
    }

    #[test]
    fn caps_pdu_general_wire_shape() {
        let pdu = CapsPdu {
            sets: vec![CapabilitySet::General(GeneralCapabilitySet {
                version: CB_CAPS_VERSION_2,
                general_flags: CB_USE_LONG_FORMAT_NAMES | CB_STREAM_FILECLIP_ENABLED,
            })],
        };
        let encoded = pdu.encode();
        // header(8) + cCapabilitiesSets(2) + pad1(2) + capsSet(12) = 24.
        assert_eq!(encoded.len(), 24);
        assert_eq!(&encoded[0..2], &[0x07, 0x00]); // CB_CLIP_CAPS
        assert_eq!(&encoded[8..10], &[0x01, 0x00]); // cCapabilitiesSets = 1
        assert_eq!(&encoded[12..14], &[0x01, 0x00]); // capabilitySetType = GENERAL
        assert_eq!(&encoded[14..16], &[0x0C, 0x00]); // lengthCapability = 12
    }

    #[test]
    fn caps_pdu_preserves_unknown_set() {
        let pdu = CapsPdu {
            sets: vec![CapabilitySet::Other {
                set_type: 0x00FF,
                data: vec![0xAA, 0xBB, 0xCC],
            }],
        };
        assert_eq!(CapsPdu::decode(&pdu.encode()).unwrap(), pdu);
    }

    #[test]
    fn caps_pdu_no_general_set_reports_default() {
        let pdu = CapsPdu {
            sets: vec![CapabilitySet::Other {
                set_type: 0x00FF,
                data: vec![],
            }],
        };
        assert_eq!(pdu.general().general_flags, 0);
    }

    #[test]
    fn format_list_roundtrip_multiple_entries() {
        let pdu = FormatListPdu {
            formats: vec![
                (CF_UNICODETEXT, String::new()),
                (CF_TEXT, String::new()),
                (0xC000, "HTML Format".to_string()),
            ],
        };
        assert_eq!(FormatListPdu::decode(&pdu.encode()).unwrap(), pdu);
    }

    #[test]
    fn format_list_empty_is_the_clipboard_emptied_signal() {
        let pdu = FormatListPdu { formats: vec![] };
        let encoded = pdu.encode();
        assert_eq!(encoded.len(), 8); // header only, zero-length body.
        assert_eq!(FormatListPdu::decode(&encoded).unwrap(), pdu);
    }

    #[test]
    fn format_list_wire_shape_unnamed_format() {
        // formatId then a lone UTF-16 NUL when there's no name.
        let pdu = FormatListPdu {
            formats: vec![(CF_TEXT, String::new())],
        };
        let encoded = pdu.encode();
        assert_eq!(&encoded[8..12], &[0x01, 0x00, 0x00, 0x00]); // formatId=1 LE
        assert_eq!(&encoded[12..14], &[0x00, 0x00]); // lone NUL
    }

    #[test]
    fn format_list_response_roundtrip() {
        for ok in [true, false] {
            let pdu = FormatListResponsePdu { ok };
            assert_eq!(FormatListResponsePdu::decode(&pdu.encode()).unwrap(), pdu);
        }
    }

    #[test]
    fn format_data_request_roundtrip() {
        let pdu = FormatDataRequestPdu {
            requested_format_id: CF_UNICODETEXT,
        };
        assert_eq!(FormatDataRequestPdu::decode(&pdu.encode()).unwrap(), pdu);
    }

    #[test]
    fn format_data_response_roundtrip_and_unicode_text() {
        let mut data = Vec::new();
        for u in "hello".encode_utf16() {
            data.extend_from_slice(&u.to_le_bytes());
        }
        data.extend_from_slice(&0u16.to_le_bytes()); // NUL terminator
        let pdu = FormatDataResponsePdu { ok: true, data };
        let decoded = FormatDataResponsePdu::decode(&pdu.encode()).unwrap();
        assert_eq!(decoded, pdu);
        assert_eq!(decoded.as_unicode_text(), "hello");
    }

    #[test]
    fn format_data_response_failure_has_no_data() {
        let pdu = FormatDataResponsePdu {
            ok: false,
            data: vec![],
        };
        assert_eq!(FormatDataResponsePdu::decode(&pdu.encode()).unwrap(), pdu);
    }

    /// Simulate the full initialization handshake end to end, matching
    /// MS-RDPECLIP 1.3.2.1: caps exchange, monitor ready, then a format
    /// list announcing Unicode text, answered, then the data round trip.
    #[test]
    fn full_initialization_and_text_transfer_sequence() {
        let server_caps = CapsPdu::default().encode();
        assert_eq!(decode_msg_type(&server_caps).unwrap(), CB_CLIP_CAPS);

        let monitor_ready = MonitorReadyPdu.encode();
        assert_eq!(decode_msg_type(&monitor_ready).unwrap(), CB_MONITOR_READY);

        let client_caps = CapsPdu::default().encode();
        let general = CapsPdu::decode(&client_caps).unwrap().general();
        assert_ne!(general.general_flags & CB_USE_LONG_FORMAT_NAMES, 0);

        let format_list = FormatListPdu {
            formats: vec![(CF_UNICODETEXT, String::new())],
        }
        .encode();
        let formats = FormatListPdu::decode(&format_list).unwrap().formats;
        assert_eq!(formats, vec![(CF_UNICODETEXT, String::new())]);

        let response = FormatListResponsePdu { ok: true }.encode();
        assert!(FormatListResponsePdu::decode(&response).unwrap().ok);

        let request = FormatDataRequestPdu {
            requested_format_id: CF_UNICODETEXT,
        }
        .encode();
        let requested = FormatDataRequestPdu::decode(&request)
            .unwrap()
            .requested_format_id;
        assert_eq!(requested, CF_UNICODETEXT);

        let mut text_data = Vec::new();
        for u in "clipboard test".encode_utf16() {
            text_data.extend_from_slice(&u.to_le_bytes());
        }
        text_data.extend_from_slice(&0u16.to_le_bytes());
        let data_response = FormatDataResponsePdu {
            ok: true,
            data: text_data,
        }
        .encode();
        let decoded = FormatDataResponsePdu::decode(&data_response).unwrap();
        assert_eq!(decoded.as_unicode_text(), "clipboard test");
    }
}
