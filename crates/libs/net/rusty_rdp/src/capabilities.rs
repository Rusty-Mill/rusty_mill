//! Capability exchange (MS-RDPBCGR 2.2.1.13 / 2.2.7).
//!
//! After licensing, the server sends a **Demand Active** PDU advertising its
//! capability sets; the client answers with a **Confirm Active** PDU listing
//! the ones it supports. Each capability set is a `TS_CAPS_SET`:
//!
//! ```text
//! capabilitySetType u16 | lengthCapability u16 | capabilityData ...
//! ```
//!
//! where `lengthCapability` counts the four header bytes. This module models
//! the core sets a minimal client cares about as typed structs and preserves
//! every other set verbatim as [`CapabilitySet::Raw`]. Both PDUs are wrapped
//! in a Share Control Header ([`crate::pdu`]).

use crate::cursor::{Reader, Writer};
use crate::error::{Error, Result};
use crate::pdu::{ShareControlHeader, PDUTYPE_CONFIRMACTIVEPDU, PDUTYPE_DEMANDACTIVEPDU};

/// `originatorId` a client must echo in Confirm Active (the server channel).
pub const CONFIRM_ACTIVE_ORIGINATOR_ID: u16 = 0x03EA;

// Capability set types (2.2.7).
/// General capabilities.
pub const CAPSTYPE_GENERAL: u16 = 1;
/// Bitmap capabilities.
pub const CAPSTYPE_BITMAP: u16 = 2;
/// Order capabilities.
pub const CAPSTYPE_ORDER: u16 = 3;
/// Revision 1 bitmap cache.
pub const CAPSTYPE_BITMAPCACHE: u16 = 4;
/// Control capabilities.
pub const CAPSTYPE_CONTROL: u16 = 5;
/// Pointer capabilities.
pub const CAPSTYPE_POINTER: u16 = 8;
/// Share capabilities.
pub const CAPSTYPE_SHARE: u16 = 9;
/// Color table cache capabilities.
pub const CAPSTYPE_COLORCACHE: u16 = 10;
/// Sound capabilities.
pub const CAPSTYPE_SOUND: u16 = 12;
/// Input capabilities.
pub const CAPSTYPE_INPUT: u16 = 13;
/// Font capabilities.
pub const CAPSTYPE_FONT: u16 = 14;
/// Brush capabilities.
pub const CAPSTYPE_BRUSH: u16 = 15;
/// Glyph cache capabilities.
pub const CAPSTYPE_GLYPHCACHE: u16 = 16;
/// Offscreen bitmap cache capabilities.
pub const CAPSTYPE_OFFSCREENCACHE: u16 = 17;
/// Revision 2 bitmap cache.
pub const CAPSTYPE_BITMAPCACHE_REV2: u16 = 19;
/// Virtual channel capabilities.
pub const CAPSTYPE_VIRTUALCHANNEL: u16 = 20;

// General capability set extraFlags (2.2.7.1.1).
/// `FASTPATH_OUTPUT_SUPPORTED`.
pub const FASTPATH_OUTPUT_SUPPORTED: u16 = 0x0001;
/// `LONG_CREDENTIALS_SUPPORTED`.
pub const LONG_CREDENTIALS_SUPPORTED: u16 = 0x0004;
/// `AUTORECONNECT_SUPPORTED`.
pub const AUTORECONNECT_SUPPORTED: u16 = 0x0008;
/// `ENC_SALTED_CHECKSUM`.
pub const ENC_SALTED_CHECKSUM: u16 = 0x0010;
/// `NO_BITMAP_COMPRESSION_HDR`.
pub const NO_BITMAP_COMPRESSION_HDR: u16 = 0x0400;

