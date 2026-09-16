//! HTTP/2 frame types and the wire codec for them (RFC 9113 §4 and §6).

pub mod continuation;
pub mod data;
pub mod goaway;
pub mod header;
pub mod headers;
pub mod padding;
pub mod ping;
pub mod priority;
pub mod push_promise;
pub mod rst_stream;
pub mod settings;
pub mod window_update;

pub use continuation::ContinuationFrame;
pub use data::DataFrame;
pub use goaway::GoAwayFrame;
pub use header::{
    FrameHeader, FrameType, DEFAULT_MAX_FRAME_SIZE, FRAME_HEADER_LEN, MAX_MAX_FRAME_SIZE,
};
pub use headers::{HeadersFrame, Priority};
pub use ping::PingFrame;
pub use priority::PriorityFrame;
pub use push_promise::PushPromiseFrame;
pub use rst_stream::RstStreamFrame;
pub use settings::{Setting, SettingId, SettingsFrame};
pub use window_update::WindowUpdateFrame;

use crate::error::Result;

/// Any HTTP/2 frame, decoded from its type-specific payload.
///
/// Unknown frame types (RFC 9113 §4.1: "implementations MUST ignore and
/// discard frames of unknown types") are preserved as `Unknown` rather than
/// rejected, so callers can choose to log or drop them.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Frame {
    Data(DataFrame),
    Headers(HeadersFrame),
    Priority(PriorityFrame),
    RstStream(RstStreamFrame),
    Settings(SettingsFrame),
    PushPromise(PushPromiseFrame),
    Ping(PingFrame),
    GoAway(GoAwayFrame),
    WindowUpdate(WindowUpdateFrame),
    Continuation(ContinuationFrame),
    Unknown {
        frame_type: u8,
        stream_id: u32,
        payload: Vec<u8>,
    },
}

impl Frame {
    /// Decode a single frame given its already-parsed header and exactly
    /// `header.length` bytes of payload.
    pub fn decode(header: &FrameHeader, payload: &[u8]) -> Result<Frame> {
        Ok(match header.frame_type {
            FrameType::Data => Frame::Data(DataFrame::decode(header, payload)?),
            FrameType::Headers => Frame::Headers(HeadersFrame::decode(header, payload)?),
            FrameType::Priority => Frame::Priority(PriorityFrame::decode(header, payload)?),
            FrameType::RstStream => Frame::RstStream(RstStreamFrame::decode(header, payload)?),
            FrameType::Settings => Frame::Settings(SettingsFrame::decode(header, payload)?),
            FrameType::PushPromise => {
                Frame::PushPromise(PushPromiseFrame::decode(header, payload)?)
            }
            FrameType::Ping => Frame::Ping(PingFrame::decode(header, payload)?),
            FrameType::GoAway => Frame::GoAway(GoAwayFrame::decode(header, payload)?),
            FrameType::WindowUpdate => {
                Frame::WindowUpdate(WindowUpdateFrame::decode(header, payload)?)
            }
            FrameType::Continuation => {
                Frame::Continuation(ContinuationFrame::decode(header, payload)?)
            }
            FrameType::Unknown(t) => Frame::Unknown {
                frame_type: t,
                stream_id: header.stream_id,
                payload: payload.to_vec(),
            },
        })
    }

    pub fn encode(&self, out: &mut Vec<u8>) {
        match self {
            Frame::Data(f) => f.encode(out),
            Frame::Headers(f) => f.encode(out),
            Frame::Priority(f) => f.encode(out),
            Frame::RstStream(f) => f.encode(out),
            Frame::Settings(f) => f.encode(out),
            Frame::PushPromise(f) => f.encode(out),
            Frame::Ping(f) => f.encode(out),
            Frame::GoAway(f) => f.encode(out),
            Frame::WindowUpdate(f) => f.encode(out),
            Frame::Continuation(f) => f.encode(out),
            Frame::Unknown {
                frame_type,
                stream_id,
                payload,
            } => {
                let h = FrameHeader::new(
                    payload.len() as u32,
                    FrameType::Unknown(*frame_type),
                    header::Flags::NONE,
                    *stream_id,
                );
                h.encode(out);
                out.extend_from_slice(payload);
            }
        }
    }

    pub fn stream_id(&self) -> u32 {
        match self {
            Frame::Data(f) => f.stream_id,
            Frame::Headers(f) => f.stream_id,
            Frame::Priority(f) => f.stream_id,
            Frame::RstStream(f) => f.stream_id,
            Frame::Settings(_) => 0,
            Frame::PushPromise(f) => f.stream_id,
            Frame::Ping(_) => 0,
            Frame::GoAway(_) => 0,
            Frame::WindowUpdate(f) => f.stream_id,
            Frame::Continuation(f) => f.stream_id,
            Frame::Unknown { stream_id, .. } => *stream_id,
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn generic_roundtrip_through_frame_enum() {
        let original = Frame::Ping(PingFrame {
            ack: false,
            opaque_data: [0, 1, 2, 3, 4, 5, 6, 7],
        });
        let mut buf = Vec::new();
        original.encode(&mut buf);

        let header = FrameHeader::decode(&buf).unwrap();
        let decoded = Frame::decode(&header, &buf[FRAME_HEADER_LEN..]).unwrap();
        assert_eq!(decoded, original);
    }

    #[test]
    fn unknown_frame_type_preserved() {
        let mut buf = Vec::new();
        let h = FrameHeader::new(3, FrameType::Unknown(0xff), header::Flags::NONE, 1);
        h.encode(&mut buf);
        buf.extend_from_slice(b"abc");

        let header = FrameHeader::decode(&buf).unwrap();
        let decoded = Frame::decode(&header, &buf[FRAME_HEADER_LEN..]).unwrap();
        assert_eq!(
            decoded,
            Frame::Unknown {
                frame_type: 0xff,
                stream_id: 1,
                payload: b"abc".to_vec(),
            }
        );

        let mut re_encoded = Vec::new();
        decoded.encode(&mut re_encoded);
        assert_eq!(re_encoded, buf);
    }
}
