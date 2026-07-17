//! Server pointer (cursor) update PDUs (MS-RDPBCGR 2.2.9.1.1.4).
//!
//! The server updates the mouse cursor with a Share Data PDU
//! ([`crate::pdu`]) of sub-type `PDUTYPE2_POINTER`. The body is a
//! `TS_POINTER_PDU`: a 2-byte `messageType`, 2 bytes of padding, and a
//! type-specific payload:
//!
//! * **System** — a predefined cursor (`SYSPTR_NULL` = hidden,
//!   `SYSPTR_DEFAULT` = arrow).
//! * **Position** — move the cursor to `(x, y)`.
//! * **Color** — a new color cursor bitmap (XOR color data + 1bpp AND mask).
//! * **New** — like Color but with an explicit `xorBpp`.
//! * **Cached** — select a previously sent cursor by cache index.
//!
//! [`ColorPointer::to_rgba`] renders a color cursor to an RGBA image with a
//! transparency alpha channel derived from the AND mask.

use crate::cursor::{Reader, Writer};
use crate::error::{Error, Result};
use crate::pdu::{ShareDataHeader, PDUTYPE2_POINTER};

// TS_POINTER_PDU messageType values (2.2.9.1.1.4).
/// `TS_PTRMSGTYPE_SYSTEM` — a predefined system cursor.
pub const TS_PTRMSGTYPE_SYSTEM: u16 = 0x0001;
/// `TS_PTRMSGTYPE_POSITION` — a cursor position update.
pub const TS_PTRMSGTYPE_POSITION: u16 = 0x0003;
/// `TS_PTRMSGTYPE_COLOR` — a color cursor bitmap.
pub const TS_PTRMSGTYPE_COLOR: u16 = 0x0006;
/// `TS_PTRMSGTYPE_CACHED` — select a cached cursor.
pub const TS_PTRMSGTYPE_CACHED: u16 = 0x0007;
/// `TS_PTRMSGTYPE_POINTER` — a new cursor with an explicit `xorBpp`.
pub const TS_PTRMSGTYPE_POINTER: u16 = 0x0008;

/// `SYSPTR_NULL` — the hidden system cursor.
pub const SYSPTR_NULL: u32 = 0x0000_0000;
/// `SYSPTR_DEFAULT` — the default arrow system cursor.
pub const SYSPTR_DEFAULT: u32 = 0x0000_7F00;

/// Color depth assumed for a `TS_PTRMSGTYPE_COLOR` cursor's XOR data.
pub const COLOR_POINTER_BPP: u16 = 24;

/// A color cursor bitmap (`TS_COLORPOINTERATTRIBUTE`, 2.2.9.1.1.4.4).
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ColorPointer {
    /// Cache slot the server stores this cursor in.
    pub cache_index: u16,
    /// Hotspot x offset within the cursor.
    pub hotspot_x: u16,
    /// Hotspot y offset within the cursor.
    pub hotspot_y: u16,
    /// Cursor width in pixels.
    pub width: u16,
    /// Cursor height in pixels.
    pub height: u16,
    /// XOR (color) mask data, bottom-up, scanlines padded to 2 bytes.
    pub xor_mask: Vec<u8>,
    /// AND (transparency) mask, 1bpp bottom-up, scanlines padded to 2 bytes.
    pub and_mask: Vec<u8>,
}

impl ColorPointer {
    fn encode(&self, w: &mut Writer) {
        w.write_u16_le(self.cache_index);
        w.write_u16_le(self.hotspot_x);
        w.write_u16_le(self.hotspot_y);
        w.write_u16_le(self.width);
        w.write_u16_le(self.height);
        w.write_u16_le(self.and_mask.len() as u16);
        w.write_u16_le(self.xor_mask.len() as u16);
        w.write_bytes(&self.xor_mask);
        w.write_bytes(&self.and_mask);
    }

    /// Read a `TS_COLORPOINTERATTRIBUTE` from `r` (also used by fast-path).
    pub fn read(r: &mut Reader<'_>) -> Result<ColorPointer> {
        let cache_index = r.read_u16_le()?;
        let hotspot_x = r.read_u16_le()?;
        let hotspot_y = r.read_u16_le()?;
        let width = r.read_u16_le()?;
        let height = r.read_u16_le()?;
        let and_len = r.read_u16_le()? as usize;
        let xor_len = r.read_u16_le()? as usize;
        let xor_mask = r.read_bytes(xor_len)?.to_vec();
        let and_mask = r.read_bytes(and_len)?.to_vec();
        Ok(ColorPointer {
            cache_index,
            hotspot_x,
            hotspot_y,
            width,
            height,
            xor_mask,
            and_mask,
        })
    }

