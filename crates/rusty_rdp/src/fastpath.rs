//! Fast-path input and output (MS-RDPBCGR 2.2.8.1.2 / 2.2.9.1.2).
//!
//! Fast-path is a compact framing that bypasses the TPKT / X.224 / MCS / Share
//! header stack for the bulk of a live session. A receiver tells the two apart
//! by the first byte: a slow-path TPKT packet begins with `0x03`, while a
//! fast-path PDU's low two bits (the *action*) are zero.
//!
//! * **Output** (server → client, [`parse_output_updates`]) packs one or more
//!   updates, each a 1-byte header (update code + fragmentation + compression)
//!   plus a 16-bit size and body. The bodies are the same update payloads as
//!   the slow path, minus the leading `updateType`.
//! * **Input** (client → server, [`encode_input_events`]) packs input events
//!   far more tightly than the slow path — a key press is two bytes.
//!
//! This module is the pure codec; the length prefix, optional MAC signature,
//! and RC4 encryption are assembled by [`crate::net`], which owns the session.

use crate::cursor::{Reader, Writer};
use crate::error::{Error, Result};
use crate::input::{
    InputEvent, KBDFLAGS_EXTENDED, KBDFLAGS_RELEASE, TS_SYNC_CAPS_LOCK, TS_SYNC_KANA_LOCK,
    TS_SYNC_NUM_LOCK, TS_SYNC_SCROLL_LOCK,
};
use crate::output::{parse_bitmap_rectangles, parse_palette, BitmapData, PaletteUpdate};
use crate::pointer::ColorPointer;

/// The fast-path action code that identifies fast-path (both directions).
pub const FASTPATH_ACTION: u8 = 0x0;
/// `FASTPATH_OUTPUT_SECURE_CHECKSUM` — the MAC uses the salted variant.
pub const FASTPATH_SECURE_CHECKSUM: u8 = 0x1;
/// `FASTPATH_OUTPUT_ENCRYPTED` — the body is RC4-encrypted with a MAC.
pub const FASTPATH_ENCRYPTED: u8 = 0x2;

// Fast-path output update codes (updateHeader bits 0-3).
/// Drawing orders.
pub const FASTPATH_UPDATETYPE_ORDERS: u8 = 0x0;
/// Bitmap update.
pub const FASTPATH_UPDATETYPE_BITMAP: u8 = 0x1;
/// Palette update.
pub const FASTPATH_UPDATETYPE_PALETTE: u8 = 0x2;
/// Synchronize.
pub const FASTPATH_UPDATETYPE_SYNCHRONIZE: u8 = 0x3;
/// Surface commands.
pub const FASTPATH_UPDATETYPE_SURFCMDS: u8 = 0x4;
/// Hidden (null) pointer.
pub const FASTPATH_UPDATETYPE_PTR_NULL: u8 = 0x5;
/// Default pointer.
pub const FASTPATH_UPDATETYPE_PTR_DEFAULT: u8 = 0x6;
/// Pointer position.
pub const FASTPATH_UPDATETYPE_PTR_POSITION: u8 = 0x8;
/// Color pointer.
pub const FASTPATH_UPDATETYPE_COLOR: u8 = 0x9;
/// Cached pointer.
pub const FASTPATH_UPDATETYPE_CACHED: u8 = 0xA;
/// New (explicit bpp) pointer.
pub const FASTPATH_UPDATETYPE_POINTER: u8 = 0xB;

/// `FASTPATH_OUTPUT_COMPRESSION_USED` in the updateHeader compression field.
pub const FASTPATH_OUTPUT_COMPRESSION_USED: u8 = 0x2;

// Fast-path input event codes (eventHeader bits 5-7).
const FASTPATH_INPUT_EVENT_SCANCODE: u8 = 0x0;
const FASTPATH_INPUT_EVENT_MOUSE: u8 = 0x1;
const FASTPATH_INPUT_EVENT_MOUSEX: u8 = 0x2;
const FASTPATH_INPUT_EVENT_SYNC: u8 = 0x3;
const FASTPATH_INPUT_EVENT_UNICODE: u8 = 0x4;

