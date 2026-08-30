//! Core client connection type (mirrors h2::client::Connection).

use crate::connect;
use crate::error::{ErrorCode, Result};
use crate::frame::{Frame, GoAwayFrame};

use super::builder::Builder;
use super::send_request::{RequestBuilder, SendRequest};

/// Core client HTTP/2 connection state: the frame-level driver
/// ([`connect::Connection`]) plus the stream-ID bookkeeping and HPACK
/// encoding needed to turn a [`RequestBuilder`] into wire frames.
#[derive(Debug)]
pub struct Connection {
    inner: connect::Connection,
    requests: SendRequest,
}

impl Connection {
    /// Build a new client connection with the given builder.
    pub fn new(builder: Builder) -> Self {
        let settings = builder.to_settings();
        let inner = connect::Connection::new(settings, connect::PeerType::Client);
        Connection {
            inner,
            requests: SendRequest::new(1),
        }
    }

    /// Apply a received frame to the connection state machine.
    pub fn apply(&mut self, frame: Frame) -> Result<Vec<Frame>> {
        self.inner.apply(frame)
    }

    /// Return the stream ID the next outgoing request will use.
    pub fn next_stream_id(&self) -> u32 {
        self.requests.next_stream_id()
    }

    /// Build a GOAWAY frame with the provided error code, for the caller's
    /// transport to write to the wire.
    pub fn send_goaway(&mut self, error_code: ErrorCode, debug_data: &[u8]) -> Frame {
        Frame::GoAway(GoAwayFrame {
            last_stream_id: 0,
            error_code,
            debug_data: debug_data.to_vec(),
        })
    }

    /// Encode `request` onto a freshly allocated stream, apply its frames
    /// to this connection's state machine (opening the stream and
    /// accounting for flow control), and return the frames a transport
    /// should write to the wire.
    pub fn send_request(&mut self, request: RequestBuilder) -> Result<Vec<Frame>> {
        let stream_id = self.requests.next_stream_id();
        self.requests.send(request)?;
        let frames = self.requests.take_frames(stream_id).unwrap_or_default();
        for frame in &frames {
            self.inner.apply_frame(frame.clone())?;
        }
        Ok(frames)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn client_connection_allocates_odd_stream_ids() {
        let conn = Connection::new(Builder::new());
        assert_eq!(conn.next_stream_id(), 1);
    }

    #[test]
    fn sending_a_request_produces_a_headers_frame_and_advances_the_stream_id() {
        let mut conn = Connection::new(Builder::new());
        let request = RequestBuilder::new("GET", "https://example.com/").end_of_stream();
        let frames = conn.send_request(request).unwrap();
        assert!(matches!(frames.as_slice(), [Frame::Headers(_)]));
        assert_eq!(conn.next_stream_id(), 3);
    }

    #[test]
    fn sending_a_request_with_a_body_produces_headers_then_data() {
        let mut conn = Connection::new(Builder::new());
        let request = RequestBuilder::new("POST", "/x").body(b"hi".to_vec());
        let frames = conn.send_request(request).unwrap();
        assert!(matches!(
            frames.as_slice(),
            [Frame::Headers(_), Frame::Data(_)]
        ));
    }

    #[test]
    fn send_goaway_builds_a_goaway_frame() {
        let mut conn = Connection::new(Builder::new());
        let frame = conn.send_goaway(ErrorCode::NoError, b"bye");
        match frame {
            Frame::GoAway(g) => assert_eq!(g.debug_data, b"bye"),
            other => panic!("expected GOAWAY, got {other:?}"),
        }
    }
}
