use crate::error::{ErrorCode, H2Error, Result};

/// The 9-octet frame header shared by every HTTP/2 frame (RFC 9113 §4.1).
pub const FRAME_HEADER_LEN: usize = 9;

/// Default and hard cap for a frame's payload length, per RFC 9113 §4.2.
pub const DEFAULT_MAX_FRAME_SIZE: u32 = 1 << 14; // 16,384
pub const MAX_MAX_FRAME_SIZE: u32 = (1 << 24) - 1; // 16,777,215

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum FrameType {
    Data,
    Headers,
    Priority,
    RstStream,
    Settings,
    PushPromise,
    Ping,
    GoAway,
    WindowUpdate,
    Continuation,
    Unknown(u8),
}

impl FrameType {
    pub fn from_u8(v: u8) -> Self {
        match v {
            0x0 => FrameType::Data,
            0x1 => FrameType::Headers,
            0x2 => FrameType::Priority,
            0x3 => FrameType::RstStream,
            0x4 => FrameType::Settings,
            0x5 => FrameType::PushPromise,
            0x6 => FrameType::Ping,
            0x7 => FrameType::GoAway,
            0x8 => FrameType::WindowUpdate,
            0x9 => FrameType::Continuation,
            other => FrameType::Unknown(other),
        }
    }

    pub fn as_u8(self) -> u8 {
        match self {
            FrameType::Data => 0x0,
            FrameType::Headers => 0x1,
            FrameType::Priority => 0x2,
            FrameType::RstStream => 0x3,
            FrameType::Settings => 0x4,
            FrameType::PushPromise => 0x5,
            FrameType::Ping => 0x6,
            FrameType::GoAway => 0x7,
            FrameType::WindowUpdate => 0x8,
            FrameType::Continuation => 0x9,
            FrameType::Unknown(v) => v,
        }
    }
}

/// Generic flags byte. Meaning is frame-type-dependent; see each frame
/// module for the named accessors (e.g. `HeadersFrame::end_headers`).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub struct Flags(u8);

impl Flags {
    pub const NONE: Flags = Flags(0x0);
    pub const END_STREAM: Flags = Flags(0x1);
    pub const ACK: Flags = Flags(0x1);
    pub const END_HEADERS: Flags = Flags(0x4);
    pub const PADDED: Flags = Flags(0x8);
    pub const PRIORITY: Flags = Flags(0x20);

    pub fn bits(self) -> u8 {
        self.0
    }

    pub fn from_bits_truncate(bits: u8) -> Self {
        Flags(bits)
    }

    pub fn contains(self, other: Flags) -> bool {
        self.0 & other.0 == other.0
    }
}

impl std::ops::BitOr for Flags {
    type Output = Flags;
    fn bitor(self, rhs: Flags) -> Flags {
        Flags(self.0 | rhs.0)
    }
}

impl std::ops::BitOrAssign for Flags {
    fn bitor_assign(&mut self, rhs: Flags) {
        self.0 |= rhs.0;
    }
}

/// The common 9-byte header preceding every frame's payload.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct FrameHeader {
    pub length: u32, // 24-bit
    pub frame_type: FrameType,
    pub flags: Flags,
    pub stream_id: u32, // 31-bit, top bit (R) is reserved and ignored
}

impl FrameHeader {
    pub fn new(length: u32, frame_type: FrameType, flags: Flags, stream_id: u32) -> Self {
        debug_assert!(length <= MAX_MAX_FRAME_SIZE);
        debug_assert!(stream_id & 0x8000_0000 == 0);
        FrameHeader {
            length,
            frame_type,
            flags,
            stream_id,
        }
    }

    pub fn encode(&self, out: &mut Vec<u8>) {
        out.push((self.length >> 16) as u8);
        out.push((self.length >> 8) as u8);
        out.push(self.length as u8);
        out.push(self.frame_type.as_u8());
        out.push(self.flags.bits());
        out.extend_from_slice(&(self.stream_id & 0x7fff_ffff).to_be_bytes());
    }

    pub fn decode(buf: &[u8]) -> Result<Self> {
        if buf.len() < FRAME_HEADER_LEN {
            return Err(H2Error::Incomplete);
        }
        let length = (buf[0] as u32) << 16 | (buf[1] as u32) << 8 | buf[2] as u32;
        let frame_type = FrameType::from_u8(buf[3]);
        let flags = Flags::from_bits_truncate(buf[4]);
        let stream_id = u32::from_be_bytes([buf[5], buf[6], buf[7], buf[8]]) & 0x7fff_ffff;
        Ok(FrameHeader {
            length,
            frame_type,
            flags,
            stream_id,
        })
    }
}

/// Verify a decoded length against the negotiated SETTINGS_MAX_FRAME_SIZE.
pub fn check_frame_size(length: u32, max_frame_size: u32) -> Result<()> {
    if length > max_frame_size {
        return Err(H2Error::Connection(
            ErrorCode::FrameSizeError,
            "frame length exceeds SETTINGS_MAX_FRAME_SIZE",
        ));
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn roundtrip() {
        let h = FrameHeader::new(
            42,
            FrameType::Headers,
            Flags::END_HEADERS | Flags::END_STREAM,
            7,
        );
        let mut buf = Vec::new();
        h.encode(&mut buf);
        assert_eq!(buf.len(), FRAME_HEADER_LEN);
        let decoded = FrameHeader::decode(&buf).unwrap();
        assert_eq!(h, decoded);
    }

    #[test]
    fn reserved_bit_ignored_on_decode() {
        let mut buf = vec![0, 0, 0, 0x0, 0, 0x80, 0, 0, 1];
        let decoded = FrameHeader::decode(&buf).unwrap();
        assert_eq!(decoded.stream_id, 0x0000_0001);
        buf[5] = 0xff;
        let decoded = FrameHeader::decode(&buf).unwrap();
        assert_eq!(decoded.stream_id, 0x7f00_0001);
    }

    #[test]
    fn incomplete_header() {
        let buf = [0u8; 5];
        assert_eq!(FrameHeader::decode(&buf), Err(H2Error::Incomplete));
    }
}
