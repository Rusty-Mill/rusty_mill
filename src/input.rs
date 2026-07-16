//! Client input PDUs (MS-RDPBCGR 2.2.8.1.1.3).
//!
//! Once the session is active the client sends keyboard and mouse activity in
//! a **Client Input Event PDU** (`TS_INPUT_PDU`), a Share Data PDU
//! ([`crate::pdu`]) of sub-type `PDUTYPE2_INPUT` whose body is:
//!
//! ```text
//! numberEvents u16 | pad u16 | inputEvents[numberEvents]
//! ```
//!
//! Each `TS_INPUT_EVENT` is a 4-byte `eventTime` (0 is accepted), a 2-byte
//! `messageType`, and 6 bytes of event data, so every event is 12 bytes.
//! This module models the slow-path events; fast-path input is a separate,
//! more compact framing left for later.

use crate::cursor::{Reader, Writer};
use crate::error::{Error, Result};
use crate::pdu::{ShareDataHeader, PDUTYPE2_INPUT};

// TS_INPUT_EVENT messageType values (2.2.8.1.1.3.1.1).
/// Keyboard synchronize event.
pub const INPUT_EVENT_SYNC: u16 = 0x0000;
/// Keyboard scancode event.
pub const INPUT_EVENT_SCANCODE: u16 = 0x0004;
/// Unicode keyboard event.
pub const INPUT_EVENT_UNICODE: u16 = 0x0005;
/// Mouse event.
pub const INPUT_EVENT_MOUSE: u16 = 0x8001;
/// Extended mouse event (X buttons).
pub const INPUT_EVENT_MOUSEX: u16 = 0x8002;

// Keyboard event flags (2.2.8.1.1.3.1.1.1).
/// `KBDFLAGS_EXTENDED` — the key is an extended (E0) scancode.
pub const KBDFLAGS_EXTENDED: u16 = 0x0100;
/// `KBDFLAGS_DOWN` — the key was already down (auto-repeat).
pub const KBDFLAGS_DOWN: u16 = 0x4000;
/// `KBDFLAGS_RELEASE` — key release (break); absence means press (make).
pub const KBDFLAGS_RELEASE: u16 = 0x8000;

// Mouse pointer flags (2.2.8.1.1.3.1.1.3).
/// `PTRFLAGS_WHEEL` — vertical wheel rotation.
pub const PTRFLAGS_WHEEL: u16 = 0x0200;
/// `PTRFLAGS_WHEEL_NEGATIVE` — the wheel rotation is negative.
pub const PTRFLAGS_WHEEL_NEGATIVE: u16 = 0x0100;
/// `PTRFLAGS_MOVE` — the pointer moved.
pub const PTRFLAGS_MOVE: u16 = 0x0800;
/// `PTRFLAGS_DOWN` — a button transitioned to pressed.
pub const PTRFLAGS_DOWN: u16 = 0x8000;
/// `PTRFLAGS_BUTTON1` — left button.
pub const PTRFLAGS_BUTTON1: u16 = 0x1000;
/// `PTRFLAGS_BUTTON2` — right button.
pub const PTRFLAGS_BUTTON2: u16 = 0x2000;
/// `PTRFLAGS_BUTTON3` — middle button.
pub const PTRFLAGS_BUTTON3: u16 = 0x4000;

// Extended mouse flags (2.2.8.1.1.3.1.1.4).
/// `PTRXFLAGS_DOWN` — an X button transitioned to pressed.
pub const PTRXFLAGS_DOWN: u16 = 0x8000;
/// `PTRXFLAGS_BUTTON1` — XButton1.
pub const PTRXFLAGS_BUTTON1: u16 = 0x0001;
/// `PTRXFLAGS_BUTTON2` — XButton2.
pub const PTRXFLAGS_BUTTON2: u16 = 0x0002;

// Sync event toggle flags (2.2.8.1.1.3.1.1.5).
/// `TS_SYNC_SCROLL_LOCK`.
pub const TS_SYNC_SCROLL_LOCK: u32 = 0x0000_0001;
/// `TS_SYNC_NUM_LOCK`.
pub const TS_SYNC_NUM_LOCK: u32 = 0x0000_0002;
/// `TS_SYNC_CAPS_LOCK`.
pub const TS_SYNC_CAPS_LOCK: u32 = 0x0000_0004;
/// `TS_SYNC_KANA_LOCK`.
pub const TS_SYNC_KANA_LOCK: u32 = 0x0000_0008;

/// Encoded size of a single `TS_INPUT_EVENT` (time + type + 6-byte data).
const INPUT_EVENT_LEN: usize = 12;