    /// Render this cursor to a top-down RGBA image.
    ///
    /// `xor_bpp` is the bit depth of the XOR color data (24 or 32). The alpha
    /// channel comes from the AND mask: a set bit marks a transparent pixel.
    pub fn to_rgba(&self, xor_bpp: u16) -> Result<Vec<u8>> {
        let bytes_pp = match xor_bpp {
            24 => 3usize,
            32 => 4usize,
            other => {
                return Err(Error::InvalidValue {
                    field: "pointer xorBpp",
                    value: other.to_string(),
                });
            }
        };
        let w = self.width as usize;
        let h = self.height as usize;
        // Scanlines are padded to a 2-byte boundary.
        let xor_stride = (w * bytes_pp).next_multiple_of_2();
        let and_stride = w.div_ceil_(16) * 2;
        if self.xor_mask.len() < xor_stride * h || self.and_mask.len() < and_stride * h {
            return Err(Error::UnexpectedEof {
                needed: xor_stride * h + and_stride * h,
                available: self.xor_mask.len() + self.and_mask.len(),
            });
        }

        let mut out = vec![0u8; w * h * 4];
        for y in 0..h {
            let src_row = h - 1 - y; // masks are bottom-up
            for x in 0..w {
                let xo = src_row * xor_stride + x * bytes_pp;
                let (b, g, r) = (
                    self.xor_mask[xo],
                    self.xor_mask[xo + 1],
                    self.xor_mask[xo + 2],
                );
                let and_byte = self.and_mask[src_row * and_stride + x / 8];
                let and_bit = (and_byte >> (7 - (x % 8))) & 1;
                let alpha = if and_bit == 1 { 0x00 } else { 0xFF };
                let dst = (y * w + x) * 4;
                out[dst] = r;
                out[dst + 1] = g;
                out[dst + 2] = b;
                out[dst + 3] = alpha;
            }
        }
        Ok(out)
    }
}

/// A decoded server pointer update.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum PointerUpdate {
    /// Select a predefined system cursor (`SYSPTR_*`).
    System(u32),
    /// Move the cursor to `(x, y)`.
    Position {
        /// Cursor x position.
        x: u16,
        /// Cursor y position.
        y: u16,
    },
    /// A color cursor bitmap (24bpp XOR data implied).
    Color(ColorPointer),
    /// A new cursor with an explicit XOR bit depth.
    New {
        /// Bit depth of the XOR color data.
        xor_bpp: u16,
        /// The cursor bitmap.
        pointer: ColorPointer,
    },
    /// Select a previously cached cursor by index.
    Cached(u16),
}

impl PointerUpdate {
    /// The `messageType` this update encodes as.
    pub fn message_type(&self) -> u16 {
        match self {
            PointerUpdate::System(_) => TS_PTRMSGTYPE_SYSTEM,
            PointerUpdate::Position { .. } => TS_PTRMSGTYPE_POSITION,
            PointerUpdate::Color(_) => TS_PTRMSGTYPE_COLOR,
            PointerUpdate::New { .. } => TS_PTRMSGTYPE_POINTER,
            PointerUpdate::Cached(_) => TS_PTRMSGTYPE_CACHED,
        }
    }

    fn encode_body(&self) -> Vec<u8> {
        let mut w = Writer::new();
        w.write_u16_le(self.message_type());
        w.write_u16_le(0); // pad2Octets
        match self {
            PointerUpdate::System(kind) => w.write_u32_le(*kind),
            PointerUpdate::Position { x, y } => {
                w.write_u16_le(*x);
                w.write_u16_le(*y);
            }
            PointerUpdate::Color(pointer) => pointer.encode(&mut w),
            PointerUpdate::New { xor_bpp, pointer } => {
                w.write_u16_le(*xor_bpp);
                pointer.encode(&mut w);
            }
            PointerUpdate::Cached(index) => w.write_u16_le(*index),
        }
        w.into_vec()
    }

    /// Encode as a Share Data PDU for `share_id`, sent from `pdu_source`.
    pub fn encode(&self, share_id: u32, pdu_source: u16) -> Result<Vec<u8>> {
        let body = self.encode_body();
        ShareDataHeader::new(share_id, PDUTYPE2_POINTER, body.len()).encode(pdu_source, &body)
    }