// Input capability set inputFlags (2.2.7.1.6).
/// `INPUT_FLAG_SCANCODES`.
pub const INPUT_FLAG_SCANCODES: u16 = 0x0001;
/// `INPUT_FLAG_MOUSEX`.
pub const INPUT_FLAG_MOUSEX: u16 = 0x0004;
/// `INPUT_FLAG_FASTPATH_INPUT`.
pub const INPUT_FLAG_FASTPATH_INPUT: u16 = 0x0008;
/// `INPUT_FLAG_UNICODE`.
pub const INPUT_FLAG_UNICODE: u16 = 0x0010;
/// `INPUT_FLAG_FASTPATH_INPUT2`.
pub const INPUT_FLAG_FASTPATH_INPUT2: u16 = 0x0020;

/// `TS_CAPS_SET` header size (`capabilitySetType` + `lengthCapability`).
const CAPS_HEADER_LEN: usize = 4;

/// One capability set. Core sets are typed; everything else is preserved raw.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum CapabilitySet {
    /// `TS_GENERAL_CAPABILITYSET`.
    General(GeneralCapabilitySet),
    /// `TS_BITMAP_CAPABILITYSET`.
    Bitmap(BitmapCapabilitySet),
    /// `TS_POINTER_CAPABILITYSET`.
    Pointer(PointerCapabilitySet),
    /// `TS_INPUT_CAPABILITYSET`.
    Input(InputCapabilitySet),
    /// `TS_SHARE_CAPABILITYSET`.
    Share(ShareCapabilitySet),
    /// Any other capability set, kept verbatim.
    Raw {
        /// `capabilitySetType`.
        capability_type: u16,
        /// `capabilityData` (the bytes after the 4-byte header).
        data: Vec<u8>,
    },
}

impl CapabilitySet {
    /// The `capabilitySetType` of this set.
    pub fn capability_type(&self) -> u16 {
        match self {
            CapabilitySet::General(_) => CAPSTYPE_GENERAL,
            CapabilitySet::Bitmap(_) => CAPSTYPE_BITMAP,
            CapabilitySet::Pointer(_) => CAPSTYPE_POINTER,
            CapabilitySet::Input(_) => CAPSTYPE_INPUT,
            CapabilitySet::Share(_) => CAPSTYPE_SHARE,
            CapabilitySet::Raw {
                capability_type, ..
            } => *capability_type,
        }
    }

    fn body(&self) -> Vec<u8> {
        match self {
            CapabilitySet::General(c) => c.encode_body(),
            CapabilitySet::Bitmap(c) => c.encode_body(),
            CapabilitySet::Pointer(c) => c.encode_body(),
            CapabilitySet::Input(c) => c.encode_body(),
            CapabilitySet::Share(c) => c.encode_body(),
            CapabilitySet::Raw { data, .. } => data.clone(),
        }
    }

    fn encode(&self, w: &mut Writer) -> Result<()> {
        let body = self.body();
        let length = CAPS_HEADER_LEN + body.len();
        if length > u16::MAX as usize {
            return Err(Error::Overflow {
                field: "lengthCapability",
            });
        }
        w.write_u16_le(self.capability_type());
        w.write_u16_le(length as u16);
        w.write_bytes(&body);
        Ok(())
    }

    fn decode(r: &mut Reader<'_>) -> Result<CapabilitySet> {
        let capability_type = r.read_u16_le()?;
        let length = r.read_u16_le()? as usize;
        if length < CAPS_HEADER_LEN {
            return Err(Error::InvalidLength {
                field: "lengthCapability",
                length,
            });
        }
        let body = r.read_bytes(length - CAPS_HEADER_LEN)?;
        Ok(match capability_type {
            CAPSTYPE_GENERAL => CapabilitySet::General(GeneralCapabilitySet::decode_body(body)?),
            CAPSTYPE_BITMAP => CapabilitySet::Bitmap(BitmapCapabilitySet::decode_body(body)?),
            CAPSTYPE_POINTER => CapabilitySet::Pointer(PointerCapabilitySet::decode_body(body)?),
            CAPSTYPE_INPUT => CapabilitySet::Input(InputCapabilitySet::decode_body(body)?),
            CAPSTYPE_SHARE => CapabilitySet::Share(ShareCapabilitySet::decode_body(body)?),
            other => CapabilitySet::Raw {
                capability_type: other,
                data: body.to_vec(),
            },
        })
    }
}

