//! Server graphics update PDUs (MS-RDPBCGR 2.2.9.1.1.3).
//!
//! After activation the server drives the display with **Update PDUs**, Share
//! Data PDUs ([`crate::pdu`]) of sub-type `PDUTYPE2_UPDATE`. The body starts
//! with a 2-byte `updateType` discriminator followed by type-specific data:
//!
//! * **Bitmap** — one or more rectangles of pixel data, each optionally
//!   RLE-compressed. The pixel/compressed stream is carried verbatim here;
//!   decoding it is the job of the bitmap decompressor (a later layer).
//! * **Palette** — a 256-entry color table for 8bpp sessions.
//! * **Synchronize** — a no-op marker.
//! * **Orders** — primary/secondary drawing orders, kept raw for now.

use crate::cursor::{Reader, Writer};
use crate::error::{Error, Result};
use crate::pdu::{ShareDataHeader, PDUTYPE2_UPDATE};

// updateType discriminator values (2.2.9.1.1.3.1).
/// Drawing orders update.
pub const UPDATETYPE_ORDERS: u16 = 0x0000;
/// Bitmap update.
pub const UPDATETYPE_BITMAP: u16 = 0x0001;
/// Palette update.
pub const UPDATETYPE_PALETTE: u16 = 0x0002;
/// Synchronize update.
pub const UPDATETYPE_SYNCHRONIZE: u16 = 0x0003;

// TS_BITMAP_DATA flags (2.2.9.1.1.3.1.2.2).
/// `BITMAP_COMPRESSION` — `bitmapDataStream` is RLE-compressed.
pub const BITMAP_COMPRESSION: u16 = 0x0001;
/// `NO_BITMAP_COMPRESSION_HDR` — the 8-byte compression header is omitted.
pub const NO_BITMAP_COMPRESSION_HDR: u16 = 0x0400;

/// Length of the optional `TS_CD_HEADER` bitmap compression header.
pub const BITMAP_COMPRESSION_HDR_LEN: usize = 8;

/// One rectangle of bitmap data (`TS_BITMAP_DATA`, 2.2.9.1.1.3.1.2.2).
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct BitmapData {
    /// Left bound of the destination rectangle (inclusive).
    pub dest_left: u16,
    /// Top bound (inclusive).
    pub dest_top: u16,
    /// Right bound (inclusive).
    pub dest_right: u16,
    /// Bottom bound (inclusive).
    pub dest_bottom: u16,
    /// Bitmap width in pixels.
    pub width: u16,
    /// Bitmap height in pixels.
    pub height: u16,
    /// Bits per pixel of the bitmap data.
    pub bits_per_pixel: u16,
    /// `flags` (`BITMAP_COMPRESSION`, `NO_BITMAP_COMPRESSION_HDR`).
    pub flags: u16,
    /// The `bitmapDataStream` verbatim — raw pixels, or the compressed stream
    /// including its `TS_CD_HEADER` when present.
    pub data: Vec<u8>,
}

impl BitmapData {
    /// An uncompressed rectangle at `(left, top)` of `width`×`height` pixels.
    ///
    /// The destination right/bottom bounds are computed as inclusive.
    pub fn uncompressed(
        left: u16,
        top: u16,
        width: u16,
        height: u16,
        bits_per_pixel: u16,
        pixels: Vec<u8>,
    ) -> Self {
        BitmapData {
            dest_left: left,
            dest_top: top,
            dest_right: left + width.saturating_sub(1),
            dest_bottom: top + height.saturating_sub(1),
            width,
            height,
            bits_per_pixel,
            flags: 0,
            data: pixels,
        }
    }

    /// Returns `true` if `data` is RLE-compressed.
    pub fn is_compressed(&self) -> bool {
        self.flags & BITMAP_COMPRESSION != 0
    }

    /// Returns `true` if a compressed stream includes the 8-byte
    /// `TS_CD_HEADER`.
    pub fn has_compression_header(&self) -> bool {
        self.is_compressed() && self.flags & NO_BITMAP_COMPRESSION_HDR == 0
    }