/// A single slow-path input event.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum InputEvent {
    /// Keyboard scancode (make/break) event.
    Scancode {
        /// `keyboardFlags` (`KBDFLAGS_*`).
        flags: u16,
        /// The scancode.
        key_code: u16,
    },
    /// Unicode character event.
    Unicode {
        /// `keyboardFlags` (`KBDFLAGS_*`).
        flags: u16,
        /// The UTF-16 code unit.
        unicode_code: u16,
    },
    /// Mouse move / button / wheel event.
    Mouse {
        /// `pointerFlags` (`PTRFLAGS_*`).
        flags: u16,
        /// Cursor x position.
        x: u16,
        /// Cursor y position.
        y: u16,
    },
    /// Extended mouse (X button) event.
    ExtendedMouse {
        /// `pointerFlags` (`PTRXFLAGS_*`).
        flags: u16,
        /// Cursor x position.
        x: u16,
        /// Cursor y position.
        y: u16,
    },
    /// Keyboard toggle-key synchronize event.
    Sync {
        /// `toggleFlags` (`TS_SYNC_*`).
        toggle_flags: u32,
    },
}

impl InputEvent {
    /// A key press (make) for `scancode`.
    pub fn key_press(scancode: u16) -> Self {
        InputEvent::Scancode {
            flags: 0,
            key_code: scancode,
        }
    }

    /// A key release (break) for `scancode`.
    pub fn key_release(scancode: u16) -> Self {
        InputEvent::Scancode {
            flags: KBDFLAGS_RELEASE,
            key_code: scancode,
        }
    }

    /// A pointer move to `(x, y)`.
    pub fn mouse_move(x: u16, y: u16) -> Self {
        InputEvent::Mouse {
            flags: PTRFLAGS_MOVE,
            x,
            y,
        }
    }

    /// A left-button press at `(x, y)`.
    pub fn left_button_down(x: u16, y: u16) -> Self {
        InputEvent::Mouse {
            flags: PTRFLAGS_BUTTON1 | PTRFLAGS_DOWN,
            x,
            y,
        }
    }

    /// A left-button release at `(x, y)`.
    pub fn left_button_up(x: u16, y: u16) -> Self {
        InputEvent::Mouse {
            flags: PTRFLAGS_BUTTON1,
            x,
            y,
        }
    }

    fn message_type(&self) -> u16 {
        match self {
            InputEvent::Scancode { .. } => INPUT_EVENT_SCANCODE,
            InputEvent::Unicode { .. } => INPUT_EVENT_UNICODE,
            InputEvent::Mouse { .. } => INPUT_EVENT_MOUSE,
            InputEvent::ExtendedMouse { .. } => INPUT_EVENT_MOUSEX,
            InputEvent::Sync { .. } => INPUT_EVENT_SYNC,
        }
    }

    fn encode(&self, w: &mut Writer) {
        w.write_u32_le(0); // eventTime (unused)
        w.write_u16_le(self.message_type());
        match *self {
            InputEvent::Scancode { flags, key_code } => {
                w.write_u16_le(flags);
                w.write_u16_le(key_code);
                w.write_u16_le(0); // pad2Octets
            }
            InputEvent::Unicode {
                flags,
                unicode_code,
            } => {
                w.write_u16_le(flags);
                w.write_u16_le(unicode_code);
                w.write_u16_le(0); // pad2Octets
            }
            InputEvent::Mouse { flags, x, y } | InputEvent::ExtendedMouse { flags, x, y } => {
                w.write_u16_le(flags);
                w.write_u16_le(x);
                w.write_u16_le(y);
            }
            InputEvent::Sync { toggle_flags } => {
                w.write_u16_le(0); // pad2Octets
                w.write_u32_le(toggle_flags);
            }
        }
    }

    fn decode(r: &mut Reader<'_>) -> Result<InputEvent> {
        let _event_time = r.read_u32_le()?;
        let message_type = r.read_u16_le()?;
        Ok(match message_type {
            INPUT_EVENT_SCANCODE => {
                let flags = r.read_u16_le()?;
                let key_code = r.read_u16_le()?;
                let _pad = r.read_u16_le()?;
                InputEvent::Scancode { flags, key_code }
            }
            INPUT_EVENT_UNICODE => {
                let flags = r.read_u16_le()?;
                let unicode_code = r.read_u16_le()?;
                let _pad = r.read_u16_le()?;
                InputEvent::Unicode {
                    flags,
                    unicode_code,
                }
            }
            INPUT_EVENT_MOUSE => {
                let flags = r.read_u16_le()?;
                let x = r.read_u16_le()?;
                let y = r.read_u16_le()?;
                InputEvent::Mouse { flags, x, y }
            }
            INPUT_EVENT_MOUSEX => {
                let flags = r.read_u16_le()?;
                let x = r.read_u16_le()?;
                let y = r.read_u16_le()?;
                InputEvent::ExtendedMouse { flags, x, y }
            }
            INPUT_EVENT_SYNC => {
                let _pad = r.read_u16_le()?;
                let toggle_flags = r.read_u32_le()?;
                InputEvent::Sync { toggle_flags }
            }
            other => {
                return Err(Error::InvalidValue {
                    field: "input messageType",
                    value: format!("0x{other:04X}"),
                });
            }
        })
    }
}

/// A `TS_INPUT_PDU` carrying one or more input events.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct InputPdu {
    /// The input events, in the order they occurred.
    pub events: Vec<InputEvent>,
}

impl InputPdu {
    /// Create an input PDU from a list of events.
    pub fn new(events: Vec<InputEvent>) -> Self {
        InputPdu { events }
    }