// ---------------------------------------------------------------------------
// Typed capability sets
// ---------------------------------------------------------------------------

/// `TS_GENERAL_CAPABILITYSET` (2.2.7.1.1).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct GeneralCapabilitySet {
    /// `osMajorType`.
    pub os_major_type: u16,
    /// `osMinorType`.
    pub os_minor_type: u16,
    /// `protocolVersion` (`0x0200`).
    pub protocol_version: u16,
    /// `generalCompressionTypes`.
    pub general_compression_types: u16,
    /// `extraFlags` bitmask.
    pub extra_flags: u16,
    /// `updateCapabilityFlag`.
    pub update_capability_flag: u16,
    /// `remoteUnshareFlag`.
    pub remote_unshare_flag: u16,
    /// `generalCompressionLevel`.
    pub general_compression_level: u16,
    /// `refreshRectSupport`.
    pub refresh_rect_support: u8,
    /// `suppressOutputSupport`.
    pub suppress_output_support: u8,
}

impl Default for GeneralCapabilitySet {
    fn default() -> Self {
        GeneralCapabilitySet {
            os_major_type: 1, // OSMAJORTYPE_WINDOWS
            os_minor_type: 3, // OSMINORTYPE_WINDOWS_NT
            protocol_version: 0x0200,
            general_compression_types: 0,
            extra_flags: FASTPATH_OUTPUT_SUPPORTED | LONG_CREDENTIALS_SUPPORTED,
            update_capability_flag: 0,
            remote_unshare_flag: 0,
            general_compression_level: 0,
            refresh_rect_support: 0,
            suppress_output_support: 0,
        }
    }
}

impl GeneralCapabilitySet {
    fn encode_body(&self) -> Vec<u8> {
        let mut w = Writer::with_capacity(20);
        w.write_u16_le(self.os_major_type);
        w.write_u16_le(self.os_minor_type);
        w.write_u16_le(self.protocol_version);
        w.write_u16_le(0); // pad2octetsA
        w.write_u16_le(self.general_compression_types);
        w.write_u16_le(self.extra_flags);
        w.write_u16_le(self.update_capability_flag);
        w.write_u16_le(self.remote_unshare_flag);
        w.write_u16_le(self.general_compression_level);
        w.write_u8(self.refresh_rect_support);
        w.write_u8(self.suppress_output_support);
        w.into_vec()
    }

    fn decode_body(body: &[u8]) -> Result<GeneralCapabilitySet> {
        let mut r = Reader::new(body);
        let os_major_type = r.read_u16_le()?;
        let os_minor_type = r.read_u16_le()?;
        let protocol_version = r.read_u16_le()?;
        let _pad = r.read_u16_le()?;
        let general_compression_types = r.read_u16_le()?;
        let extra_flags = r.read_u16_le()?;
        let update_capability_flag = r.read_u16_le()?;
        let remote_unshare_flag = r.read_u16_le()?;
        let general_compression_level = r.read_u16_le()?;
        let refresh_rect_support = r.read_u8()?;
        let suppress_output_support = r.read_u8()?;
        Ok(GeneralCapabilitySet {
            os_major_type,
            os_minor_type,
            protocol_version,
            general_compression_types,
            extra_flags,
            update_capability_flag,
            remote_unshare_flag,
            general_compression_level,
            refresh_rect_support,
            suppress_output_support,
        })
    }
}