// Fast-path keyboard event flags (eventHeader bits 0-4).
const FASTPATH_INPUT_KBDFLAGS_RELEASE: u8 = 0x01;
const FASTPATH_INPUT_KBDFLAGS_EXTENDED: u8 = 0x02;

/// Returns `true` if `first_byte` begins a fast-path PDU (rather than TPKT).
pub fn is_fastpath(first_byte: u8) -> bool {
    first_byte & 0x03 == FASTPATH_ACTION
}

/// Read a fast-path length determinant (1 or 2 bytes) from `r`.
///
/// The length counts the whole PDU, including the header byte and the length
/// field itself.
pub fn read_length(r: &mut Reader<'_>) -> Result<usize> {
    let first = r.read_u8()?;
    if first & 0x80 != 0 {
        let second = r.read_u8()?;
        Ok((((first & 0x7F) as usize) << 8) | second as usize)
    } else {
        Ok(first as usize)
    }
}

/// Write a fast-path length determinant for a total PDU of `length` bytes.
pub fn write_length(w: &mut Writer, length: usize) -> Result<()> {
    if length <= 0x7F {
        w.write_u8(length as u8);
    } else if length <= 0x7FFF {
        w.write_u8(0x80 | (length >> 8) as u8);
        w.write_u8(length as u8);
    } else {
        return Err(Error::Overflow {
            field: "fast-path length",
        });
    }
    Ok(())
}

/// One decoded fast-path output update.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum FastPathUpdate {
    /// Bitmap rectangles.
    Bitmap(Vec<BitmapData>),
    /// Palette update.
    Palette(PaletteUpdate),
    /// Update synchronize marker.
    Synchronize,
    /// Hidden pointer.
    PointerHidden,
    /// Default pointer.
    PointerDefault,
    /// Pointer position update.
    PointerPosition {
        /// Cursor x.
        x: u16,
        /// Cursor y.
        y: u16,
    },
    /// Color pointer bitmap.
    PointerColor(ColorPointer),
    /// New pointer with explicit XOR bit depth.
    PointerNew {
        /// XOR bit depth.
        xor_bpp: u16,
        /// Cursor bitmap.
        pointer: ColorPointer,
    },
    /// Cached pointer selection.
    PointerCached(u16),
    /// Any update this crate does not decode (orders, surface commands, …),
    /// kept as `(update_code, body)`.
    Raw {
        /// The updateHeader update code.
        update_code: u8,
        /// The update body.
        data: Vec<u8>,
    },
}

/// Parse the update array from a decrypted fast-path output body.
///
/// Cross-PDU fragmentation is not reassembled; each update is decoded from its
/// own body (correct for the common single-fragment case).
pub fn parse_output_updates(body: &[u8]) -> Result<Vec<FastPathUpdate>> {
    let mut r = Reader::new(body);
    let mut updates = Vec::new();
    while r.remaining() > 0 {
        let header = r.read_u8()?;
        let update_code = header & 0x0F;
        let compression = (header >> 6) & 0x03;
        if compression & FASTPATH_OUTPUT_COMPRESSION_USED != 0 {
            let _compression_flags = r.read_u8()?;
        }
        let size = r.read_u16_le()? as usize;
        let data = r.read_bytes(size)?;
        updates.push(decode_update(update_code, data)?);
    }
    Ok(updates)
}