    /// The compressed payload with the `TS_CD_HEADER` stripped, when present.
    ///
    /// For an uncompressed or header-less stream this returns `data`
    /// unchanged.
    pub fn compressed_payload(&self) -> &[u8] {
        if self.has_compression_header() && self.data.len() >= BITMAP_COMPRESSION_HDR_LEN {
            &self.data[BITMAP_COMPRESSION_HDR_LEN..]
        } else {
            &self.data
        }
    }

    /// Return the raw little-endian pixel bytes for this rectangle,
    /// RLE-decompressing when necessary.
    ///
    /// For an uncompressed rectangle this clones `data`; for a compressed one
    /// it runs the interleaved RLE decoder ([`crate::rle`]) over the payload.
    pub fn decompressed(&self) -> Result<Vec<u8>> {
        if self.is_compressed() {
            crate::rle::decompress_bitmap(
                self.compressed_payload(),
                self.width as usize,
                self.height as usize,
                self.bits_per_pixel,
            )
        } else {
            Ok(self.data.clone())
        }
    }

    fn encode(&self, w: &mut Writer) -> Result<()> {
        if self.data.len() > u16::MAX as usize {
            return Err(Error::Overflow {
                field: "bitmapLength",
            });
        }
        w.write_u16_le(self.dest_left);
        w.write_u16_le(self.dest_top);
        w.write_u16_le(self.dest_right);
        w.write_u16_le(self.dest_bottom);
        w.write_u16_le(self.width);
        w.write_u16_le(self.height);
        w.write_u16_le(self.bits_per_pixel);
        w.write_u16_le(self.flags);
        w.write_u16_le(self.data.len() as u16);
        w.write_bytes(&self.data);
        Ok(())
    }

    fn decode(r: &mut Reader<'_>) -> Result<BitmapData> {
        let dest_left = r.read_u16_le()?;
        let dest_top = r.read_u16_le()?;
        let dest_right = r.read_u16_le()?;
        let dest_bottom = r.read_u16_le()?;
        let width = r.read_u16_le()?;
        let height = r.read_u16_le()?;
        let bits_per_pixel = r.read_u16_le()?;
        let flags = r.read_u16_le()?;
        let bitmap_length = r.read_u16_le()? as usize;
        let data = r.read_bytes(bitmap_length)?.to_vec();
        Ok(BitmapData {
            dest_left,
            dest_top,
            dest_right,
            dest_bottom,
            width,
            height,
            bits_per_pixel,
            flags,
            data,
        })
    }
}

/// A palette entry (`TS_PALETTE_ENTRY`).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct PaletteEntry {
    /// Red component.
    pub red: u8,
    /// Green component.
    pub green: u8,
    /// Blue component.
    pub blue: u8,
}

/// A palette update (`TS_UPDATE_PALETTE_DATA`, 2.2.9.1.1.3.1.1).
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct PaletteUpdate {
    /// The color table entries (256 for an 8bpp session).
    pub entries: Vec<PaletteEntry>,
}

/// A server graphics update PDU.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum UpdatePdu {
    /// Bitmap update: one or more pixel rectangles.
    Bitmap(Vec<BitmapData>),
    /// Palette update.
    Palette(PaletteUpdate),
    /// Synchronize update (no payload).
    Synchronize,
    /// Drawing orders, kept raw (the bytes after `updateType`).
    Orders(Vec<u8>),
}

impl UpdatePdu {
    /// The `updateType` discriminator for this PDU.
    pub fn update_type(&self) -> u16 {
        match self {
            UpdatePdu::Bitmap(_) => UPDATETYPE_BITMAP,
            UpdatePdu::Palette(_) => UPDATETYPE_PALETTE,
            UpdatePdu::Synchronize => UPDATETYPE_SYNCHRONIZE,
            UpdatePdu::Orders(_) => UPDATETYPE_ORDERS,
        }
    }