/// `TS_BITMAP_CAPABILITYSET` (2.2.7.1.2).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct BitmapCapabilitySet {
    /// `preferredBitsPerPixel`.
    pub preferred_bits_per_pixel: u16,
    /// `receive1BitPerPixel`.
    pub receive_1bpp: u16,
    /// `receive4BitsPerPixel`.
    pub receive_4bpp: u16,
    /// `receive8BitsPerPixel`.
    pub receive_8bpp: u16,
    /// `desktopWidth`.
    pub desktop_width: u16,
    /// `desktopHeight`.
    pub desktop_height: u16,
    /// `desktopResizeFlag`.
    pub desktop_resize_flag: u16,
    /// `bitmapCompressionFlag`.
    pub bitmap_compression_flag: u16,
    /// `highColorFlags`.
    pub high_color_flags: u8,
    /// `drawingFlags`.
    pub drawing_flags: u8,
    /// `multipleRectangleSupport`.
    pub multiple_rectangle_support: u16,
}

impl BitmapCapabilitySet {
    /// A typical client bitmap capability for a given desktop size and depth.
    pub fn new(width: u16, height: u16, bits_per_pixel: u16) -> Self {
        BitmapCapabilitySet {
            preferred_bits_per_pixel: bits_per_pixel,
            receive_1bpp: 1,
            receive_4bpp: 1,
            receive_8bpp: 1,
            desktop_width: width,
            desktop_height: height,
            desktop_resize_flag: 1,
            bitmap_compression_flag: 1,
            high_color_flags: 0,
            drawing_flags: 0,
            multiple_rectangle_support: 1,
        }
    }

    fn encode_body(&self) -> Vec<u8> {
        let mut w = Writer::with_capacity(24);
        w.write_u16_le(self.preferred_bits_per_pixel);
        w.write_u16_le(self.receive_1bpp);
        w.write_u16_le(self.receive_4bpp);
        w.write_u16_le(self.receive_8bpp);
        w.write_u16_le(self.desktop_width);
        w.write_u16_le(self.desktop_height);
        w.write_u16_le(0); // pad2octets
        w.write_u16_le(self.desktop_resize_flag);
        w.write_u16_le(self.bitmap_compression_flag);
        w.write_u8(self.high_color_flags);
        w.write_u8(self.drawing_flags);
        w.write_u16_le(self.multiple_rectangle_support);
        w.write_u16_le(0); // pad2octetsB
        w.into_vec()
    }

    fn decode_body(body: &[u8]) -> Result<BitmapCapabilitySet> {
        let mut r = Reader::new(body);
        let preferred_bits_per_pixel = r.read_u16_le()?;
        let receive_1bpp = r.read_u16_le()?;
        let receive_4bpp = r.read_u16_le()?;
        let receive_8bpp = r.read_u16_le()?;
        let desktop_width = r.read_u16_le()?;
        let desktop_height = r.read_u16_le()?;
        let _pad = r.read_u16_le()?;
        let desktop_resize_flag = r.read_u16_le()?;
        let bitmap_compression_flag = r.read_u16_le()?;
        let high_color_flags = r.read_u8()?;
        let drawing_flags = r.read_u8()?;
        let multiple_rectangle_support = r.read_u16_le()?;
        Ok(BitmapCapabilitySet {
            preferred_bits_per_pixel,
            receive_1bpp,
            receive_4bpp,
            receive_8bpp,
            desktop_width,
            desktop_height,
            desktop_resize_flag,
            bitmap_compression_flag,
            high_color_flags,
            drawing_flags,
            multiple_rectangle_support,
        })
    }
}

/// `TS_POINTER_CAPABILITYSET` (2.2.7.1.5).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct PointerCapabilitySet {
    /// `colorPointerFlag`.
    pub color_pointer_flag: u16,
    /// `colorPointerCacheSize`.
    pub color_pointer_cache_size: u16,
    /// `pointerCacheSize`.
    pub pointer_cache_size: u16,
}

impl Default for PointerCapabilitySet {
    fn default() -> Self {
        PointerCapabilitySet {
            color_pointer_flag: 1,
            color_pointer_cache_size: 20,
            pointer_cache_size: 21,
        }
    }
}

impl PointerCapabilitySet {
    fn encode_body(&self) -> Vec<u8> {
        let mut w = Writer::with_capacity(6);
        w.write_u16_le(self.color_pointer_flag);
        w.write_u16_le(self.color_pointer_cache_size);
        w.write_u16_le(self.pointer_cache_size);
        w.into_vec()
    }