fn decode_update(update_code: u8, data: &[u8]) -> Result<FastPathUpdate> {
    Ok(match update_code {
        // The fast-path update code replaces the slow-path `updateType`, so
        // the bitmap and palette bodies start straight at their data.
        FASTPATH_UPDATETYPE_BITMAP => FastPathUpdate::Bitmap(parse_bitmap_rectangles(data)?),
        FASTPATH_UPDATETYPE_PALETTE => FastPathUpdate::Palette(parse_palette(data)?),
        FASTPATH_UPDATETYPE_SYNCHRONIZE => FastPathUpdate::Synchronize,
        FASTPATH_UPDATETYPE_PTR_NULL => FastPathUpdate::PointerHidden,
        FASTPATH_UPDATETYPE_PTR_DEFAULT => FastPathUpdate::PointerDefault,
        FASTPATH_UPDATETYPE_PTR_POSITION => {
            let mut r = Reader::new(data);
            FastPathUpdate::PointerPosition {
                x: r.read_u16_le()?,
                y: r.read_u16_le()?,
            }
        }
        FASTPATH_UPDATETYPE_COLOR => {
            let mut r = Reader::new(data);
            FastPathUpdate::PointerColor(ColorPointer::read(&mut r)?)
        }
        FASTPATH_UPDATETYPE_POINTER => {
            let mut r = Reader::new(data);
            let xor_bpp = r.read_u16_le()?;
            FastPathUpdate::PointerNew {
                xor_bpp,
                pointer: ColorPointer::read(&mut r)?,
            }
        }
        FASTPATH_UPDATETYPE_CACHED => {
            let mut r = Reader::new(data);
            FastPathUpdate::PointerCached(r.read_u16_le()?)
        }
        other => FastPathUpdate::Raw {
            update_code: other,
            data: data.to_vec(),
        },
    })
}

/// Encode a batch of input events as fast-path event bytes.
///
/// Returns `(number_of_events, event_bytes)`. The caller ([`crate::net`])
/// prepends the fast-path header and length and applies encryption.
pub fn encode_input_events(events: &[InputEvent]) -> (usize, Vec<u8>) {
    let mut w = Writer::new();
    for event in events {
        encode_input_event(&mut w, event);
    }
    (events.len(), w.into_vec())
}

