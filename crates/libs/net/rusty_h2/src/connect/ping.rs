//! Ping/Pong keepalive state (RFC 9113 §6.2).
//!
//! HTTP/2 connections periodically send PING frames to:
//! - Verify liveness (both sides must respond with ACK to unacknowledged ping)
//! - Keepalive during idle periods
//!
//! The receiver MUST respond with a PING frame containing the same
//! opaque data, but with the ACK flag set (RFC 9113 §6.2).

#[derive(Debug, Clone)]
pub struct PingState {
    /// The current in-flight ping value (opaque data).
    current_ping: Option<[u8; 8]>,
    /// How long to wait for the ping ACK (in milliseconds).
    ack_timeout_ms: u64,
}

impl PingState {
    pub fn new(ack_timeout_ms: u64) -> Self {
        PingState {
            current_ping: None,
            ack_timeout_ms,
        }
    }

    /// Start a new ping (no data — random bytes are used in a real impl).
    pub fn start_ping(&mut self, ping_data: [u8; 8]) {
        self.current_ping = Some(ping_data);
    }

    /// Clear the ping state when the ACK is received.
    pub fn ack_received(&mut self) {
        self.current_ping = None;
    }

    /// Whether there is a pending ping awaiting ACK.
    pub fn is_pending(&self) -> bool {
        self.current_ping.is_some()
    }

    /// Return the current ping data if one is pending.
    pub fn current_ping(&self) -> Option<[u8; 8]> {
        self.current_ping
    }

    /// Whether the ping has timed out.
    #[allow(dead_code)]
    pub fn is_timed_out(&self, elapsed_ms: u64) -> bool {
        self.is_pending() && elapsed_ms > self.ack_timeout_ms
    }
}