    fn decode_body(body: &[u8]) -> Result<PointerCapabilitySet> {
        let mut r = Reader::new(body);
        Ok(PointerCapabilitySet {
            color_pointer_flag: r.read_u16_le()?,
            color_pointer_cache_size: r.read_u16_le()?,
            pointer_cache_size: r.read_u16_le()?,
        })
    }
}

/// `TS_INPUT_CAPABILITYSET` (2.2.7.1.6).
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct InputCapabilitySet {
    /// `inputFlags` bitmask.
    pub input_flags: u16,
    /// `keyboardLayout`.
    pub keyboard_layout: u32,
    /// `keyboardType`.
    pub keyboard_type: u32,
    /// `keyboardSubType`.
    pub keyboard_subtype: u32,
    /// `keyboardFunctionKey`.
    pub keyboard_function_key: u32,
    /// `imeFileName` (up to 31 characters).
    pub ime_file_name: String,
}

impl Default for InputCapabilitySet {
    fn default() -> Self {
        InputCapabilitySet {
            input_flags: INPUT_FLAG_SCANCODES | INPUT_FLAG_MOUSEX | INPUT_FLAG_UNICODE,
            keyboard_layout: 0x0409,
            keyboard_type: 4,
            keyboard_subtype: 0,
            keyboard_function_key: 12,
            ime_file_name: String::new(),
        }
    }
}

impl InputCapabilitySet {
    fn encode_body(&self) -> Vec<u8> {
        let mut w = Writer::with_capacity(84);
        w.write_u16_le(self.input_flags);
        w.write_u16_le(0); // pad2octetsA
        w.write_u32_le(self.keyboard_layout);
        w.write_u32_le(self.keyboard_type);
        w.write_u32_le(self.keyboard_subtype);
        w.write_u32_le(self.keyboard_function_key);
        write_utf16le_fixed(&mut w, &self.ime_file_name, 64);
        w.into_vec()
    }

    fn decode_body(body: &[u8]) -> Result<InputCapabilitySet> {
        let mut r = Reader::new(body);
        let input_flags = r.read_u16_le()?;
        let _pad = r.read_u16_le()?;
        let keyboard_layout = r.read_u32_le()?;
        let keyboard_type = r.read_u32_le()?;
        let keyboard_subtype = r.read_u32_le()?;
        let keyboard_function_key = r.read_u32_le()?;
        let ime_file_name = read_utf16le_fixed(r.read_bytes(64)?);
        Ok(InputCapabilitySet {
            input_flags,
            keyboard_layout,
            keyboard_type,
            keyboard_subtype,
            keyboard_function_key,
            ime_file_name,
        })
    }
}

/// `TS_SHARE_CAPABILITYSET` (2.2.7.2.4).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct ShareCapabilitySet {
    /// `nodeId` — the sender's channel id (or 0 in a client's Confirm Active).
    pub node_id: u16,
}

impl ShareCapabilitySet {
    fn encode_body(&self) -> Vec<u8> {
        let mut w = Writer::with_capacity(4);
        w.write_u16_le(self.node_id);
        w.write_u16_le(0); // pad2octets
        w.into_vec()
    }

    fn decode_body(body: &[u8]) -> Result<ShareCapabilitySet> {
        let mut r = Reader::new(body);
        Ok(ShareCapabilitySet {
            node_id: r.read_u16_le()?,
        })
    }
}

// ---------------------------------------------------------------------------
// Demand Active / Confirm Active PDUs
// ---------------------------------------------------------------------------

/// `TS_DEMAND_ACTIVE_PDU` (2.2.1.13.1) — the server's capability advertisement.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct DemandActive {
    /// The share id the server assigns to this session.
    pub share_id: u32,
    /// `sourceDescriptor` (typically `b"RDP\0"`).
    pub source_descriptor: Vec<u8>,
    /// The advertised capability sets.
    pub capability_sets: Vec<CapabilitySet>,
    /// `sessionId`.
    pub session_id: u32,
}