fn encode_input_event(w: &mut Writer, event: &InputEvent) {
    match *event {
        InputEvent::Scancode { flags, key_code } => {
            let mut ev_flags = 0u8;
            if flags & KBDFLAGS_RELEASE != 0 {
                ev_flags |= FASTPATH_INPUT_KBDFLAGS_RELEASE;
            }
            if flags & KBDFLAGS_EXTENDED != 0 {
                ev_flags |= FASTPATH_INPUT_KBDFLAGS_EXTENDED;
            }
            w.write_u8((FASTPATH_INPUT_EVENT_SCANCODE << 5) | ev_flags);
            w.write_u8(key_code as u8);
        }
        InputEvent::Unicode {
            flags,
            unicode_code,
        } => {
            let ev_flags = if flags & KBDFLAGS_RELEASE != 0 {
                FASTPATH_INPUT_KBDFLAGS_RELEASE
            } else {
                0
            };
            w.write_u8((FASTPATH_INPUT_EVENT_UNICODE << 5) | ev_flags);
            w.write_u16_le(unicode_code);
        }
        InputEvent::Mouse { flags, x, y } => {
            w.write_u8(FASTPATH_INPUT_EVENT_MOUSE << 5);
            w.write_u16_le(flags);
            w.write_u16_le(x);
            w.write_u16_le(y);
        }
        InputEvent::ExtendedMouse { flags, x, y } => {
            w.write_u8(FASTPATH_INPUT_EVENT_MOUSEX << 5);
            w.write_u16_le(flags);
            w.write_u16_le(x);
            w.write_u16_le(y);
        }
        InputEvent::Sync { toggle_flags } => {
            // The toggle state rides in the low bits of the event header.
            let mut ev_flags = 0u8;
            if toggle_flags & TS_SYNC_SCROLL_LOCK != 0 {
                ev_flags |= 0x01;
            }
            if toggle_flags & TS_SYNC_NUM_LOCK != 0 {
                ev_flags |= 0x02;
            }
            if toggle_flags & TS_SYNC_CAPS_LOCK != 0 {
                ev_flags |= 0x04;
            }
            if toggle_flags & TS_SYNC_KANA_LOCK != 0 {
                ev_flags |= 0x08;
            }
            w.write_u8((FASTPATH_INPUT_EVENT_SYNC << 5) | ev_flags);
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::input::{KBDFLAGS_RELEASE, PTRFLAGS_MOVE};

    #[test]
    fn detects_fastpath_vs_tpkt() {
        assert!(is_fastpath(0x00)); // action 0
        assert!(is_fastpath(0x04)); // numberEvents set, action 0
        assert!(!is_fastpath(0x03)); // TPKT version 3
    }

    #[test]
    fn length_roundtrip() {
        for len in [1usize, 0x7F, 0x80, 0x1234, 0x7FFF] {
            let mut w = Writer::new();
            write_length(&mut w, len).unwrap();
            let bytes = w.into_vec();
            let mut r = Reader::new(&bytes);
            assert_eq!(read_length(&mut r).unwrap(), len, "len {len:#x}");
        }
    }

    #[test]
    fn parses_bitmap_update() {
        // One fast-path bitmap update: numberRectangles + a single 1x1 16bpp
        // TS_BITMAP_DATA at (5, 6) — no leading updateType.
        let mut inner = Writer::new();
        inner.write_u16_le(1); // numberRectangles
        for v in [5u16, 6, 5, 6, 1, 1, 16, 0, 2] {
            inner.write_u16_le(v); // dest bounds, size, bpp, flags, bitmapLength
        }
        inner.write_bytes(&[0x00, 0xF8]); // one 16bpp pixel
        let body_bytes = inner.into_vec();

        let mut w = Writer::new();
        w.write_u8(FASTPATH_UPDATETYPE_BITMAP); // updateHeader
        w.write_u16_le(body_bytes.len() as u16);
        w.write_bytes(&body_bytes);
        let updates = parse_output_updates(w.as_slice()).unwrap();
        assert_eq!(updates.len(), 1);
        match &updates[0] {
            FastPathUpdate::Bitmap(rects) => {
                assert_eq!(rects[0].dest_left, 5);
                assert_eq!(rects[0].dest_top, 6);
            }
            other => panic!("expected Bitmap, got {other:?}"),
        }
    }

    #[test]
    fn parses_pointer_position() {
        let mut inner = Writer::new();
        inner.write_u16_le(320);
        inner.write_u16_le(240);
        let inner = inner.into_vec();
        let mut w = Writer::new();
        w.write_u8(FASTPATH_UPDATETYPE_PTR_POSITION);
        w.write_u16_le(inner.len() as u16);
        w.write_bytes(&inner);
        let updates = parse_output_updates(w.as_slice()).unwrap();
        assert_eq!(
            updates[0],
            FastPathUpdate::PointerPosition { x: 320, y: 240 }
        );
    }

    #[test]
    fn parses_null_and_default_pointer() {
        let mut w = Writer::new();
        w.write_u8(FASTPATH_UPDATETYPE_PTR_NULL);
        w.write_u16_le(0);
        w.write_u8(FASTPATH_UPDATETYPE_PTR_DEFAULT);
        w.write_u16_le(0);
        let updates = parse_output_updates(w.as_slice()).unwrap();
        assert_eq!(
            updates,
            vec![
                FastPathUpdate::PointerHidden,
                FastPathUpdate::PointerDefault
            ]
        );
    }

    #[test]
    fn encodes_input_events_compactly() {
        let (count, bytes) = encode_input_events(&[
            InputEvent::key_press(0x1E),
            InputEvent::Scancode {
                flags: KBDFLAGS_RELEASE,
                key_code: 0x1E,
            },
            InputEvent::Mouse {
                flags: PTRFLAGS_MOVE,
                x: 100,
                y: 200,
            },
        ]);
        assert_eq!(count, 3);
        // key press: header (scancode<<5 | 0) + keycode = 2 bytes.
        assert_eq!(bytes[0], 0x00);
        assert_eq!(bytes[1], 0x1E);
        // key release: header has the RELEASE flag.
        assert_eq!(bytes[2], FASTPATH_INPUT_KBDFLAGS_RELEASE);
        assert_eq!(bytes[3], 0x1E);
        // mouse: header (mouse<<5), then flags/x/y = 1 + 6 bytes.
        assert_eq!(bytes[4], FASTPATH_INPUT_EVENT_MOUSE << 5);
        assert_eq!(&bytes[5..7], &PTRFLAGS_MOVE.to_le_bytes());
        assert_eq!(&bytes[7..9], &100u16.to_le_bytes());
        assert_eq!(&bytes[9..11], &200u16.to_le_bytes());
    }
}