    /// Convenience: a PDU carrying a single event.
    pub fn single(event: InputEvent) -> Self {
        InputPdu {
            events: vec![event],
        }
    }

    fn encode_body(&self) -> Vec<u8> {
        let mut w = Writer::with_capacity(4 + self.events.len() * INPUT_EVENT_LEN);
        w.write_u16_le(self.events.len() as u16);
        w.write_u16_le(0); // pad2Octets
        for event in &self.events {
            event.encode(&mut w);
        }
        w.into_vec()
    }

    /// Encode as a Share Data PDU for `share_id`, sent from `pdu_source`.
    pub fn encode(&self, share_id: u32, pdu_source: u16) -> Result<Vec<u8>> {
        let body = self.encode_body();
        ShareDataHeader::new(share_id, PDUTYPE2_INPUT, body.len()).encode(pdu_source, &body)
    }

    /// Decode a Share Data input PDU, returning `(pdu_source, share_id, pdu)`.
    pub fn decode(buf: &[u8]) -> Result<(u16, u32, InputPdu)> {
        let (source, header, body) = ShareDataHeader::decode(buf)?;
        if header.pdu_type2 != PDUTYPE2_INPUT {
            return Err(Error::InvalidValue {
                field: "pduType2",
                value: header.pdu_type2.to_string(),
            });
        }
        let mut r = Reader::new(body);
        let number_events = r.read_u16_le()? as usize;
        let _pad = r.read_u16_le()?;
        let mut events = Vec::with_capacity(number_events);
        for _ in 0..number_events {
            events.push(InputEvent::decode(&mut r)?);
        }
        Ok((source, header.share_id, InputPdu { events }))
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn roundtrip(pdu: &InputPdu) {
        let bytes = pdu.encode(0x0001_00EA, 1007).unwrap();
        let (source, share_id, decoded) = InputPdu::decode(&bytes).unwrap();
        assert_eq!(source, 1007);
        assert_eq!(share_id, 0x0001_00EA);
        assert_eq!(&decoded, pdu);
    }

    #[test]
    fn keyboard_events_roundtrip() {
        roundtrip(&InputPdu::new(vec![
            InputEvent::key_press(0x1E), // 'a' make
            InputEvent::key_release(0x1E),
            InputEvent::Scancode {
                flags: KBDFLAGS_EXTENDED | KBDFLAGS_RELEASE,
                key_code: 0x48,
            },
        ]));
    }

    #[test]
    fn mouse_events_roundtrip() {
        roundtrip(&InputPdu::new(vec![
            InputEvent::mouse_move(640, 480),
            InputEvent::left_button_down(640, 480),
            InputEvent::left_button_up(640, 480),
            InputEvent::ExtendedMouse {
                flags: PTRXFLAGS_DOWN | PTRXFLAGS_BUTTON1,
                x: 10,
                y: 20,
            },
        ]));
    }

    #[test]
    fn unicode_and_sync_roundtrip() {
        roundtrip(&InputPdu::new(vec![
            InputEvent::Unicode {
                flags: 0,
                unicode_code: 0x20AC, // euro sign
            },
            InputEvent::Sync {
                toggle_flags: TS_SYNC_NUM_LOCK | TS_SYNC_CAPS_LOCK,
            },
        ]));
    }

    #[test]
    fn body_layout_single_event() {
        let pdu = InputPdu::single(InputEvent::key_press(0x1E));
        let bytes = pdu.encode(0x1234, 1007).unwrap();
        let (_, _, decoded) = InputPdu::decode(&bytes).unwrap();
        // numberEvents = 1.
        let (_, header, body) = ShareDataHeader::decode(&bytes).unwrap();
        assert_eq!(header.pdu_type2, PDUTYPE2_INPUT);
        assert_eq!(u16::from_le_bytes([body[0], body[1]]), 1);
        // Each event is 12 bytes: 4 (header body) + 12.
        assert_eq!(body.len(), 4 + INPUT_EVENT_LEN);
        assert_eq!(decoded.events[0], InputEvent::key_press(0x1E));
    }

    #[test]
    fn rejects_unknown_event_type() {
        // Build an input PDU body with a bogus messageType.
        let mut body = Writer::new();
        body.write_u16_le(1); // numberEvents
        body.write_u16_le(0); // pad
        body.write_u32_le(0); // eventTime
        body.write_u16_le(0x1234); // bad messageType
        body.write_bytes(&[0; 6]);
        let bytes = ShareDataHeader::new(1, PDUTYPE2_INPUT, body.len())
            .encode(1007, body.as_slice())
            .unwrap();
        assert!(matches!(
            InputPdu::decode(&bytes).unwrap_err(),
            Error::InvalidValue {
                field: "input messageType",
                ..
            }
        ));
    }

    #[test]
    fn rejects_wrong_pdu_type2() {
        let bytes = ShareDataHeader::new(1, crate::pdu::PDUTYPE2_UPDATE, 4)
            .encode(1007, &[0, 0, 0, 0])
            .unwrap();
        assert!(matches!(
            InputPdu::decode(&bytes).unwrap_err(),
            Error::InvalidValue {
                field: "pduType2",
                ..
            }
        ));
    }
}