impl DemandActive {
    /// Encode as a Share Control PDU sent from `pdu_source`.
    pub fn encode(&self, pdu_source: u16) -> Result<Vec<u8>> {
        let caps = encode_capability_sets(&self.capability_sets)?;
        let mut body = Writer::new();
        body.write_u32_le(self.share_id);
        body.write_u16_le(self.source_descriptor.len() as u16);
        // lengthCombinedCapabilities = numberCapabilities + pad2 + caps.
        body.write_u16_le((4 + caps.len()) as u16);
        body.write_bytes(&self.source_descriptor);
        body.write_u16_le(self.capability_sets.len() as u16);
        body.write_u16_le(0); // pad2Octets
        body.write_bytes(&caps);
        body.write_u32_le(self.session_id);
        ShareControlHeader::encode(PDUTYPE_DEMANDACTIVEPDU, pdu_source, body.as_slice())
    }

    /// Decode from a Share Control PDU, returning `(pdu_source, pdu)`.
    pub fn decode(buf: &[u8]) -> Result<(u16, DemandActive)> {
        let (control, body) = ShareControlHeader::decode(buf)?;
        if control.pdu_type != PDUTYPE_DEMANDACTIVEPDU {
            return Err(Error::InvalidValue {
                field: "pduType",
                value: control.pdu_type.to_string(),
            });
        }
        let mut r = Reader::new(body);
        let share_id = r.read_u32_le()?;
        let len_source = r.read_u16_le()? as usize;
        let _len_combined = r.read_u16_le()?;
        let source_descriptor = r.read_bytes(len_source)?.to_vec();
        let number_caps = r.read_u16_le()? as usize;
        let _pad = r.read_u16_le()?;
        let capability_sets = decode_capability_sets(&mut r, number_caps)?;
        let session_id = r.read_u32_le()?;
        Ok((
            control.pdu_source,
            DemandActive {
                share_id,
                source_descriptor,
                capability_sets,
                session_id,
            },
        ))
    }
}

/// `TS_CONFIRM_ACTIVE_PDU` (2.2.1.13.2) — the client's capability confirmation.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ConfirmActive {
    /// The share id echoed from the Demand Active PDU.
    pub share_id: u32,
    /// `originatorId` (must be [`CONFIRM_ACTIVE_ORIGINATOR_ID`]).
    pub originator_id: u16,
    /// `sourceDescriptor` (typically `b"MSTSC\0"`).
    pub source_descriptor: Vec<u8>,
    /// The confirmed capability sets.
    pub capability_sets: Vec<CapabilitySet>,
}

impl ConfirmActive {
    /// Build a Confirm Active echoing `share_id`, with the given capability
    /// sets and the conventional originator id and source descriptor.
    pub fn new(share_id: u32, capability_sets: Vec<CapabilitySet>) -> Self {
        ConfirmActive {
            share_id,
            originator_id: CONFIRM_ACTIVE_ORIGINATOR_ID,
            source_descriptor: b"MSTSC\0".to_vec(),
            capability_sets,
        }
    }

    /// Encode as a Share Control PDU sent from `pdu_source`.
    pub fn encode(&self, pdu_source: u16) -> Result<Vec<u8>> {
        let caps = encode_capability_sets(&self.capability_sets)?;
        let mut body = Writer::new();
        body.write_u32_le(self.share_id);
        body.write_u16_le(self.originator_id);
        body.write_u16_le(self.source_descriptor.len() as u16);
        body.write_u16_le((4 + caps.len()) as u16);
        body.write_bytes(&self.source_descriptor);
        body.write_u16_le(self.capability_sets.len() as u16);
        body.write_u16_le(0); // pad2Octets
        body.write_bytes(&caps);
        ShareControlHeader::encode(PDUTYPE_CONFIRMACTIVEPDU, pdu_source, body.as_slice())
    }