    fn encode_body(&self) -> Result<Vec<u8>> {
        let mut w = Writer::new();
        w.write_u16_le(self.update_type());
        match self {
            UpdatePdu::Bitmap(rects) => {
                if rects.len() > u16::MAX as usize {
                    return Err(Error::Overflow {
                        field: "numberRectangles",
                    });
                }
                w.write_u16_le(rects.len() as u16);
                for rect in rects {
                    rect.encode(&mut w)?;
                }
            }
            UpdatePdu::Palette(palette) => {
                w.write_u16_le(0); // pad2Octets
                w.write_u32_le(palette.entries.len() as u32);
                for e in &palette.entries {
                    w.write_u8(e.red);
                    w.write_u8(e.green);
                    w.write_u8(e.blue);
                }
            }
            UpdatePdu::Synchronize => {
                w.write_u16_le(0); // pad2Octets
            }
            UpdatePdu::Orders(data) => {
                w.write_bytes(data);
            }
        }
        Ok(w.into_vec())
    }

    /// Encode as a Share Data PDU for `share_id`, sent from `pdu_source`.
    pub fn encode(&self, share_id: u32, pdu_source: u16) -> Result<Vec<u8>> {
        let body = self.encode_body()?;
        ShareDataHeader::new(share_id, PDUTYPE2_UPDATE, body.len()).encode(pdu_source, &body)
    }

