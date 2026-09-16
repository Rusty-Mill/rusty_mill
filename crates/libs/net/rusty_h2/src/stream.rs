//! HTTP/2 stream states and transitions (RFC 9113 §5.1).

use crate::error::{ErrorCode, H2Error, Result};

/// A stream's state, per the state machine diagram in RFC 9113 §5.1.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum StreamState {
    Idle,
    ReservedLocal,
    ReservedRemote,
    Open,
    HalfClosedLocal,
    HalfClosedRemote,
    Closed,
}

/// An event that drives a stream's state transition.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Event {
    /// We sent (Send) or received (Recv) a HEADERS frame that opens the stream.
    SendHeaders,
    RecvHeaders,
    /// A HEADERS or DATA frame carrying END_STREAM.
    SendEndStream,
    RecvEndStream,
    /// We sent/received a PUSH_PROMISE, reserving the promised stream.
    SendPushPromise,
    RecvPushPromise,
    SendRstStream,
    RecvRstStream,
}

/// A stream's state plus the transition logic. Frame types not relevant to
/// state (DATA/WINDOW_UPDATE without END_STREAM, PRIORITY, etc.) don't
/// generate an `Event` and never change state, matching RFC 9113 §5.1.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Stream {
    pub id: u32,
    pub state: StreamState,
}

impl Stream {
    pub fn new(id: u32) -> Self {
        Stream {
            id,
            state: StreamState::Idle,
        }
    }

    /// Apply `event`, returning a stream error (RFC 9113 §5.4.2) if the
    /// event is invalid for the current state.
    pub fn apply(&mut self, event: Event) -> Result<()> {
        use Event::*;
        use StreamState::*;

        let next = match (self.state, event) {
            (Idle, SendHeaders) | (Idle, RecvHeaders) => Open,
            (Idle, SendPushPromise) => ReservedLocal,
            (Idle, RecvPushPromise) => ReservedRemote,

            (ReservedLocal, SendHeaders) => HalfClosedRemote,
            (ReservedRemote, RecvHeaders) => HalfClosedLocal,
            (ReservedLocal, SendRstStream) | (ReservedLocal, RecvRstStream) => Closed,
            (ReservedRemote, SendRstStream) | (ReservedRemote, RecvRstStream) => Closed,

            (Open, SendEndStream) => HalfClosedLocal,
            (Open, RecvEndStream) => HalfClosedRemote,
            (Open, SendRstStream) | (Open, RecvRstStream) => Closed,

            (HalfClosedLocal, RecvEndStream) => Closed,
            (HalfClosedLocal, SendRstStream) | (HalfClosedLocal, RecvRstStream) => Closed,

            (HalfClosedRemote, SendEndStream) => Closed,
            (HalfClosedRemote, SendRstStream) | (HalfClosedRemote, RecvRstStream) => Closed,

            // Frames that don't change state but are still valid to see:
            // e.g. a HEADERS carrying END_STREAM triggers both SendHeaders
            // (or RecvHeaders) and SendEndStream/RecvEndStream in sequence;
            // callers apply both events. Anything else is a protocol error.
            _ => {
                return Err(H2Error::Stream(
                    self.id,
                    ErrorCode::StreamClosed,
                    "event is not valid for the stream's current state",
                ))
            }
        };
        self.state = next;
        Ok(())
    }

    pub fn is_closed(&self) -> bool {
        self.state == StreamState::Closed
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use Event::*;
    use StreamState::*;

    #[test]
    fn idle_to_open_via_headers() {
        let mut s = Stream::new(1);
        s.apply(RecvHeaders).unwrap();
        assert_eq!(s.state, Open);
    }

    #[test]
    fn full_request_response_cycle() {
        let mut s = Stream::new(1);
        s.apply(SendHeaders).unwrap(); // client sends request headers
        assert_eq!(s.state, Open);
        s.apply(SendEndStream).unwrap(); // client's DATA carries END_STREAM
        assert_eq!(s.state, HalfClosedLocal);
        s.apply(RecvEndStream).unwrap(); // server's response completes
        assert_eq!(s.state, Closed);
        assert!(s.is_closed());
    }

    #[test]
    fn push_promise_reserves_stream() {
        let mut s = Stream::new(2);
        s.apply(RecvPushPromise).unwrap();
        assert_eq!(s.state, ReservedRemote);
        s.apply(RecvHeaders).unwrap();
        assert_eq!(s.state, HalfClosedLocal);
    }

    #[test]
    fn rst_stream_closes_from_any_active_state() {
        for event in [SendHeaders, RecvHeaders] {
            let mut s = Stream::new(1);
            s.apply(event).unwrap();
            s.apply(RecvRstStream).unwrap();
            assert_eq!(s.state, Closed);
        }
    }

    #[test]
    fn invalid_transition_is_stream_error() {
        let mut s = Stream::new(1);
        // Can't half-close a stream that was never opened.
        assert!(s.apply(SendEndStream).is_err());
    }

    #[test]
    fn closed_stream_rejects_further_events() {
        let mut s = Stream::new(1);
        s.apply(SendHeaders).unwrap();
        s.apply(SendRstStream).unwrap();
        assert!(s.apply(RecvHeaders).is_err());
    }
}