    /// Decode from a Share Control PDU, returning `(pdu_source, pdu)`.
    pub fn decode(buf: &[u8]) -> Result<(u16, ConfirmActive)> {
        let (control, body) = ShareControlHeader::decode(buf)?;
        if control.pdu_type != PDUTYPE_CONFIRMACTIVEPDU {
            return Err(Error::InvalidValue {
                field: "pduType",
                value: control.pdu_type.to_string(),
            });
        }
        let mut r = Reader::new(body);
        let share_id = r.read_u32_le()?;
        let originator_id = r.read_u16_le()?;
        let len_source = r.read_u16_le()? as usize;
        let _len_combined = r.read_u16_le()?;
        let source_descriptor = r.read_bytes(len_source)?.to_vec();
        let number_caps = r.read_u16_le()? as usize;
        let _pad = r.read_u16_le()?;
        let capability_sets = decode_capability_sets(&mut r, number_caps)?;
        Ok((
            control.pdu_source,
            ConfirmActive {
                share_id,
                originator_id,
                source_descriptor,
                capability_sets,
            },
        ))
    }
}

fn encode_capability_sets(sets: &[CapabilitySet]) -> Result<Vec<u8>> {
    let mut w = Writer::new();
    for set in sets {
        set.encode(&mut w)?;
    }
    Ok(w.into_vec())
}

fn decode_capability_sets(r: &mut Reader<'_>, count: usize) -> Result<Vec<CapabilitySet>> {
    let mut sets = Vec::with_capacity(count);
    for _ in 0..count {
        sets.push(CapabilitySet::decode(r)?);
    }
    Ok(sets)
}

/// A minimal but complete client capability set for a Confirm Active PDU at
/// the given desktop size and color depth.
pub fn client_capability_sets(width: u16, height: u16, bits_per_pixel: u16) -> Vec<CapabilitySet> {
    vec![
        CapabilitySet::General(GeneralCapabilitySet::default()),
        CapabilitySet::Bitmap(BitmapCapabilitySet::new(width, height, bits_per_pixel)),
        CapabilitySet::Pointer(PointerCapabilitySet::default()),
        CapabilitySet::Input(InputCapabilitySet::default()),
        CapabilitySet::Share(ShareCapabilitySet { node_id: 0 }),
    ]
}

/// A minimal but complete server capability set for a Demand Active PDU at
/// the given desktop size and color depth. Real servers also advertise an
/// Order capability set (not modeled by this crate, which does not decode
/// drawing orders); a client that requires one to proceed will reject this.
pub fn server_capability_sets(width: u16, height: u16, bits_per_pixel: u16) -> Vec<CapabilitySet> {
    vec![
        CapabilitySet::General(GeneralCapabilitySet::default()),
        CapabilitySet::Bitmap(BitmapCapabilitySet::new(width, height, bits_per_pixel)),
        CapabilitySet::Pointer(PointerCapabilitySet::default()),
        CapabilitySet::Share(ShareCapabilitySet { node_id: 0 }),
    ]
}

// ---------------------------------------------------------------------------
// UTF-16LE fixed-field helpers
// ---------------------------------------------------------------------------