    /// Decode a Share Data update PDU, returning `(pdu_source, share_id, pdu)`.
    pub fn decode(buf: &[u8]) -> Result<(u16, u32, UpdatePdu)> {
        let (source, header, body) = ShareDataHeader::decode(buf)?;
        if header.pdu_type2 != PDUTYPE2_UPDATE {
            return Err(Error::InvalidValue {
                field: "pduType2",
                value: header.pdu_type2.to_string(),
            });
        }
        let mut r = Reader::new(body);
        let update_type = r.read_u16_le()?;
        let pdu = match update_type {
            UPDATETYPE_BITMAP => {
                let count = r.read_u16_le()? as usize;
                let mut rects = Vec::with_capacity(count);
                for _ in 0..count {
                    rects.push(BitmapData::decode(&mut r)?);
                }
                UpdatePdu::Bitmap(rects)
            }
            UPDATETYPE_PALETTE => {
                let _pad = r.read_u16_le()?;
                let count = r.read_u32_le()? as usize;
                let mut entries = Vec::with_capacity(count);
                for _ in 0..count {
                    let rgb = r.read_bytes(3)?;
                    entries.push(PaletteEntry {
                        red: rgb[0],
                        green: rgb[1],
                        blue: rgb[2],
                    });
                }
                UpdatePdu::Palette(PaletteUpdate { entries })
            }
            UPDATETYPE_SYNCHRONIZE => {
                let _pad = r.read_u16_le()?;
                UpdatePdu::Synchronize
            }
            UPDATETYPE_ORDERS => UpdatePdu::Orders(r.peek_remaining().to_vec()),
            other => {
                return Err(Error::InvalidValue {
                    field: "updateType",
                    value: format!("0x{other:04X}"),
                });
            }
        };
        Ok((source, header.share_id, pdu))
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn roundtrip(pdu: &UpdatePdu) {
        let bytes = pdu.encode(0x0001_00EA, 1002).unwrap();
        let (source, share_id, decoded) = UpdatePdu::decode(&bytes).unwrap();
        assert_eq!(source, 1002);
        assert_eq!(share_id, 0x0001_00EA);
        assert_eq!(&decoded, pdu);
    }

    #[test]
    fn uncompressed_bitmap_roundtrip() {
        // 2x2 rectangle of 16bpp pixels (8 bytes).
        let pixels = vec![0x00, 0xF8, 0xE0, 0x07, 0x1F, 0x00, 0xFF, 0xFF];
        let rect = BitmapData::uncompressed(10, 20, 2, 2, 16, pixels);
        assert_eq!(rect.dest_right, 11);
        assert_eq!(rect.dest_bottom, 21);
        assert!(!rect.is_compressed());
        roundtrip(&UpdatePdu::Bitmap(vec![rect]));
    }

    #[test]
    fn compressed_bitmap_stream_preserved() {
        // Compressed with the 8-byte TS_CD_HEADER present.
        let mut stream = vec![0u8; BITMAP_COMPRESSION_HDR_LEN];
        stream.extend_from_slice(&[0xAA, 0xBB, 0xCC]); // fake compressed payload
        let rect = BitmapData {
            dest_left: 0,
            dest_top: 0,
            dest_right: 63,
            dest_bottom: 63,
            width: 64,
            height: 64,
            bits_per_pixel: 16,
            flags: BITMAP_COMPRESSION,
            data: stream,
        };
        assert!(rect.is_compressed());
        assert!(rect.has_compression_header());
        assert_eq!(rect.compressed_payload(), &[0xAA, 0xBB, 0xCC]);
        roundtrip(&UpdatePdu::Bitmap(vec![rect]));
    }

    #[test]
    fn compressed_without_header() {
        let rect = BitmapData {
            dest_left: 0,
            dest_top: 0,
            dest_right: 7,
            dest_bottom: 7,
            width: 8,
            height: 8,
            bits_per_pixel: 16,
            flags: BITMAP_COMPRESSION | NO_BITMAP_COMPRESSION_HDR,
            data: vec![0x11, 0x22, 0x33],
        };
        assert!(rect.is_compressed());
        assert!(!rect.has_compression_header());
        assert_eq!(rect.compressed_payload(), &[0x11, 0x22, 0x33]);
    }

    #[test]
    fn decompressed_uncompressed_is_clone() {
        let rect = BitmapData::uncompressed(0, 0, 2, 1, 8, vec![0xAA, 0xBB]);
        assert_eq!(rect.decompressed().unwrap(), [0xAA, 0xBB]);
    }

    #[test]
    fn decompressed_runs_rle() {
        // A headerless compressed rectangle: colour run of 0x77, 3 pixels.
        let rect = BitmapData {
            dest_left: 0,
            dest_top: 0,
            dest_right: 2,
            dest_bottom: 0,
            width: 3,
            height: 1,
            bits_per_pixel: 8,
            flags: BITMAP_COMPRESSION | NO_BITMAP_COMPRESSION_HDR,
            data: vec![0x63, 0x77], // colour run, len 3
        };
        assert_eq!(rect.decompressed().unwrap(), [0x77, 0x77, 0x77]);
    }

    #[test]
    fn multi_rectangle_bitmap() {
        let a = BitmapData::uncompressed(0, 0, 1, 1, 16, vec![0x00, 0x00]);
        let b = BitmapData::uncompressed(1, 0, 1, 1, 16, vec![0xFF, 0xFF]);
        roundtrip(&UpdatePdu::Bitmap(vec![a, b]));
    }

    #[test]
    fn palette_roundtrip() {
        let entries: Vec<PaletteEntry> = (0..256)
            .map(|i| PaletteEntry {
                red: i as u8,
                green: (255 - i) as u8,
                blue: (i / 2) as u8,
            })
            .collect();
        let pdu = UpdatePdu::Palette(PaletteUpdate { entries });
        let bytes = pdu.encode(1, 1002).unwrap();
        // updateType(2) + pad(2) + numberColors(4) + 256*3.
        let (_, _, body) = ShareDataHeader::decode(&bytes).unwrap();
        assert_eq!(u16::from_le_bytes([body[0], body[1]]), UPDATETYPE_PALETTE);
        assert_eq!(
            u32::from_le_bytes([body[4], body[5], body[6], body[7]]),
            256
        );
        roundtrip(&pdu);
    }

    #[test]
    fn synchronize_and_orders_roundtrip() {
        roundtrip(&UpdatePdu::Synchronize);
        roundtrip(&UpdatePdu::Orders(vec![0x01, 0x02, 0x03, 0x04]));
    }

    #[test]
    fn rejects_unknown_update_type() {
        let mut body = Writer::new();
        body.write_u16_le(0x00FF); // bogus updateType
        let bytes = ShareDataHeader::new(1, PDUTYPE2_UPDATE, body.len())
            .encode(1002, body.as_slice())
            .unwrap();
        assert!(matches!(
            UpdatePdu::decode(&bytes).unwrap_err(),
            Error::InvalidValue {
                field: "updateType",
                ..
            }
        ));
    }

    #[test]
    fn rejects_wrong_pdu_type2() {
        let bytes = ShareDataHeader::new(1, crate::pdu::PDUTYPE2_INPUT, 2)
            .encode(1002, &[0, 0])
            .unwrap();
        assert!(matches!(
            UpdatePdu::decode(&bytes).unwrap_err(),
            Error::InvalidValue {
                field: "pduType2",
                ..
            }
        ));
    }
}