    /// Decode a Share Data pointer PDU, returning `(pdu_source, share_id, pdu)`.
    pub fn decode(buf: &[u8]) -> Result<(u16, u32, PointerUpdate)> {
        let (source, header, body) = ShareDataHeader::decode(buf)?;
        if header.pdu_type2 != PDUTYPE2_POINTER {
            return Err(Error::InvalidValue {
                field: "pduType2",
                value: header.pdu_type2.to_string(),
            });
        }
        let mut r = Reader::new(body);
        let message_type = r.read_u16_le()?;
        let _pad = r.read_u16_le()?;
        let update = match message_type {
            TS_PTRMSGTYPE_SYSTEM => PointerUpdate::System(r.read_u32_le()?),
            TS_PTRMSGTYPE_POSITION => PointerUpdate::Position {
                x: r.read_u16_le()?,
                y: r.read_u16_le()?,
            },
            TS_PTRMSGTYPE_COLOR => PointerUpdate::Color(ColorPointer::read(&mut r)?),
            TS_PTRMSGTYPE_POINTER => {
                let xor_bpp = r.read_u16_le()?;
                PointerUpdate::New {
                    xor_bpp,
                    pointer: ColorPointer::read(&mut r)?,
                }
            }
            TS_PTRMSGTYPE_CACHED => PointerUpdate::Cached(r.read_u16_le()?),
            other => {
                return Err(Error::InvalidValue {
                    field: "pointer messageType",
                    value: format!("0x{other:04X}"),
                });
            }
        };
        Ok((source, header.share_id, update))
    }
}

/// Round `self` up to the next even number.
trait NextMultipleOf2 {
    fn next_multiple_of_2(self) -> usize;
}
impl NextMultipleOf2 for usize {
    fn next_multiple_of_2(self) -> usize {
        (self + 1) & !1
    }
}

/// Local `div_ceil` to keep the crate's MSRV below 1.73.
trait DivCeil {
    fn div_ceil_(self, d: usize) -> usize;
}
impl DivCeil for usize {
    fn div_ceil_(self, d: usize) -> usize {
        (self + d - 1) / d
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn roundtrip(pdu: PointerUpdate) {
        let bytes = pdu.encode(0x1234, 1002).unwrap();
        let (source, share_id, decoded) = PointerUpdate::decode(&bytes).unwrap();
        assert_eq!(source, 1002);
        assert_eq!(share_id, 0x1234);
        assert_eq!(decoded, pdu);
    }

    #[test]
    fn system_and_position_roundtrip() {
        roundtrip(PointerUpdate::System(SYSPTR_NULL));
        roundtrip(PointerUpdate::System(SYSPTR_DEFAULT));
        roundtrip(PointerUpdate::Position { x: 640, y: 480 });
    }

    #[test]
    fn cached_roundtrip() {
        roundtrip(PointerUpdate::Cached(3));
    }

    #[test]
    fn color_pointer_roundtrip() {
        let pointer = ColorPointer {
            cache_index: 0,
            hotspot_x: 0,
            hotspot_y: 0,
            width: 1,
            height: 1,
            xor_mask: vec![0x11, 0x22, 0x33, 0x00], // 1px 24bpp, padded to 4
            and_mask: vec![0x00, 0x00],             // 1px 1bpp, padded to 2
        };
        roundtrip(PointerUpdate::Color(pointer.clone()));
        roundtrip(PointerUpdate::New {
            xor_bpp: 24,
            pointer,
        });
    }

    #[test]
    fn color_pointer_renders_opaque_pixel() {
        // 1x1 cursor, blue=0x11 green=0x22 red=0x33, AND bit clear → opaque.
        let pointer = ColorPointer {
            cache_index: 0,
            hotspot_x: 0,
            hotspot_y: 0,
            width: 1,
            height: 1,
            xor_mask: vec![0x11, 0x22, 0x33, 0x00],
            and_mask: vec![0x00, 0x00],
        };
        assert_eq!(pointer.to_rgba(24).unwrap(), [0x33, 0x22, 0x11, 0xFF]);
    }

    #[test]
    fn color_pointer_and_mask_is_transparent() {
        // AND bit set for the single pixel → alpha 0.
        let pointer = ColorPointer {
            cache_index: 0,
            hotspot_x: 0,
            hotspot_y: 0,
            width: 1,
            height: 1,
            xor_mask: vec![0xFF, 0xFF, 0xFF, 0x00],
            and_mask: vec![0x80, 0x00], // top bit = pixel 0
        };
        let rgba = pointer.to_rgba(24).unwrap();
        assert_eq!(rgba[3], 0x00);
    }

    #[test]
    fn rejects_unknown_message_type() {
        let body = {
            let mut w = Writer::new();
            w.write_u16_le(0x00FF); // bogus messageType
            w.write_u16_le(0);
            w.into_vec()
        };
        let bytes = ShareDataHeader::new(1, PDUTYPE2_POINTER, body.len())
            .encode(1002, &body)
            .unwrap();
        assert!(matches!(
            PointerUpdate::decode(&bytes).unwrap_err(),
            Error::InvalidValue {
                field: "pointer messageType",
                ..
            }
        ));
    }
}