fn write_utf16le_fixed(w: &mut Writer, s: &str, byte_len: usize) {
    let max_units = byte_len / 2;
    let mut written = 0usize;
    for unit in s.encode_utf16() {
        if written / 2 >= max_units.saturating_sub(1) {
            break;
        }
        w.write_u16_le(unit);
        written += 2;
    }
    while written < byte_len {
        w.write_u8(0);
        written += 1;
    }
}

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
    fn general_caps_roundtrip_and_size() {
        let set = CapabilitySet::General(GeneralCapabilitySet::default());
        let mut w = Writer::new();
        set.encode(&mut w).unwrap();
        let bytes = w.into_vec();
        // type = 1, length = 24.
        assert_eq!(&bytes[..4], &[0x01, 0x00, 0x18, 0x00]);
        let mut r = Reader::new(&bytes);
        assert_eq!(CapabilitySet::decode(&mut r).unwrap(), set);
    }

    #[test]
    fn bitmap_caps_carry_desktop_size() {
        let set = CapabilitySet::Bitmap(BitmapCapabilitySet::new(1920, 1080, 32));
        let mut w = Writer::new();
        set.encode(&mut w).unwrap();
        let bytes = w.into_vec();
        // length = 28.
        assert_eq!(u16::from_le_bytes([bytes[2], bytes[3]]), 28);
        let mut r = Reader::new(&bytes);
        let CapabilitySet::Bitmap(decoded) = CapabilitySet::decode(&mut r).unwrap() else {
            panic!("expected bitmap");
        };
        assert_eq!(decoded.desktop_width, 1920);
        assert_eq!(decoded.desktop_height, 1080);
        assert_eq!(decoded.preferred_bits_per_pixel, 32);
    }

    #[test]
    fn input_caps_roundtrip() {
        let set = CapabilitySet::Input(InputCapabilitySet::default());
        let mut w = Writer::new();
        set.encode(&mut w).unwrap();
        // length = 88.
        assert_eq!(u16::from_le_bytes([w.as_slice()[2], w.as_slice()[3]]), 88);
        let mut r = Reader::new(w.as_slice());
        assert_eq!(CapabilitySet::decode(&mut r).unwrap(), set);
    }

    #[test]
    fn unknown_caps_preserved() {
        let set = CapabilitySet::Raw {
            capability_type: CAPSTYPE_ORDER,
            data: vec![0xAA; 84],
        };
        let mut w = Writer::new();
        set.encode(&mut w).unwrap();
        let mut r = Reader::new(w.as_slice());
        assert_eq!(CapabilitySet::decode(&mut r).unwrap(), set);
    }

    #[test]
    fn demand_active_roundtrip() {
        let pdu = DemandActive {
            share_id: 0x0001_00EA,
            source_descriptor: b"RDP\0".to_vec(),
            capability_sets: vec![
                CapabilitySet::General(GeneralCapabilitySet::default()),
                CapabilitySet::Bitmap(BitmapCapabilitySet::new(1024, 768, 16)),
                CapabilitySet::Raw {
                    capability_type: CAPSTYPE_ORDER,
                    data: vec![0; 84],
                },
            ],
            session_id: 0,
        };
        let bytes = pdu.encode(1002).unwrap();
        let (source, decoded) = DemandActive::decode(&bytes).unwrap();
        assert_eq!(source, 1002);
        assert_eq!(decoded, pdu);
    }

    #[test]
    fn confirm_active_roundtrip() {
        let pdu = ConfirmActive::new(0x0001_00EA, client_capability_sets(1280, 800, 32));
        let bytes = pdu.encode(1007).unwrap();
        let (source, decoded) = ConfirmActive::decode(&bytes).unwrap();
        assert_eq!(source, 1007);
        assert_eq!(decoded.originator_id, CONFIRM_ACTIVE_ORIGINATOR_ID);
        assert_eq!(decoded, pdu);
    }

    #[test]
    fn demand_active_with_server_capability_sets_roundtrips() {
        let pdu = DemandActive {
            share_id: 0x0001_00EA,
            source_descriptor: b"RDP\0".to_vec(),
            capability_sets: server_capability_sets(1024, 768, 16),
            session_id: 0,
        };
        let bytes = pdu.encode(1002).unwrap();
        let (source, decoded) = DemandActive::decode(&bytes).unwrap();
        assert_eq!(source, 1002);
        assert_eq!(decoded, pdu);
    }

    #[test]
    fn confirm_active_rejects_demand_active_bytes() {
        let demand = DemandActive {
            share_id: 1,
            source_descriptor: b"RDP\0".to_vec(),
            capability_sets: vec![],
            session_id: 0,
        };
        let bytes = demand.encode(1002).unwrap();
        assert!(ConfirmActive::decode(&bytes).is_err());
    }
}
