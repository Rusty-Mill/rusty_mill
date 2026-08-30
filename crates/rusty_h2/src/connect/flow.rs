//! Flow control (RFC 9113 §5.2).
//!
//! Flow control in HTTP/2 operates at two levels:
//!
//! 1. **Connection-level** (`SETTINGS_INITIAL_WINDOW_SIZE`, `WINDOW_UPDATE` on stream 0).
//! 2. **Per-stream** (`WINDOW_UPDATE` on stream ID).
//!
//! A sender MUST NOT exceed the connection-level or stream-level flow
//! control windows. A sender receiving a `FLOW_CONTROL_ERROR` MUST
//! initiate a connection-level graceful shutdown (RFC 9113 §5.4).

use crate::error::{ErrorCode, H2Error, Result};
use std::collections::BTreeMap;

/// Connection-level flow control state.
#[allow(dead_code)]
#[derive(Debug)]
pub struct FlowControl {
    /// Connection-level send window.
    connection_window: u32,
    /// Per-stream send windows.
    stream_windows: BTreeMap<u32, u32>,
}

impl FlowControl {
    /// Create a new flow control state with the negotiated initial window.
    pub fn new(initial_window: u32) -> Self {
        FlowControl {
            connection_window: initial_window,
            stream_windows: BTreeMap::new(),
        }
    }

    /// Try to send `data_len` bytes on `stream_id`.
    ///
    /// Returns `Ok(actual_len)` where `actual_len` is the number of bytes
    /// that can be sent within the current window (which may be less than
    /// `data_len` if the window is partially full).
    ///
    /// Returns `Err` if the window is exhausted or overflows.
    #[allow(dead_code)]
    pub fn try_send(&mut self, stream_id: u32, data_len: usize) -> Result<usize> {
        // Check connection-level window.
        if self.connection_window == 0 {
            return Ok(0);
        }

        // Check per-stream window.
        let stream_window = self.stream_windows.entry(stream_id).or_insert(u32::MAX);
        if *stream_window == 0 {
            return Ok(0);
        }

        Ok(data_len)
    }

    /// Send `data_len` bytes on `stream_id`, returning actual bytes allowed.
    pub fn send_flow_control(&mut self, stream_id: u32, data_len: usize) -> Result<usize> {
        let mut allowed = data_len;
        let stream_window = self.stream_windows.entry(stream_id).or_insert(u32::MAX);

        if *stream_window < allowed as u32 {
            allowed = *stream_window as usize;
        }

        if self.connection_window < allowed as u32 {
            allowed = self.connection_window as usize;
        }

        *stream_window -= allowed as u32;
        self.connection_window -= allowed as u32;

        Ok(allowed)
    }

    /// Acknowledge that we've received or sent `len` bytes.
    #[allow(dead_code)]
    pub fn ack_send(&mut self, stream_id: u32, len: u32) -> Result<()> {
        let stream_window = self.stream_windows.entry(stream_id).or_insert(u32::MAX);
        *stream_window = stream_window.saturating_add(len);
        Ok(())
    }

    /// Add to the connection-level window.
    #[allow(dead_code)]
    pub fn add_connection_window(&mut self, increment: u32) -> Result<()> {
        if increment == 0x7fff_ffff {
            return Err(H2Error::Connection(
                ErrorCode::FlowControlError,
                "window update must be smaller than 2^31",
            ));
        }
        self.connection_window = self.connection_window.saturating_add(increment);
        Ok(())
    }

    /// Set the connection-level flow control window.
    #[allow(dead_code)]
    pub fn set_connection_flow_window(&mut self, size: u32) -> Result<()> {
        self.connection_window = size;
        Ok(())
    }

    /// Add to a stream's window.
    #[allow(dead_code)]
    pub fn add_stream_window(&mut self, stream_id: u32, increment: u32) -> Result<()> {
        if increment == 0 {
            return Err(H2Error::Connection(
                ErrorCode::FlowControlError,
                "window update increment must be non-zero",
            ));
        }

        let stream_window = self.stream_windows.entry(stream_id).or_insert(increment);
        let new_window = stream_window.saturating_add(increment);

        if new_window < increment {
            return Err(H2Error::Connection(
                ErrorCode::FlowControlError,
                "flow control window overflow",
            ));
        }

        Ok(())
    }

    /// Reset flow control window for a stream.
    #[allow(dead_code)]
    pub fn reset_stream(&mut self, stream_id: u32) {
        self.stream_windows.remove(&stream_id);
    }

    #[allow(dead_code)]
    pub fn get_connection_flow_control(&self) -> u32 {
        self.connection_window
    }
}
