use crate::error::{ErrorCode, H2Error, Result};
use crate::frame::header::{Flags, FrameHeader, FrameType};

/// Known SETTINGS parameters (RFC 9113 §6.5.2).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum SettingId {
    HeaderTableSize,
    EnablePush,
    MaxConcurrentStreams,
    InitialWindowSize,
    MaxFrameSize,
    MaxHeaderListSize,
    Unknown(u16),
}

impl SettingId {
    pub fn from_u16(v: u16) -> Self {
        match v {
            0x1 => SettingId::HeaderTableSize,
            0x2 => SettingId::EnablePush,
            0x3 => SettingId::MaxConcurrentStreams,
            0x4 => SettingId::InitialWindowSize,
            0x5 => SettingId::MaxFrameSize,
            0x6 => SettingId::MaxHeaderListSize,
            other => SettingId::Unknown(other),
        }
    }

    pub fn as_u16(self) -> u16 {
        match self {
            SettingId::HeaderTableSize => 0x1,
            SettingId::EnablePush => 0x2,
            SettingId::MaxConcurrentStreams => 0x3,
            SettingId::InitialWindowSize => 0x4,
            SettingId::MaxFrameSize => 0x5,
            SettingId::MaxHeaderListSize => 0x6,
            SettingId::Unknown(v) => v,
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Setting {
    pub id: SettingId,
    pub value: u32,
}

/// SETTINGS frame (RFC 9113 §6.5). Always associated with stream 0.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct SettingsFrame {
    pub ack: bool,
    pub settings: Vec<Setting>,
}

impl SettingsFrame {
    pub fn ack() -> Self {
        SettingsFrame {
            ack: true,
            settings: Vec::new(),
        }
    }

    pub fn new(settings: Vec<Setting>) -> Self {
        SettingsFrame {
            ack: false,
            settings,
        }
    }

    pub fn encode(&self, out: &mut Vec<u8>) {
        let flags = if self.ack { Flags::ACK } else { Flags::NONE };
        let length = self.settings.len() as u32 * 6;
        let header = FrameHeader::new(length, FrameType::Settings, flags, 0);
        header.encode(out);
        for s in &self.settings {
            out.extend_from_slice(&s.id.as_u16().to_be_bytes());
            out.extend_from_slice(&s.value.to_be_bytes());
        }
    }

    pub fn decode(header: &FrameHeader, payload: &[u8]) -> Result<Self> {
        if header.stream_id != 0 {
            return Err(H2Error::Connection(
                ErrorCode::ProtocolError,
                "SETTINGS frame must be on stream 0",
            ));
        }
        let ack = header.flags.contains(Flags::ACK);
        if ack {
            if header.length != 0 {
                return Err(H2Error::Connection(
                    ErrorCode::FrameSizeError,
                    "SETTINGS ACK must be empty",
                ));
            }
            return Ok(SettingsFrame {
                ack: true,
                settings: Vec::new(),
            });
        }
        if !header.length.is_multiple_of(6) {
            return Err(H2Error::Connection(
                ErrorCode::FrameSizeError,
                "SETTINGS frame length must be a multiple of 6",
            ));
        }
        if payload.len() != header.length as usize {
            return Err(H2Error::Incomplete);
        }
        let mut settings = Vec::with_capacity(payload.len() / 6);
        for chunk in payload.chunks_exact(6) {
            let id = SettingId::from_u16(u16::from_be_bytes([chunk[0], chunk[1]]));
            let value = u32::from_be_bytes([chunk[2], chunk[3], chunk[4], chunk[5]]);
            if let SettingId::EnablePush = id {
                if value > 1 {
                    return Err(H2Error::Connection(
                        ErrorCode::ProtocolError,
                        "SETTINGS_ENABLE_PUSH must be 0 or 1",
                    ));
                }
            }
            if let SettingId::InitialWindowSize = id {
                if value > 0x7fff_ffff {
                    return Err(H2Error::Connection(
                        ErrorCode::FlowControlError,
                        "SETTINGS_INITIAL_WINDOW_SIZE exceeds maximum flow-control window",
                    ));
                }
            }
            if let SettingId::MaxFrameSize = id {
                if !(super::header::DEFAULT_MAX_FRAME_SIZE..=super::header::MAX_MAX_FRAME_SIZE)
                    .contains(&value)
                {
                    return Err(H2Error::Connection(
                        ErrorCode::ProtocolError,
                        "SETTINGS_MAX_FRAME_SIZE out of allowed range",
                    ));
                }
            }
            settings.push(Setting { id, value });
        }
        Ok(SettingsFrame {
            ack: false,
            settings,
        })
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn roundtrip() {
        let f = SettingsFrame::new(vec![
            Setting {
                id: SettingId::HeaderTableSize,
                value: 4096,
            },
            Setting {
                id: SettingId::MaxConcurrentStreams,
                value: 100,
            },
        ]);
        let mut buf = Vec::new();
        f.encode(&mut buf);
        let header = FrameHeader::decode(&buf).unwrap();
        let decoded = SettingsFrame::decode(&header, &buf[9..]).unwrap();
        assert_eq!(decoded, f);
    }

    #[test]
    fn ack_roundtrip() {
        let f = SettingsFrame::ack();
        let mut buf = Vec::new();
        f.encode(&mut buf);
        let header = FrameHeader::decode(&buf).unwrap();
        let decoded = SettingsFrame::decode(&header, &buf[9..]).unwrap();
        assert_eq!(decoded, f);
    }

    #[test]
    fn invalid_enable_push_rejected() {
        let payload = [0u8, 2, 0, 0, 0, 2];
        let header = FrameHeader::new(6, FrameType::Settings, Flags::NONE, 0);
        assert!(SettingsFrame::decode(&header, &payload).is_err());
    }

    #[test]
    fn non_multiple_of_six_rejected() {
        let header = FrameHeader::new(5, FrameType::Settings, Flags::NONE, 0);
        assert!(SettingsFrame::decode(&header, &[0; 5]).is_err());
    }
}
